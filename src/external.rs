//! Generators outside this binary: `generator = "<path>"` on a
//! `[[generate.target]]`.
//!
//! An external generator speaks the generator protocol
//! (`comline_codegen::protocol`): one CBOR [`Request`] on stdin, one CBOR
//! [`Response`] on stdout. The CLI asks it to describe itself first, refuses
//! a protocol or IR format it doesn't share, and then sends the request to
//! generate. The built-ins answer the same requests in process
//! ([`builtin`]), so the protocol is exercised on every run.
//!
//! A generator is untrusted code. A `.wasm` runs under wasmtime with no
//! preopened directories, no environment, no network and no arguments beyond
//! `argv[0]`, with its memory limited ([`WASM_MEMORY`]). A native executable
//! runs as its own process with a cleared environment - it can do anything
//! the user can, so only a `.wasm` is sandboxed. Both have a wall-time limit
//! ([`DEFAULT_TIMEOUT`], `timeout` on the target) and a stdout cap
//! ([`STDOUT_CAP`]); stderr is shown, prefixed with the generator's name, up
//! to [`STDERR_CAP`]. Every path in a response is checked ([`check_path`])
//! before anything is written.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use comline_codegen::protocol::{
    self, Description, FileOut, GenerateRequest, LanguageInfo, Request, Response, IR_FORMAT,
    PROTOCOL_VERSION,
};
use comline_codegen::versions::LangVersion;
use comline_codegen::Registry;
use miette::{miette, Result};

/// How long one call to a generator may take, unless the target sets
/// `timeout` (in seconds).
pub const DEFAULT_TIMEOUT: Duration = comline_codegen::config::DEFAULT_TIMEOUT;
/// The most a generator may write to stdout: one encoded response.
pub const STDOUT_CAP: usize = 256 * 1024 * 1024;
/// The most generated file contents one response may carry, all files
/// together.
pub const GENERATED_CAP: usize = 256 * 1024 * 1024;
/// The most of a generator's stderr that's shown; the rest is dropped.
pub const STDERR_CAP: usize = 64 * 1024;
/// The most linear memory a `.wasm` generator may grow to.
#[cfg(feature = "gen-wasm")]
pub const WASM_MEMORY: usize = 1024 * 1024 * 1024;

/// A generator at a path, not yet run.
pub struct External {
    /// What messages call it: the file name.
    pub name: String,
    path: PathBuf,
    timeout: Duration,
    runner: Runner,
}

enum Runner {
    Native,
    #[cfg(feature = "gen-wasm")]
    Wasm(wasm::Compiled),
}

