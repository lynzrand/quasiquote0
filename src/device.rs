//! Durable application state independent of transport and hardware workers.

use crate::config::Config;
use crate::frame::{digest_hex, Frame, Sha256Digest};
use crate::store::{
    DesiredFrame, DeviceState as PersistentState, FrameMeta, FrameWriter, PullValidators, Store,
    StoreError,
};
use core::fmt;

pub struct Device {
    store: Store,
    config: Config,
    state: PersistentState,
}

#[derive(Debug)]
pub struct DeviceError(String);

impl fmt::Display for DeviceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DeviceError {}

impl From<StoreError> for DeviceError {
    fn from(error: StoreError) -> Self {
        Self(error.to_string())
    }
}

impl Device {
    pub fn mount() -> Result<Self, DeviceError> {
        let store = Store::mount()?;
        let config = store.load_config()?.unwrap_or_default();
        let state = store.load_state()?;
        Ok(Self {
            store,
            config,
            state,
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn state(&self) -> &PersistentState {
        &self.state
    }

    pub fn pull_snapshot(&self) -> (Config, PullValidators) {
        (self.config.clone(), self.state.pull.clone())
    }

    pub fn filesystem_info(&self) -> Result<(usize, usize), DeviceError> {
        self.store.info().map_err(Into::into)
    }

    pub fn replace_config(&mut self, config: Config) -> Result<(), DeviceError> {
        config
            .validate()
            .map_err(|error| DeviceError(error.to_string()))?;
        self.store.save_config(&config)?;
        self.config = config;
        Ok(())
    }

    pub fn clear_wifi(&mut self) -> Result<(), DeviceError> {
        let mut config = self.config.clone();
        config.wifi.ssid.clear();
        config.wifi.password.clear();
        config.pull.enabled = false;
        self.replace_config(config)
    }

    /// Make a complete, verified frame durable before pointing state at it.
    pub fn accept_frame(
        &mut self,
        frame: &Frame,
        meta: FrameMeta,
    ) -> Result<Sha256Digest, DeviceError> {
        let digest = self.store.save_frame(frame)?;
        self.state.desired = Some(DesiredFrame {
            sha256: digest_hex(&digest),
            meta,
        });
        self.store.save_state(&self.state)?;
        Ok(digest)
    }

    pub fn begin_frame_write(&self, expected: Sha256Digest) -> Result<FrameWriter, DeviceError> {
        self.store.begin_frame_write(expected).map_err(Into::into)
    }

    pub fn begin_unannounced_frame_write(&self) -> Result<FrameWriter, DeviceError> {
        self.store
            .begin_unannounced_frame_write()
            .map_err(Into::into)
    }

    /// Point durable state at a frame after its writer has verified and
    /// atomically published the content-addressed file.
    pub fn commit_frame(
        &mut self,
        digest: Sha256Digest,
        meta: FrameMeta,
    ) -> Result<(), DeviceError> {
        self.state.desired = Some(DesiredFrame {
            sha256: digest_hex(&digest),
            meta,
        });
        self.store.save_state(&self.state)?;
        Ok(())
    }

    pub fn commit_pulled_frame(
        &mut self,
        digest: Sha256Digest,
        meta: FrameMeta,
        etag: Option<String>,
        last_modified: Option<String>,
        now_ms: u64,
    ) -> Result<(), DeviceError> {
        self.state.desired = Some(DesiredFrame {
            sha256: digest_hex(&digest),
            meta,
        });
        self.state.pull.etag = etag;
        self.state.pull.last_modified = last_modified;
        self.state.pull.last_success_ms = Some(now_ms);
        self.state.pull.last_error = None;
        self.store.save_state(&self.state)?;
        Ok(())
    }

    pub fn record_pull_error(&mut self, error: String) -> Result<(), DeviceError> {
        self.state.pull.last_error = Some(error.chars().take(160).collect());
        self.store.save_state(&self.state)?;
        Ok(())
    }

    pub fn load_desired_frame(&self) -> Result<Option<Frame>, DeviceError> {
        let Some(desired) = &self.state.desired else {
            return Ok(None);
        };
        let digest = parse_digest(&desired.sha256)?;
        self.store.load_frame(&digest).map_err(Into::into)
    }

    pub fn mark_displayed(
        &mut self,
        content: &Sha256Digest,
        composed: &Sha256Digest,
        firmware_version: &str,
    ) -> Result<(), DeviceError> {
        self.state.displayed_content_sha256 = Some(digest_hex(content));
        self.state.displayed_composed_sha256 = Some(digest_hex(composed));
        self.state.firmware_version = Some(firmware_version.to_owned());
        self.store.save_state(&self.state)?;
        Ok(())
    }

    pub fn invalidate_displayed(&mut self) -> Result<(), DeviceError> {
        self.state.displayed_content_sha256 = None;
        self.state.displayed_composed_sha256 = None;
        self.store.save_state(&self.state)?;
        Ok(())
    }

    pub fn mark_ota_pending(&mut self, version: Option<String>) -> Result<(), DeviceError> {
        self.state.ota_pending = Some(version.unwrap_or_else(|| "pending".to_owned()));
        self.store.save_state(&self.state)?;
        Ok(())
    }

    pub fn confirm_ota(&mut self) -> Result<(), DeviceError> {
        if self.state.ota_pending.is_some() {
            crate::ota::confirm_running_image().map_err(DeviceError)?;
            self.state.ota_pending = None;
            self.store.save_state(&self.state)?;
        }
        Ok(())
    }
}

fn parse_digest(value: &str) -> Result<Sha256Digest, DeviceError> {
    if value.len() != 64 {
        return Err(DeviceError("stored digest has invalid length".to_owned()));
    }
    let mut output = [0; 32];
    for (slot, chars) in output.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let high =
            hex(chars[0]).ok_or_else(|| DeviceError("stored digest is not hex".to_owned()))?;
        let low =
            hex(chars[1]).ok_or_else(|| DeviceError("stored digest is not hex".to_owned()))?;
        *slot = high << 4 | low;
    }
    Ok(output)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exact_hex_digests() {
        let digest = [0xab; 32];
        assert_eq!(parse_digest(&digest_hex(&digest)).unwrap(), digest);
    }

    #[test]
    fn rejects_bad_hex_digests() {
        assert!(parse_digest("bad").is_err());
        assert!(parse_digest(&"g".repeat(64)).is_err());
    }
}
