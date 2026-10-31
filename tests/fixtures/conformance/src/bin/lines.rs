//! A stub implementation of a third-party framing, `com.acme.lines@1`: a
//! request is `REQ <request id> <method> <params as JSON>`, a response
//! `OK <request id> <payload hex>`, `ERR <request id> <error id> <body hex>`
//! or `FAULT <request id>`, each as ASCII.

use std::io::{BufRead, Write};

use serde_json::{json, Map, Value};

const NAME: &str = "com.acme.lines@1";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn encode(op: &str, input: &Value) -> Option<String> {
    let id = input["request_id"].as_str()?;
    Some(match op {
        "request" => format!(
            "REQ {id} {} {}",
            input["method"].as_str()?,
            serde_json::to_string(&input["params"]).ok()?
        ),
        "response" => match input["kind"].as_str()? {
            "ok" => format!("OK {id} {}", input["payload"].as_str()?),
            "err" => format!("ERR {id} {} {}", input["id"], input["body"].as_str()?),
            "fault" => format!("FAULT {id}"),
            _ => return None,
        },
        _ => return None,
    })
}

/// The input `text` decodes to, the fields the wire doesn't carry taken
/// from `input`; `None` to refuse.
fn decode(op: &str, input: &Value, text: &str) -> Option<Value> {
    let mut out: Map<String, Value> = Map::new();
    out.insert("framing".into(), json!(NAME));
    let words: Vec<&str> = text.splitn(4, ' ').collect();
    match (op, words.as_slice()) {
        ("request", ["REQ", id, method, params]) => {
            for key in ["call_id", "format"] {
                out.insert(key.into(), input.get(key)?.clone());
            }
            out.insert("request_id".into(), json!(id));
            out.insert("method".into(), json!(method));
            out.insert("params".into(), serde_json::from_str(params).ok()?);
        }
        ("response", ["OK", id, payload]) => {
            unhex(payload)?;
            out.insert("request_id".into(), json!(id));
            out.insert("kind".into(), json!("ok"));
            out.insert("payload".into(), json!(payload));
        }
        ("response", ["ERR", id, error, body]) => {
            unhex(body)?;
            out.insert("request_id".into(), json!(id));
            out.insert("kind".into(), json!("err"));
            out.insert("id".into(), json!(error.parse::<u16>().ok()?));
            out.insert("body".into(), json!(body));
        }
        ("response", ["FAULT", id]) => {
            out.insert("request_id".into(), json!(id));
            out.insert("kind".into(), json!("fault"));
        }
        _ => return None,
    }
    Some(Value::Object(out))
}

fn answer(request: &Value) -> Value {
    let id = request["id"].clone();
    let op = request["op"].as_str().unwrap_or_default();
    let input = &request["input"];
    if op == "version" {
        return json!({ "id": id, "version": {
            "protocol": 1, "ops": ["request", "response", "version"],
            "formats": [], "framings": [NAME],
        }});
    }
    if !matches!(op, "request" | "response") || input["framing"] != NAME {
        return json!({ "id": id, "not_implemented": format!("{op} on {}", input["framing"]) });
    }
    match request.get("bytes").and_then(Value::as_str) {
        None => match encode(op, input) {
            Some(text) => json!({ "id": id, "bytes": hex(text.as_bytes()) }),
            None => json!({ "id": id, "error": "malformed input" }),
        },
        Some(bytes) => {
            let text = unhex(bytes).and_then(|b| String::from_utf8(b).ok());
            match text.and_then(|t| decode(op, input, &t)) {
                Some(input) => json!({ "id": id, "accept": true, "input": input }),
                None => json!({ "id": id, "accept": false }),
            }
        }
    }
}

fn main() {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let reply = match serde_json::from_str::<Value>(&line.unwrap()) {
            Ok(Value::Array(requests)) => Value::Array(requests.iter().map(answer).collect()),
            Ok(request) => answer(&request),
            Err(_) => json!({ "id": null, "error": "not JSON" }),
        };
        writeln!(out, "{reply}").unwrap();
        out.flush().unwrap();
    }
}
