//! A planted bad implementation: `comline-reference`, except that every
//! request frame it encodes has its first byte flipped.

use std::io::{BufRead, Write};

use serde_json::Value;

fn main() {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let mut reply: Value =
            serde_json::from_str(&comline_reference::handle_line(&line)).unwrap();
        let request: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        if let (Value::Array(requests), Value::Array(replies)) = (&request, &mut reply) {
            for (request, reply) in requests.iter().zip(replies) {
                if request["op"] == "request" && request.get("bytes").is_none() {
                    if let Some(Value::String(hex)) = reply.get_mut("bytes") {
                        let first = u8::from_str_radix(&hex[..2], 16).unwrap() ^ 0xff;
                        hex.replace_range(..2, &format!("{first:02x}"));
                    }
                }
            }
        }
        writeln!(out, "{reply}").unwrap();
        out.flush().unwrap();
    }
}