impl External {
    /// The generator at `path` (already absolute). A `.wasm` is compiled
    /// here, once, for every call after.
    pub fn open(path: &Path, timeout: Duration) -> Result<Self> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        if !path.is_file() {
            return Err(miette!(
                "generator `{}`: no such file (relative paths are relative to comline.toml's \
                 directory)",
                path.display()
            ));
        }
        let is_wasm = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("wasm"));
        let runner = if is_wasm {
            #[cfg(feature = "gen-wasm")]
            {
                Runner::Wasm(
                    wasm::Compiled::new(path).map_err(|e| miette!("generator `{name}`: {e}"))?,
                )
            }
            #[cfg(not(feature = "gen-wasm"))]
            {
                return Err(miette!(
                    "generator `{name}` is a `.wasm`, and this build of comline was built without \
                     the `gen-wasm` feature that runs one"
                ));
            }
        } else {
            Runner::Native
        };
        Ok(External {
            name,
            path: path.to_owned(),
            timeout,
            runner,
        })
    }

    /// One request, one response.
    fn call(&self, request: &Request) -> Result<Response> {
        let input = protocol::encode(request).map_err(|e| miette!("{e}"))?;
        let output = match &self.runner {
            Runner::Native => native::run(&self.path, &[], &self.name, &input, self.timeout),
            #[cfg(feature = "gen-wasm")]
            Runner::Wasm(compiled) => compiled.run(&self.name, &input, self.timeout),
        }
        .map_err(|why| miette!("generator `{}`: {why}", self.name))?;
        protocol::decode(&output).map_err(|e| {
            miette!(
                "generator `{}` answered with something that isn't a protocol response: {e}",
                self.name
            )
        })
    }

    /// What the generator handles, checked against this CLI's protocol
    /// version and IR format.
    pub fn describe(&self) -> Result<Description> {
        let description = match self.call(&Request::describe())? {
            Response::Describe(d) => d,
            Response::Error { message } => {
                return Err(miette!(
                    "generator `{}` refused to describe itself: {message}",
                    self.name
                ))
            }
            Response::Generate(_) => {
                return Err(miette!(
                    "generator `{}` answered `Describe` with generated files",
                    self.name
                ))
            }
        };
        let (min, max) = description.protocols;
        if !(min..=max).contains(&PROTOCOL_VERSION) {
            return Err(miette!(
                "generator `{}` speaks generator protocol {}, and this comline speaks protocol \
                 {PROTOCOL_VERSION}",
                self.name,
                if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                },
            ));
        }
        if !description.ir_formats.contains(&IR_FORMAT) {
            return Err(miette!(
                "generator `{}` reads IR format {}, and this comline writes IR format {IR_FORMAT}",
                self.name,
                list(description.ir_formats.iter()),
            ));
        }
        Ok(description)
    }

    /// Generates one target. The response is checked whole before it's
    /// returned: nothing it carries has been written.
    pub fn generate(&self, request: GenerateRequest) -> Result<Vec<FileOut>> {
        let response = self.call(&Request::generate(request))?;
        files(&format!("generator `{}`", self.name), response)
    }
}

/// `items` as `a, b, c`, or `none`.
fn list<T: std::fmt::Display>(items: impl Iterator<Item = T>) -> String {
    let all: Vec<String> = items.map(|i| i.to_string()).collect();
    if all.is_empty() {
        "none".to_string()
    } else {
        all.join(", ")
    }
}

/// The one language a description offers, when the target names none.
pub fn only_language(name: &str, description: &Description) -> Result<String> {
    let mut ids: Vec<&str> = description
        .languages
        .iter()
        .map(|l| l.id.as_str())
        .collect();
    ids.sort();
    ids.dedup();
    match ids.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(miette!("generator `{name}` describes no languages")),
        many => Err(miette!(
            "generator `{name}` offers {}: set `language` on its `[[generate.target]]` to pick one",
            list(many.iter().map(|l| format!("`{l}`")))
        )),
    }
}

/// The entry of `description` for `language#version`: the newest floor at
/// or below the version, as the registry picks among built-ins.
pub fn select<'d>(
    name: &str,
    description: &'d Description,
    language: &str,
    version: &str,
) -> Result<&'d LanguageInfo> {
    let offered: Vec<&LanguageInfo> = description
        .languages
        .iter()
        .filter(|l| l.id.eq_ignore_ascii_case(language))
        .collect();
    if offered.is_empty() {
        return Err(miette!(
            "generator `{name}` doesn't generate `{language}`; it offers {}",
            list(description.languages.iter().map(|l| format!("`{}`", l.id)))
        ));
    }
    let declared = LangVersion::parse(version).ok_or_else(|| {
        miette!("`{language}#{version}`: `{version}` isn't a version - expected dotted numbers, like `1.70.0`")
    })?;
    offered
        .iter()
        .filter_map(|l| LangVersion::parse(&l.versions.floor).map(|floor| (floor, *l)))
        .filter(|(floor, _)| *floor <= declared)
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, l)| l)
        .ok_or_else(|| {
            miette!(
                "generator `{name}` handles `{language}` from {} on, not `{version}`",
                list(offered.iter().map(|l| format!("`{}`", l.versions.floor)))
            )
        })
}

