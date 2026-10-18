//! Bounded newline messages; no streams, request IDs, or binary uploads.
use serde::Deserialize;
pub const MAX_LINE: usize = 1024;
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Status,
    Wifi { ssid: String, password: String },
    ClearWifi,
}
#[derive(Default)]
pub struct Lines {
    bytes: Vec<u8>,
    discarding: bool,
}
impl Lines {
    pub fn push(&mut self, byte: u8) -> Option<Result<Command, &'static str>> {
        if byte == b'\r' {
            return None;
        }
        if byte == b'\n' {
            let overflow = self.discarding;
            self.discarding = false;
            let bytes = std::mem::take(&mut self.bytes);
            if overflow {
                return Some(Err("Setup command exceeds 1024 bytes"));
            }
            if bytes.is_empty() {
                return None;
            }
            return Some(serde_json::from_slice(&bytes).map_err(|_| "Invalid setup command"));
        }
        if !self.discarding {
            if self.bytes.len() == MAX_LINE {
                self.bytes.clear();
                self.discarding = true;
            } else {
                self.bytes.push(byte);
            }
        }
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflow_is_discarded_through_newline_then_setup_recovers() {
        let mut lines = Lines::default();
        for _ in 0..MAX_LINE + 10 {
            assert!(lines.push(b'x').is_none());
        }
        assert!(lines.push(b'\n').unwrap().is_err());
        let mut result = None;
        for byte in b"{\"cmd\":\"status\"}\n" {
            result = lines.push(*byte);
        }
        assert!(matches!(result, Some(Ok(Command::Status))));
    }
    #[test]
    fn wifi_credentials_allow_escaped_characters_without_extra_commands() {
        let input = br#"{"cmd":"wifi","ssid":"desk","password":"a\n\"b"}"#;
        let mut lines = Lines::default();
        for byte in input {
            assert!(lines.push(*byte).is_none());
        }
        match lines.push(b'\n').unwrap().unwrap() {
            Command::Wifi { password, .. } => assert_eq!(password, "a\n\"b"),
            _ => panic!(),
        }
    }
}
