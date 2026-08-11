//! Newline-delimited JSON-RPC 2.0 for a blocking stream.
//!
//! Stream 0 is the only JSON control channel. Payload streams are separate
//! Yamux streams, so this layer never buffers frames or firmware images.

use serde::Serialize;
use serde_json::{json, Value};
use std::io::{self, Read, Write};

pub const MAX_LINE: usize = 16 * 1024;

#[derive(Debug)]
pub struct Request {
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

#[derive(Debug)]
pub struct Error {
    pub code: i64,
    pub message: String,
}

impl Error {
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
        }
    }

    pub fn method_not_found() -> Self {
        Self {
            code: -32601,
            message: "method not found".to_owned(),
        }
    }
}

pub trait Handler {
    fn handle(&mut self, request: Request) -> Result<Value, Error>;

    /// Called only after a response has been fully serialized and flushed.
    /// Implementations use this for actions such as an orderly reboot.
    fn after_response(&mut self) {}
}

/// Temporary minimal handler retained while an application is being brought up
/// without mounted persistent state. Production firmware uses `rpc::DeviceRpc`.
pub struct PingHandler;

impl Handler for PingHandler {
    fn handle(&mut self, request: Request) -> Result<Value, Error> {
        if request.method == "system.ping" {
            Ok(json!({"service":"quasiquote0"}))
        } else {
            Err(Error::method_not_found())
        }
    }
}

/// Serve requests until the control stream closes or transport fails.
pub fn serve(mut stream: impl Read + Write, handler: &mut impl Handler) -> io::Result<()> {
    let mut line = Vec::new();
    let mut dropping = false;
    loop {
        let mut byte = [0];
        let read = stream.read(&mut byte)?;
        if read == 0 {
            return Ok(());
        }
        match byte[0] {
            b'\n' if dropping => {
                dropping = false;
                write_value(
                    &mut stream,
                    error_response(Value::Null, -32700, "line too long"),
                )?;
            }
            b'\n' => {
                let request = std::mem::take(&mut line);
                handle_line(&mut stream, handler, &request)?;
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

fn handle_line(
    stream: &mut (impl Read + Write),
    handler: &mut impl Handler,
    line: &[u8],
) -> io::Result<()> {
    let value: Value = match serde_json::from_slice(line) {
        Ok(value) => value,
        Err(_) => return write_value(stream, error_response(Value::Null, -32700, "parse error")),
    };
    let object = match value.as_object() {
        Some(object) => object,
        None => {
            return write_value(
                stream,
                error_response(Value::Null, -32600, "invalid request"),
            )
        }
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return write_value(
            stream,
            error_response(Value::Null, -32600, "invalid request"),
        );
    }
    let id = object.get("id").cloned();
    let method = match object.get("method").and_then(Value::as_str) {
        Some(method) => method.to_owned(),
        None => {
            return write_value(
                stream,
                error_response(id.unwrap_or(Value::Null), -32600, "invalid request"),
            )
        }
    };
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
    let response = handler.handle(Request {
        id: id.clone(),
        method,
        params,
    });
    match response {
        Ok(_) if object.get("id").is_none() => Ok(()),
        Ok(result) => {
            write_value(stream, result_response(id.unwrap_or(Value::Null), result))?;
            handler.after_response();
            Ok(())
        }
        Err(_) if object.get("id").is_none() => Ok(()),
        Err(error) => {
            write_value(
                stream,
                error_response(id.unwrap_or(Value::Null), error.code, &error.message),
            )?;
            handler.after_response();
            Ok(())
        }
    }
}

pub fn result_response(id: Value, result: impl Serialize) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

pub fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

pub fn notification(method: &str, params: impl Serialize) -> Value {
    json!({"jsonrpc":"2.0","method":method,"params":params})
}

pub fn write_value(stream: &mut impl Write, value: Value) -> io::Result<()> {
    stream.write_all(value.to_string().as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct TestStream {
        read: Cursor<Vec<u8>>,
        write: Vec<u8>,
    }

    impl TestStream {
        fn new(input: &str) -> Self {
            Self {
                read: Cursor::new(input.as_bytes().to_vec()),
                write: Vec::new(),
            }
        }
    }

    impl Read for TestStream {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.read.read(bytes)
        }
    }

    impl Write for TestStream {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.write.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn parses_request_and_stops_at_stream_eof() {
        let mut stream =
            TestStream::new("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"system.ping\"}\n");
        serve(&mut stream, &mut PingHandler).unwrap();
        assert_eq!(
            std::str::from_utf8(&stream.write).unwrap(),
            "{\"id\":1,\"jsonrpc\":\"2.0\",\"result\":{\"service\":\"quasiquote0\"}}\n"
        );
    }

    #[test]
    fn notification_does_not_write_a_response() {
        let mut stream = TestStream::new("{\"jsonrpc\":\"2.0\",\"method\":\"system.ping\"}\n");
        serve(&mut stream, &mut PingHandler).unwrap();
        assert!(stream.write.is_empty());
    }
}