/// A built-in's answer to `request`, through the generator protocol in
/// process: the request is encoded, answered by
/// [`protocol::respond`] as an external generator's `serve_stdio` would, and
/// checked like any response.
pub fn builtin(
    registry: &Registry,
    language: &str,
    request: GenerateRequest,
) -> Result<Vec<FileOut>> {
    let bytes = protocol::encode(&Request::generate(request)).map_err(|e| miette!("{e}"))?;
    files(
        &format!("`{language}` generator"),
        protocol::respond(registry, &bytes),
    )
}

/// The files in `response`, after every check: an error response, a wrong
/// answer, a bad path or too much output is refused whole.
fn files(who: &str, response: Response) -> Result<Vec<FileOut>> {
    let generated = match response {
        Response::Generate(g) => g,
        Response::Error { message } => return Err(miette!("{who}: {message}")),
        Response::Describe(_) => {
            return Err(miette!("{who} answered `Generate` with a description"))
        }
    };
    for diagnostic in &generated.diagnostics {
        crate::ui::warn(format!("{who}: {diagnostic}"));
    }
    let mut total = 0usize;
    for file in &generated.files {
        check_path(&file.path).map_err(|why| {
            miette!(
                "{who} returned the path `{}`, which {why}; nothing was written",
                file.path.escape_debug()
            )
        })?;
        total = total.saturating_add(file.contents.len());
    }
    if total > GENERATED_CAP {
        return Err(miette!(
            "{who} generated {total} bytes, over the {GENERATED_CAP}-byte limit; nothing was written"
        ));
    }
    Ok(generated.files)
}

/// Why `path` can't be a generated file's path, if it can't: it must be
/// relative, `/`-separated and normalized, so it stays under the target's
/// output directory on every platform.
pub fn check_path(path: &str) -> std::result::Result<(), &'static str> {
    if path.is_empty() {
        return Err("is empty");
    }
    if path.contains('\0') {
        return Err("contains a NUL");
    }
    if path.contains('\\') {
        return Err("contains a `\\` (a generated path is `/`-separated)");
    }
    if path.starts_with('/') {
        return Err("is absolute");
    }
    for component in path.split('/') {
        match component {
            "" => return Err("has an empty component"),
            "." | ".." => return Err("isn't normalized (it has a `.` or `..` component)"),
            c if c.contains(':') => return Err("has a `:` (a drive or stream prefix on Windows)"),
            _ => {}
        }
    }
    Ok(())
}

/// A shared, capped byte buffer: what a generator wrote to one stream.
#[derive(Clone)]
struct Capped {
    bytes: Arc<Mutex<Vec<u8>>>,
    cap: usize,
    /// Set once something past `cap` was offered.
    over: Arc<AtomicBool>,
}

impl Capped {
    fn new(cap: usize) -> Self {
        Capped {
            bytes: Arc::default(),
            cap,
            over: Arc::default(),
        }
    }

    /// Keeps what fits; `false` once anything didn't.
    fn push(&self, data: &[u8]) -> bool {
        let mut bytes = self.bytes.lock().unwrap_or_else(|e| e.into_inner());
        let room = self.cap.saturating_sub(bytes.len());
        bytes.extend_from_slice(&data[..data.len().min(room)]);
        if data.len() > room {
            self.over.store(true, Ordering::SeqCst);
            return false;
        }
        true
    }

    fn over(&self) -> bool {
        self.over.load(Ordering::SeqCst)
    }

    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut *self.bytes.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Shows a generator's stderr, each line prefixed with its name.
fn show_stderr(name: &str, stderr: &[u8], truncated: bool) {
    let text = String::from_utf8_lossy(stderr);
    let mut out = std::io::stderr().lock();
    for line in text.lines() {
        let _ = writeln!(out, "  [{name}] {line}");
    }
    if truncated {
        let _ = writeln!(
            out,
            "  [{name}] ... (stderr past {STDERR_CAP} bytes not shown)"
        );
    }
}

fn too_much_output() -> String {
    format!("its output passed the {STDOUT_CAP}-byte limit; nothing was written")
}

fn timed_out(timeout: Duration) -> String {
    format!(
        "didn't finish within {}s and was stopped (raise `timeout` on the target if it needs \
         longer); nothing was written",
        timeout.as_secs_f64()
    )
}

pub(crate) mod native {
    use super::*;

