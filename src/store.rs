//! Power-loss-aware LittleFS persistence for application-owned state.
//!
//! The ESP-IDF NVS partition remains available to the Wi-Fi stack, but no
//! Quote/0 configuration or frame content is stored there. A factory-erased
//! LittleFS partition is formatted once; a nonblank mount failure is surfaced
//! to recovery code and is never reformatted automatically.

use crate::config::Config;
use crate::frame::{digest_hex, Frame, Sha256Digest, FRAME_BYTES};
use core::fmt;
#[cfg(target_os = "espidf")]
use esp_idf_svc::{fs::littlefs::Littlefs, io::vfs::MountedLittlefs};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(target_os = "espidf")]
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const MOUNT_PATH: &str = "/littlefs";
pub const PARTITION_LABEL: &str = "littlefs";

const CONFIG_PATH: &str = "config/device.v1.json";
const ADMIN_PATH: &str = "config/admin.v1.json";
const STATE_PATH: &str = "state/device.v1.json";
const FRAME_DIRECTORY: &str = "frames";

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct DesiredFrame {
    pub sha256: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct PullValidators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct DeviceState {
    #[serde(default = "state_schema")]
    pub schema: u32,
    pub desired: Option<DesiredFrame>,
    pub displayed_content_sha256: Option<String>,
    pub displayed_composed_sha256: Option<String>,
    #[serde(default)]
    pub displayed_status_bar: Option<bool>,
    #[serde(default)]
    pub pull: PullValidators,
    pub firmware_version: Option<String>,
    pub ota_pending: Option<String>,
}

impl Default for DeviceState {
    fn default() -> Self {
        Self {
            schema: state_schema(),
            desired: None,
            displayed_content_sha256: None,
            displayed_composed_sha256: None,
            displayed_status_bar: None,
            pull: PullValidators::default(),
            firmware_version: None,
            ota_pending: None,
        }
    }
}

#[derive(Debug)]
pub struct StoreError(String);

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

/// Keep the mounted VFS alive for the lifetime of all `std::fs` operations.
pub struct Store {
    #[cfg(target_os = "espidf")]
    _mount: MountedLittlefs<Littlefs<()>>,
    root: PathBuf,
}

// ESP-IDF's VFS serializes LittleFS operations. `MountedLittlefs` carries raw
// partition pointers and therefore conservatively opts out of `Send`, but this
// wrapper never exposes it: every Store access is serialized by Device's
// mutex, and the mount remains alive until the whole device state is dropped.
// This is required for a transfer worker to write while the USB control task
// remains responsive.
unsafe impl Send for Store {}

/// An uncommitted frame write. Data is written directly to LittleFS as it is
/// received. Its temporary file is removed unless [`finish`](Self::finish)
/// verifies both the exact canonical size and caller-announced digest.
pub struct FrameWriter {
    temporary: PathBuf,
    final_path: Option<PathBuf>,
    expected: Option<Sha256Digest>,
    file: Option<File>,
    hasher: Sha256,
    written: usize,
}

impl Store {
    #[cfg(target_os = "espidf")]
    pub fn mount() -> Result<Self, StoreError> {
        let blank = partition_is_erased()?;
        let mut filesystem =
            unsafe { Littlefs::new_partition(PARTITION_LABEL) }.map_err(esp_error)?;
        if blank {
            filesystem.format().map_err(esp_error)?;
        }
        let mount = MountedLittlefs::mount(filesystem, MOUNT_PATH).map_err(esp_error)?;
        let store = Self {
            _mount: mount,
            root: PathBuf::from(MOUNT_PATH),
        };
        store.ensure_layout()?;
        store.remove_abandoned_temporary_files()?;
        Ok(store)
    }

    #[cfg(not(target_os = "espidf"))]
    pub fn open_at(root: PathBuf) -> Result<Self, StoreError> {
        let store = Self { root };
        store.ensure_layout()?;
        store.remove_abandoned_temporary_files()?;
        Ok(store)
    }

    pub fn admin_token(&self) -> Result<Option<String>, StoreError> {
        self.read_json_optional(ADMIN_PATH)
    }

    pub fn save_admin_token(&self, token: &str) -> Result<(), StoreError> {
        self.write_json_atomic(ADMIN_PATH, &token)
    }

    pub fn load_config(&self) -> Result<Option<Config>, StoreError> {
        let config: Option<Config> = self.read_json_optional(CONFIG_PATH)?;
        if let Some(config) = &config {
            config
                .validate()
                .map_err(|error| StoreError(error.to_string()))?;
        }
        Ok(config)
    }

    /// A configuration is replaceable state, not a reason to strand a device
    /// in recovery. Preserve an invalid document for inspection and continue
    /// with the safe, unconfigured default instead. Mount/IO failures still
    /// propagate because formatting or guessing would risk the valid frame.
    pub fn load_config_or_quarantine(&self) -> Result<Option<Config>, StoreError> {
        match self.load_config() {
            Ok(config) => Ok(config),
            Err(_) => {
                self.quarantine(CONFIG_PATH)?;
                Ok(None)
            }
        }
    }

    pub fn save_config(&self, config: &Config) -> Result<(), StoreError> {
        config
            .validate()
            .map_err(|error| StoreError(error.to_string()))?;
        self.write_json_atomic(CONFIG_PATH, config)
    }

    pub fn load_state(&self) -> Result<DeviceState, StoreError> {
        let state: DeviceState = match self.read_json_optional(STATE_PATH)? {
            Some(state) => state,
            None => DeviceState::default(),
        };
        if state.schema != state_schema() {
            return Err(StoreError("unsupported device-state schema".to_owned()));
        }
        Ok(state)
    }

    /// Delete immutable content only after a durable state document identifies
    /// its replacement. This also collects complete but unreferenced files
    /// left by a rejected HTTP digest or a power loss after frame publication.
    pub fn remove_unreferenced_frames(&self, state: &DeviceState) -> Result<(), StoreError> {
        let retained = state
            .desired
            .as_ref()
            .map(|desired| self.path(&format!("{FRAME_DIRECTORY}/{}.bin", desired.sha256)));
        for entry in fs::read_dir(self.path(FRAME_DIRECTORY))? {
            let entry = entry?;
            if entry.file_type()?.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "bin")
                && retained.as_ref() != Some(&entry.path())
            {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }

    pub fn save_state(&self, state: &DeviceState) -> Result<(), StoreError> {
        if state.schema != state_schema() {
            return Err(StoreError("unsupported device-state schema".to_owned()));
        }
        self.write_json_atomic(STATE_PATH, state)
    }

    /// Start an atomic, bounded write for a content-addressed canonical frame.
    pub fn begin_frame_write(&self, expected: Sha256Digest) -> Result<FrameWriter, StoreError> {
        self.begin_frame_write_inner(Some(expected))
    }

    pub fn begin_unannounced_frame_write(&self) -> Result<FrameWriter, StoreError> {
        self.begin_frame_write_inner(None)
    }

    fn begin_frame_write_inner(
        &self,
        expected: Option<Sha256Digest>,
    ) -> Result<FrameWriter, StoreError> {
        let final_path = expected.map(|digest| self.frame_path(&digest));
        let temporary = final_path
            .as_ref()
            .map(|path| PathBuf::from(format!("{}.new", path.display())))
            .unwrap_or_else(|| self.path(&format!("{FRAME_DIRECTORY}/http-pull.bin.new")));
        match fs::remove_file(&temporary) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let file = if final_path.as_ref().is_some_and(|path| path.exists()) {
            None
        } else {
            Some(
                OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&temporary)?,
            )
        };
        Ok(FrameWriter {
            temporary,
            final_path,
            expected,
            file,
            hasher: Sha256::new(),
            written: 0,
        })
    }

    pub fn load_frame(&self, digest: &Sha256Digest) -> Result<Option<Frame>, StoreError> {
        let path = self.frame_path(digest);
        if !path.exists() {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;
        let frame = Frame::from_bytes(&bytes)
            .map_err(|_| StoreError("stored frame has invalid length".to_owned()))?;
        if frame.sha256() != *digest {
            return Err(StoreError("stored frame digest does not match".to_owned()));
        }
        Ok(Some(frame))
    }

    #[cfg(target_os = "espidf")]
    pub fn info(&self) -> Result<(usize, usize), StoreError> {
        self._mount
            .info()
            .map(|info| (info.total_bytes, info.used_bytes))
            .map_err(esp_error)
    }

    fn ensure_layout(&self) -> Result<(), StoreError> {
        fs::create_dir_all(self.path("config"))?;
        fs::create_dir_all(self.path("state"))?;
        fs::create_dir_all(self.path(FRAME_DIRECTORY))?;
        Ok(())
    }

    fn remove_abandoned_temporary_files(&self) -> Result<(), StoreError> {
        for directory in ["config", "state", FRAME_DIRECTORY] {
            for entry in fs::read_dir(self.path(directory))? {
                let entry = entry?;
                if entry.file_type()?.is_file()
                    && entry.file_name().to_string_lossy().ends_with(".new")
                {
                    fs::remove_file(entry.path())?;
                }
            }
        }
        Ok(())
    }

    fn read_json_optional<T: DeserializeOwned>(
        &self,
        relative: &str,
    ) -> Result<Option<T>, StoreError> {
        let path = self.path(relative);
        match File::open(path) {
            Ok(mut file) => {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                Ok(Some(serde_json::from_slice(&bytes)?))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn write_json_atomic<T: Serialize>(&self, relative: &str, value: &T) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(value)?;
        write_atomic(&self.path(relative), &bytes)
    }

    fn quarantine(&self, relative: &str) -> Result<(), StoreError> {
        let original = self.path(relative);
        if !original.exists() {
            return Ok(());
        }
        for suffix in 0..100 {
            let candidate = PathBuf::from(format!("{}.corrupt.{suffix}", original.display()));
            if !candidate.exists() {
                fs::rename(original, candidate)?;
                return Ok(());
            }
        }
        Err(StoreError(
            "too many quarantined configuration files".to_owned(),
        ))
    }

    fn frame_path(&self, digest: &Sha256Digest) -> PathBuf {
        self.path(&format!("{FRAME_DIRECTORY}/{}.bin", digest_hex(digest)))
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

impl Write for FrameWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let remaining = FRAME_BYTES.saturating_sub(self.written);
        if bytes.len() > remaining {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "frame exceeds canonical length",
            ));
        }
        if let Some(file) = self.file.as_mut() {
            file.write_all(bytes)?;
        }
        self.hasher.update(bytes);
        self.written += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(file) = self.file.as_mut() {
            file.flush()
        } else {
            Ok(())
        }
    }
}

impl FrameWriter {
    pub fn finish(mut self) -> Result<Sha256Digest, StoreError> {
        if self.written != FRAME_BYTES {
            return Err(StoreError("frame has incorrect length".to_owned()));
        }
        let actual: Sha256Digest = self.hasher.finalize_reset().into();
        if self.expected.is_some_and(|expected| actual != expected) {
            return Err(StoreError(
                "frame digest does not match announcement".to_owned(),
            ));
        }
        let final_path = self.final_path.take().unwrap_or_else(|| {
            self.temporary
                .with_file_name(format!("{}.bin", digest_hex(&actual)))
        });
        if let Some(file) = self.file.take() {
            file.sync_all()?;
            drop(file);
            let metadata = fs::metadata(&self.temporary)?;
            if metadata.len() != FRAME_BYTES as u64 {
                return Err(StoreError("temporary frame verification failed".to_owned()));
            }
            if final_path.exists() {
                fs::remove_file(&self.temporary)?;
            } else {
                fs::rename(&self.temporary, final_path)?;
            }
        }
        Ok(actual)
    }
}

impl Drop for FrameWriter {
    fn drop(&mut self) {
        let _ = self.file.take();
        let _ = fs::remove_file(&self.temporary);
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let temporary = PathBuf::from(format!("{}.new", path.display()));
    match fs::remove_file(&temporary) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);

    // Reopen the temporary file before it becomes active so corrupt writes
    // cannot replace a valid destination merely because the write syscall
    // itself succeeded.
    let mut check = Vec::with_capacity(bytes.len());
    File::open(&temporary)?.read_to_end(&mut check)?;
    if check != bytes {
        return Err(StoreError("temporary file verification failed".to_owned()));
    }
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(target_os = "espidf")]
fn partition_is_erased() -> Result<bool, StoreError> {
    let label = CString::new(PARTITION_LABEL).expect("partition labels cannot contain NUL");
    let partition = unsafe {
        esp_idf_sys::esp_partition_find_first(
            esp_idf_sys::esp_partition_type_t_ESP_PARTITION_TYPE_DATA,
            esp_idf_sys::esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_DATA_LITTLEFS,
            label.as_ptr(),
        )
    };
    if partition.is_null() {
        return Err(StoreError("LittleFS partition not found".to_owned()));
    }
    let mut probe = [0u8; 64];
    esp_idf_sys::esp!(unsafe {
        esp_idf_sys::esp_partition_read(partition, 0, probe.as_mut_ptr().cast(), probe.len())
    })
    .map_err(esp_error)?;
    Ok(probe.iter().all(|byte| *byte == 0xff))
}

#[cfg(target_os = "espidf")]
fn esp_error(error: esp_idf_sys::EspError) -> StoreError {
    StoreError(error.to_string())
}

const fn state_schema() -> u32 {
    1
}
