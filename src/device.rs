//! Durable application state independent of transport and hardware workers.

use crate::config::Config;
use crate::frame::{digest_hex, parse_digest, Frame, Sha256Digest};
use crate::store::{
    DesiredFrame, DeviceState as PersistentState, FrameWriter, PullValidators, Store, StoreError,
};
use core::fmt;

pub struct Device {
    store: Store,
    config: Config,
    state: PersistentState,
    // Pull failures change far more often than the desired frame. Keep them
    // in RAM so an outage cannot turn into avoidable LittleFS wear.
    last_pull_error: Option<String>,
    pub display_clock: Option<i64>,
    pub display_refreshes: u32,
    pub display_error: Option<String>,
    pub display_refresh_mode: &'static str,
    pub display_refresh_ms: u64,
    pub partial_refresh_error: Option<String>,
    pub pull_attempts: u32,
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
    #[cfg(target_os = "espidf")]
    pub fn mount() -> Result<Self, DeviceError> {
        Self::from_store(Store::mount()?)
    }

    #[cfg(not(target_os = "espidf"))]
    pub fn open_at(root: std::path::PathBuf) -> Result<Self, DeviceError> {
        Self::from_store(Store::open_at(root)?)
    }

    fn from_store(store: Store) -> Result<Self, DeviceError> {
        let config = store.load_config_or_quarantine()?.unwrap_or_default();
        let state = store.load_state()?;
        store.remove_unreferenced_frames(&state)?;
        Ok(Self {
            store,
            config,
            state,
            last_pull_error: None,
            display_clock: None,
            display_refreshes: 0,
            display_error: None,
            display_refresh_mode: "none",
            display_refresh_ms: 0,
            partial_refresh_error: None,
            pull_attempts: 0,
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

    pub fn pull_error(&self) -> Option<&str> {
        self.last_pull_error.as_deref()
    }

    #[cfg(target_os = "espidf")]
    pub fn filesystem_info(&self) -> Result<(usize, usize), DeviceError> {
        self.store.info().map_err(Into::into)
    }

    pub fn admin_token(&self) -> Result<Option<String>, DeviceError> {
        self.store.admin_token().map_err(Into::into)
    }
    pub fn save_admin_token(&self, token: &str) -> Result<(), DeviceError> {
        self.store.save_admin_token(token).map_err(Into::into)
    }

    pub fn replace_config(&mut self, config: Config) -> Result<(), DeviceError> {
        config
            .validate()
            .map_err(|error| DeviceError(error.to_string()))?;
        if config.pull.url != self.config.pull.url
            || config.pull.bearer_token != self.config.pull.bearer_token
        {
            // Validators describe one resource and authorization context. Clear
            // them durably before changing that context: a failed config write
            // then causes an extra full fetch, never a false 304 on the new URL.
            let mut next = self.state.clone();
            next.pull = PullValidators::default();
            self.persist_state(next)?;
        }
        self.store.save_config(&config)?;
        self.config = config;
        Ok(())
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
    pub fn commit_frame(&mut self, digest: Sha256Digest) -> Result<(), DeviceError> {
        self.publish_frame(digest, None)
    }

    pub fn commit_pulled_frame(
        &mut self,
        digest: Sha256Digest,
        validators: PullValidators,
    ) -> Result<(), DeviceError> {
        self.publish_frame(digest, Some(validators))
    }

    fn publish_frame(
        &mut self,
        digest: Sha256Digest,
        validators: Option<PullValidators>,
    ) -> Result<(), DeviceError> {
        let mut next = self.state.clone();
        next.desired = Some(DesiredFrame {
            sha256: digest_hex(&digest),
        });
        if let Some(validators) = validators {
            next.pull = validators;
            self.persist_state(next)?;
            self.last_pull_error = None;
        } else {
            self.persist_state(next)?;
        }
        self.store.remove_unreferenced_frames(&self.state)?;
        Ok(())
    }

    pub fn record_pull_error(&mut self, error: String) {
        self.last_pull_error = Some(error.chars().take(160).collect());
    }

    fn persist_state(&mut self, next: PersistentState) -> Result<(), DeviceError> {
        self.store.save_state(&next)?;
        self.state = next;
        Ok(())
    }

    pub fn discard_unreferenced_frames(&self) -> Result<(), DeviceError> {
        self.store
            .remove_unreferenced_frames(&self.state)
            .map_err(Into::into)
    }

    pub fn load_desired_frame(&self) -> Result<Option<Frame>, DeviceError> {
        let Some(desired) = &self.state.desired else {
            return Ok(None);
        };
        let digest = parse_digest(&desired.sha256).map_err(|error| DeviceError(error.into()))?;
        self.store.load_frame(&digest).map_err(Into::into)
    }

    pub fn mark_displayed(
        &mut self,
        content: &Sha256Digest,
        composed: &Sha256Digest,
        firmware_version: &str,
        clock_seconds: i64,
        status_bar: bool,
    ) -> Result<(), DeviceError> {
        // Heartbeats update the acknowledged status in RAM. Persist only a
        // content/firmware transition, avoiding one LittleFS write per minute.
        let content = digest_hex(content);
        let persist = self.state.displayed_content_sha256.as_ref() != Some(&content)
            || self.state.firmware_version.as_deref() != Some(firmware_version)
            || self.state.displayed_status_bar != Some(status_bar);
        let mut next = self.state.clone();
        next.displayed_content_sha256 = Some(content);
        next.displayed_composed_sha256 = Some(digest_hex(composed));
        next.firmware_version = Some(firmware_version.to_owned());
        next.displayed_status_bar = Some(status_bar);
        if persist {
            self.store.save_state(&next)?;
        }
        self.state = next;
        self.display_clock = Some(clock_seconds);
        self.display_refreshes = self.display_refreshes.saturating_add(1);
        self.display_error = None;
        Ok(())
    }

    pub fn desired_digest(&self) -> Result<Sha256Digest, DeviceError> {
        self.state
            .desired
            .as_ref()
            .map(|desired| parse_digest(&desired.sha256).map_err(|error| DeviceError(error.into())))
            .unwrap_or_else(|| Ok(Frame::white().sha256()))
    }

    pub fn mark_ota_pending(&mut self) -> Result<(), DeviceError> {
        let mut next = self.state.clone();
        next.ota_pending = Some("pending".to_owned());
        self.persist_state(next)
    }

    #[cfg(target_os = "espidf")]
    pub fn confirm_ota(&mut self) -> Result<(), DeviceError> {
        if self.state.ota_pending.is_some() {
            crate::ota::confirm_running_image().map_err(DeviceError)?;
            let mut next = self.state.clone();
            next.ota_pending = None;
            self.persist_state(next)?;
        }
        Ok(())
    }
}