    /// Runs the executable at `path` with `args` and `input` on stdin, and
    /// returns its stdout once it exits successfully.
    ///
    /// The environment is cleared. `PATH` isn't kept: a generator gets its
    /// request on stdin and has no reason to find other programs. On
    /// Windows, `SYSTEMROOT` is kept, without which a process can't load the
    /// system libraries every program needs.
    pub fn run(
        path: &Path,
        args: &[String],
        name: &str,
        input: &[u8],
        timeout: Duration,
    ) -> std::result::Result<Vec<u8>, String> {
        let mut command = Command::new(path);
        command
            .args(args)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        if let Some(root) = std::env::var_os("SYSTEMROOT") {
            command.env("SYSTEMROOT", root);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("couldn't start it: {e}"))?;

        let mut stdin = child.stdin.take().expect("piped");
        let input = input.to_vec();
        // A generator that never reads stdin would otherwise block this
        // write, and with it the timeout.
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });

        let stdout = Capped::new(STDOUT_CAP);
        let reader = drain(child.stdout.take().expect("piped"), stdout.clone());
        let stderr = Capped::new(STDERR_CAP);
        let errors = drain(child.stderr.take().expect("piped"), stderr.clone());

        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if stdout.over() || Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(e) => return Err(format!("couldn't wait for it: {e}")),
            }
        };
        let _ = writer.join();
        let _ = reader.join();
        let _ = errors.join();
        show_stderr(name, &stderr.take(), stderr.over());

        if stdout.over() {
            return Err(too_much_output());
        }
        match status {
            None => Err(timed_out(timeout)),
            Some(status) if !status.success() => {
                Err(format!("exited with {status}; nothing was written"))
            }
            Some(_) => Ok(stdout.take()),
        }
    }

    /// Reads `from` into `into` until it ends, or until `into` is full.
    fn drain(mut from: impl Read + Send + 'static, into: Capped) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut chunk = [0u8; 64 * 1024];
            loop {
                match from.read(&mut chunk) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        // Past the cap, stderr keeps draining (and dropping)
                        // so the generator doesn't block on a full pipe;
                        // stdout stops, and the generator is killed.
                        if !into.push(&chunk[..n]) && into.cap == STDOUT_CAP {
                            return;
                        }
                    }
                }
            }
        })
    }
}

