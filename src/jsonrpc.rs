//! Newline-delimited JSON-RPC 2.0 for a blocking stream.
//!
//! This retains quote-unquote's CDC framing rules: accept CRLF, limit lines
//! to 16 KiB, and isolate malformed input to the current line.

use serde_json::{json, Value};
use std::io::{self, Read, Write};

pub const MAX_LINE: usize = 16 * 1024;

pub fn serve(mut stream: impl Read + Write) -> io::Result<()> {
    let mut line = Vec::new();
    let mut dropping = false;
    loop {
        let mut byte = [0];
        if stream.read(&mut byte)? == 0 {
            continue;
        }
        match byte[0] {
            b'\n' if dropping => {
                dropping = false;
                write_response(
                    &mut stream,
                    json!({"jsonrpc":"2.0","error":{"code":-32700,"message":"line too long"}}),
                )?;
            }
            b'\n' => {
                let request = std::mem::take(&mut line);
                write_response(&mut stream, response(&request))?;
            }
            b'\r' => {}
            _ if dropping => {}
            _ if line.len() == MAX_LINE => {
                line.clear();
                dropping = true;
            }
            byte => line.push(byte),
        }
    }
}

fn response(line: &[u8]) -> Value {
    let request: Value = match serde_json::from_slice(line) {
        Ok(request) => request,
        Err(_) => return json!({"jsonrpc":"2.0","error":{"code":-32700,"message":"parse error"}}),
    };
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    match request.get("method").and_then(Value::as_str) {
        Some("system.ping") => json!({"jsonrpc":"2.0","id":id,"result":{"service":"quasiquote0"}}),
        _ => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}}),
    }
}

fn write_response(stream: &mut impl Write, value: Value) -> io::Result<()> {
    stream.write_all(value.to_string().as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}
