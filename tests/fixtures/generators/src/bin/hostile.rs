//! A generator that misbehaves, one way per name: the tests copy it to
//! `<mode>.wasm` or `<mode>` and the mode is `argv[0]`'s file stem, since a
//! generator gets no other arguments and no environment.

use std::io::{Read, Write};

use comline_codegen::protocol::{
    decode, encode, Description, FileOut, Generated, LanguageInfo, Request, RequestBody, Response,
    VersionsInfo, IR_FORMAT, PROTOCOL_VERSION,
};

fn main() {
    let arg0 = std::env::args().next().unwrap_or_default();
    let mode = std::path::Path::new(&arg0)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input).unwrap();
    let request: Request = decode(&input).unwrap();
    let response = match request.body {
        RequestBody::Describe => describe(&mode),
        RequestBody::Generate(_) => generate(&mode),
    };
    let mut out = std::io::stdout().lock();
    out.write_all(&encode(&response).unwrap()).unwrap();
    out.flush().unwrap();
}

fn describe(mode: &str) -> Response {
    let protocol = if mode == "mismatch" {
        7
    } else {
        PROTOCOL_VERSION
    };
    Response::Describe(Description {
        languages: vec![LanguageInfo {
            id: "rust".into(),
            ext: "rs".into(),
            versions: VersionsInfo {
                floor: if mode == "future" { "9.0.0" } else { "1.0.0" }.into(),
                changes: vec![],
                verified_up_to: "1.0.0".into(),
                forward_compatible: true,
            },
            support: vec![],
        }],
        protocols: (protocol, protocol),
        ir_formats: vec![IR_FORMAT],
    })
}

fn files(paths: &[&str]) -> Response {
    Response::Generate(Generated {
        files: paths
            .iter()
            .map(|p| FileOut {
                path: p.to_string(),
                contents: b"x".to_vec(),
            })
            .collect(),
        diagnostics: vec![],
    })
}

fn generate(mode: &str) -> Response {
    match mode {
        // A good file first: nothing at all may be written.
        "dotdot" => files(&["api.rs", "../escape.rs"]),
        "absolute" => files(&["api.rs", "/tmp/comline-hostile-absolute.rs"]),
        "oversized" => {
            let chunk = vec![0u8; 1024 * 1024];
            let mut out = std::io::stdout().lock();
            loop {
                out.write_all(&chunk).unwrap();
            }
        }
        "timeout" =>
        {
            #[allow(clippy::empty_loop)]
            loop {}
        }
        "readfs" => match std::fs::read_to_string("comline.toml")
            .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        {
            Ok(text) => Response::Generate(Generated {
                files: vec![FileOut {
                    path: "leaked.rs".into(),
                    contents: text.into_bytes(),
                }],
                diagnostics: vec![],
            }),
            Err(e) => Response::Error {
                message: format!("couldn't read the filesystem: {e}"),
            },
        },
        "stderr" => {
            eprintln!("hello from the generator");
            let line = "x".repeat(1023);
            for _ in 0..100 {
                eprintln!("{line}");
            }
            Response::Error {
                message: "refusing after the noise".into(),
            }
        }
        other => Response::Error {
            message: format!("unknown mode `{other}`"),
        },
    }
}