#[cfg(feature = "gen-wasm")]
mod wasm {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, Trap};
    use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
    use wasmtime_wasi::p1::{self, WasiP1Ctx};
    use wasmtime_wasi::p2::pipe::MemoryInputPipe;
    use wasmtime_wasi::{I32Exit, WasiCtxBuilder};

    /// A compiled module, run once per call.
    pub struct Compiled {
        engine: Engine,
        module: Module,
    }

    struct State {
        wasi: WasiP1Ctx,
        limits: StoreLimits,
    }

    impl Compiled {
        pub fn new(path: &Path) -> std::result::Result<Self, String> {
            let mut config = Config::new();
            // The wall-time limit: the epoch is bumped once the time is up,
            // which traps the guest at its next loop header or call.
            config.epoch_interruption(true);
            let engine =
                Engine::new(&config).map_err(|e| format!("couldn't set up wasmtime: {e:#}"))?;
            let module = Module::from_file(&engine, path)
                .map_err(|e| format!("isn't a WebAssembly module wasmtime can run: {e:#}"))?;
            Ok(Compiled { engine, module })
        }

        /// Runs the module's `_start` with `input` on stdin, and returns its
        /// stdout once it exits successfully.
        pub fn run(
            &self,
            name: &str,
            input: &[u8],
            timeout: Duration,
        ) -> std::result::Result<Vec<u8>, String> {
            let stdout = Capped::new(STDOUT_CAP);
            let stderr = Capped::new(STDERR_CAP);
            // Nothing beyond stdio: no preopened directories, no
            // environment, no arguments past `argv[0]`, no sockets.
            let wasi = WasiCtxBuilder::new()
                .stdin(MemoryInputPipe::new(input.to_vec()))
                .stdout(Stream {
                    into: stdout.clone(),
                    fail_past_cap: true,
                })
                .stderr(Stream {
                    into: stderr.clone(),
                    fail_past_cap: false,
                })
                .arg(name)
                .allow_tcp(false)
                .allow_udp(false)
                .allow_ip_name_lookup(false)
                .build_p1();
            let limits = StoreLimitsBuilder::new()
                .memory_size(WASM_MEMORY)
                .instances(1)
                .build();
            let mut store = Store::new(&self.engine, State { wasi, limits });
            store.limiter(|s| &mut s.limits);
            store.set_epoch_deadline(1);

            let mut linker: Linker<State> = Linker::new(&self.engine);
            p1::add_to_linker_sync(&mut linker, |s| &mut s.wasi).map_err(|e| format!("{e:#}"))?;
            let module = self.module.clone();

            // The guest runs on its own thread, so a host call that blocks
            // (a WASI sleep) can't hold the timeout up either.
            let (done, finished) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let result = linker
                    .instantiate(&mut store, &module)
                    .and_then(|instance| instance.get_typed_func::<(), ()>(&mut store, "_start"))
                    .and_then(|start| start.call(&mut store, ()));
                let _ = done.send(result);
            });
            let result = match finished.recv_timeout(timeout) {
                Ok(result) => result,
                Err(_) => {
                    self.engine.increment_epoch();
                    show_stderr(name, &stderr.take(), stderr.over());
                    return Err(timed_out(timeout));
                }
            };
            show_stderr(name, &stderr.take(), stderr.over());
            if stdout.over() {
                return Err(too_much_output());
            }
            match result {
                Ok(()) => Ok(stdout.take()),
                Err(e) => match e.downcast_ref::<I32Exit>() {
                    Some(I32Exit(0)) => Ok(stdout.take()),
                    Some(I32Exit(code)) => {
                        Err(format!("exited with code {code}; nothing was written"))
                    }
                    None if e.downcast_ref::<Trap>() == Some(&Trap::Interrupt) => {
                        Err(timed_out(timeout))
                    }
                    None => Err(format!("failed: {e:#}; nothing was written")),
                },
            }
        }
    }

    /// stdout or stderr, into a [`Capped`] buffer. Past the cap, stdout
    /// fails the write (the generator can't finish a response that won't be
    /// read), and stderr drops it.
    struct Stream {
        into: Capped,
        fail_past_cap: bool,
    }

    impl IsTerminal for Stream {
        fn is_terminal(&self) -> bool {
            false
        }
    }

    impl StdoutStream for Stream {
        fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
            Box::new(Stream {
                into: self.into.clone(),
                fail_past_cap: self.fail_past_cap,
            })
        }
    }

    impl tokio::io::AsyncWrite for Stream {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if !self.into.push(buf) && self.fail_past_cap {
                return Poll::Ready(Err(std::io::Error::other("output limit reached")));
            }
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::check_path;

    #[test]
    fn paths_must_be_relative_and_normalized() {
        for good in ["a.rs", "a/b.rs", "src/lib.rs", "Cargo.toml", "a-b_c/d.e.ts"] {
            assert_eq!(check_path(good), Ok(()), "{good}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "a/./b",
            "./a",
            "a//b",
            "a/",
            "C:/x",
            "C:x",
            "\\\\server\\share\\x",
            "a\\b",
            "a\0b",
            "..",
            ".",
        ] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
    }
}
