//! Portable settings, credentials, and retained source image. Device access
//! codes belong to the destination device and are not part of this format.
use crate::{
    config::Config,
    frame::{parse_digest, Sha256Digest, FRAME_BYTES},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA: u32 = 1;
pub const MAX_BYTES: usize = 16 * 1024;
pub const MAX_CONFIG_BYTES: usize = 4 * 1024;

/// A verified immutable source image, ready to stream without a full Frame
/// allocation on the HTTP server stack.
pub struct RetainedFrame {
    pub digest: Sha256Digest,
    pub file: std::fs::File,
}

#[derive(Serialize)]
pub struct Backup {
    pub schema: u32,
    pub config: Config,
    pub frame: Option<FrameBackup>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrameBackup {
    #[serde(deserialize_with = "digest_text")]
    pub sha256: String,
    #[serde(rename = "hex", with = "hex_frame")]
    pub bytes: Vec<u8>,
}

fn digest_text<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    raw_hex(deserializer, 32, "Backup digest must have 64 hex digits").map(str::to_owned)
}

/// Borrow hex directly from the JSON document. Escaping is unnecessary for
/// hex; rejecting it also avoids serde allocating an unbounded unescape buffer.
fn raw_hex<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
    bytes: usize,
    error: &'static str,
) -> Result<&'de str, D::Error> {
    let raw: &'de serde_json::value::RawValue = Deserialize::deserialize(deserializer)?;
    let text = raw.get();
    if text.len() != bytes * 2 + 2 || !text.starts_with('"') || !text.ends_with('"') {
        return Err(serde::de::Error::custom(error));
    }
    Ok(&text[1..text.len() - 1])
}

impl<'de> Deserialize<'de> for Backup {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input<'a> {
            schema: u32,
            #[serde(borrow)]
            config: &'a serde_json::value::RawValue,
            frame: Option<FrameBackup>,
        }
        let input = Input::deserialize(deserializer)?;
        // Borrow the JSON first, so oversized settings cannot allocate large
        // credential strings before the backup's resource bounds are checked.
        if input.config.get().len() > MAX_CONFIG_BYTES {
            return Err(serde::de::Error::custom(
                "Backup settings exceed 4096 bytes",
            ));
        }
        let config = serde_json::from_str(input.config.get()).map_err(serde::de::Error::custom)?;
        Ok(Self {
            schema: input.schema,
            config,
            frame: input.frame,
        })
    }
}

impl Backup {
    /// Check every field before creating a staged restore or changing settings.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err("Unsupported backup schema".into());
        }
        self.config.validate().map_err(|error| error.to_string())?;
        if serde_json::to_vec(&self.config)
            .map_err(|error| error.to_string())?
            .len()
            > MAX_CONFIG_BYTES
        {
            return Err("Backup settings exceed 4096 bytes".into());
        }
        if let Some(frame) = &self.frame {
            if frame.bytes.len() != FRAME_BYTES {
                return Err("Backup image must contain exactly 5624 bytes".into());
            }
            let expected = parse_digest(&frame.sha256).map_err(str::to_owned)?;
            let actual: Sha256Digest = Sha256::digest(&frame.bytes).into();
            if actual != expected {
                return Err("Backup image digest does not match its bytes".into());
            }
        }
        Ok(())
    }
}

mod hex_frame {
    use super::*;
    use std::fmt;

    pub fn serialize<S: serde::Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        struct Hex<'a>(&'a [u8]);
        impl fmt::Display for Hex<'_> {
            fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
                for byte in self.0 {
                    write!(out, "{byte:02x}")?;
                }
                Ok(())
            }
        }
        serializer.collect_str(&Hex(bytes))
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        use serde::de::Error;
        let value = raw_hex(
            deserializer,
            FRAME_BYTES,
            "Backup image must contain exactly 5624 bytes",
        )?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(FRAME_BYTES)
            .map_err(|_| D::Error::custom("Not enough memory to import image"))?;
        for pair in value.as_bytes().chunks_exact(2) {
            let high = (pair[0] as char)
                .to_digit(16)
                .ok_or_else(|| D::Error::custom("Backup image is not hex"))?;
            let low = (pair[1] as char)
                .to_digit(16)
                .ok_or_else(|| D::Error::custom("Backup image is not hex"))?;
            bytes.push((high << 4 | low) as u8);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Frame;

    #[test]
    fn json_round_trip_preserves_credentials_and_image_bytes() {
        let mut config = Config::default();
        config.wifi.password = "backup-test-password".into();
        config.pull.bearer_token = "backup-test-token".into();
        let backup = Backup {
            schema: SCHEMA,
            config,
            frame: Some(FrameBackup {
                sha256: Frame::white().sha256_hex(),
                bytes: vec![0xff; FRAME_BYTES],
            }),
        };
        let json = serde_json::to_vec(&backup).unwrap();
        assert!(json.len() <= MAX_BYTES);
        let restored: Backup = serde_json::from_slice(&json).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.config, backup.config);
        assert_eq!(restored.frame.unwrap().bytes, vec![0xff; FRAME_BYTES]);
    }

    #[test]
    fn invalid_backup_data_is_rejected_before_publication() {
        let mut backup = Backup {
            schema: SCHEMA,
            config: Config::default(),
            frame: Some(FrameBackup {
                sha256: Frame::white().sha256_hex(),
                bytes: vec![0xff; FRAME_BYTES],
            }),
        };
        assert!(backup.validate().is_ok());
        backup.schema = 2;
        assert_eq!(backup.validate().unwrap_err(), "Unsupported backup schema");
        backup.schema = SCHEMA;
        backup.frame.as_mut().unwrap().bytes.fill(0);
        assert!(backup.validate().unwrap_err().contains("digest"));
        backup.frame.as_mut().unwrap().bytes.pop();
        assert!(backup.validate().unwrap_err().contains("5624"));
    }

    #[test]
    fn malformed_large_fields_are_rejected_before_allocating_copies() {
        // On-board review caught OOM when serde copied an overlong hex string.
        let mut value = serde_json::json!({"schema":1,"config":{},"frame":{
            "sha256":Frame::white().sha256_hex(),"hex":"f".repeat(15000)
        }});
        let parse = |value: &serde_json::Value| {
            serde_json::from_slice::<Backup>(&serde_json::to_vec(value).unwrap())
        };
        assert!(parse(&value).err().unwrap().to_string().contains("5624"));
        value["frame"]["hex"] = serde_json::json!("é".repeat(FRAME_BYTES));
        assert!(parse(&value).err().unwrap().to_string().contains("not hex"));
        value["frame"] = serde_json::Value::Null;
        value["config"] = serde_json::json!({"pull":{"bearer_token":"x".repeat(15000)}});
        assert!(parse(&value).err().unwrap().to_string().contains("4096"));
    }
}
