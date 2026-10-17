//! The whole pipeline, for real: a `.comline` schema with a `protocol` →
//! `comline generate --mode lib` → the emitted crate is compiled against
//! `comline-runtime` and a hand-written driver runs a client ⇆ provider
//! round-trip over an in-memory transport (a request/response call, a raised
//! typed error, the *second* of two declared errors, a provider fault, a
//! one-way notify).
//!
//! Slow: it fetches and builds `comline-runtime`. Needs network.

use std::fs;
use std::process::Command;

use crate::util::*;

/// Driven against the generated `chat_e2e` crate (see
/// `tests/fixtures/chat_project/src/chat.ids`).
const DRIVER: &str = r#"
use std::sync::{Arc, Mutex};
use std::thread;

use chat_e2e::chat::{
    Chat, ChatClient, ChatDispatcher, ChatSendError, Message, Rejected, TooLong,
};
use comline_runtime::contract::{CallError, Fault, RuntimeError};
use comline_runtime::format::MsgPack;
use comline_runtime::transport::duplex;

struct Svc {
    notes: Arc<Mutex<Vec<String>>>,
}

impl Chat for Svc {
    fn send(&self, text: &str) -> Result<Message, Fault<ChatSendError>> {
        if text.is_empty() {
            // A declared error: `From<E> for Fault<E>` means `?`/`.into()`
            // works, but spell it out here so the test pins the shape.
            return Err(Fault::App(ChatSendError::Rejected(Rejected {
                reason: "empty".into(),
            })));
        }
        if text.len() > 8 {
            // the second error in `! Rejected, TooLong`
            return Err(Fault::App(ChatSendError::TooLong(TooLong { limit: 8 })));
        }
        if text == "boom" {
            // Nothing in the schema declares this failure. Before the fault
            // channel the provider's only options were to panic or to lie.
            return Err(Fault::Internal);
        }
        Ok(Message { body: format!("echo: {text}"), seq: 1 })
    }
    fn note(&self, text: &str) {
        self.notes.lock().unwrap().push(text.to_string());
    }
}

#[test]
fn client_and_provider_over_duplex() {
    let (client_side, provider_side) = duplex();
    let notes = Arc::new(Mutex::new(Vec::new()));
    let notes_for_svc = notes.clone();

    // Provider: the generated `serve` helper runs the connection handshake
    // (IR_HASH + format name) before serving.
    let provider = thread::spawn(move || {
        let mut provider_side = provider_side;
        ChatDispatcher(Svc { notes: notes_for_svc })
            .serve(&mut provider_side, MsgPack)
            .unwrap();
    });

    // Client: the generated `connect` helper does the matching handshake.
    let mut client = ChatClient::connect(client_side, MsgPack).expect("handshake");

    assert_eq!(client.send("hi").unwrap().body, "echo: hi");

    match client.send("").unwrap_err() {
        CallError::App(ChatSendError::Rejected(r)) => assert_eq!(r.reason, "empty"),
        other => panic!("expected a Rejected, got {other:?}"),
    }

    // The declared error renders through the generated `Display`, from the
    // schema's `message` template.
    assert_eq!(
        Rejected { reason: "empty".into() }.to_string(),
        "rejected: empty",
    );

    // Both arms of `! Rejected, TooLong` are reachable, and each decodes
    // into its own variant rather than into whichever shares an ordinal.
    match client.send("far too long to accept").unwrap_err() {
        CallError::App(ChatSendError::TooLong(t)) => {
            assert_eq!(t.limit, 8);
            assert_eq!(t.to_string(), "too long: 8 max");
        }
        other => panic!("expected a TooLong, got {other:?}"),
    }

    // An undeclared failure arrives as a fault, not as some ordinal the
    // client would have decoded into the wrong error type.
    match client.send("boom").unwrap_err() {
        CallError::Runtime(RuntimeError::Provider) => {}
        other => panic!("expected a provider fault, got {other:?}"),
    }

    // and the connection survives it — a fault is a reply, not a teardown
    assert_eq!(client.send("ok").unwrap().body, "echo: ok");

    client.note("saved").unwrap(); // one-way

    drop(client);
    provider.join().unwrap();

    assert_eq!(&*notes.lock().unwrap(), &["saved".to_string()]);
}
"#;

#[test]
fn a_generated_protocol_crate_runs_a_real_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let project = copy_fixture("chat_project", temp.path());

    comline_cmd()
        .current_dir(&project)
        .args([
            "generate", "--target", "rust", "--mode", "lib", "--out", "gen",
        ])
        .assert()
        .success();

    let crate_dir = project.join("gen/rust");
    assert!(crate_dir.join("Cargo.toml").exists());
    assert!(crate_dir.join("src/chat.rs").exists());

    // A lib crate git-deps `comline-runtime`; drop the driver in `tests/`.
    fs::create_dir_all(crate_dir.join("tests")).unwrap();
    fs::write(crate_dir.join("tests/roundtrip.rs"), DRIVER).unwrap();

    let out = Command::new(env!("CARGO"))
        .args(["test", "--quiet"])
        .current_dir(&crate_dir)
        .env("CARGO_TARGET_DIR", crate_dir.join("target"))
        .output()
        .expect("run cargo test on the generated crate");

    assert!(
        out.status.success(),
        "the generated crate's round-trip test failed\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
