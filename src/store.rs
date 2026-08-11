//! Power-loss-aware LittleFS persistence for application-owned state.
//!
//! The ESP-IDF NVS partition remains available to the Wi-Fi stack, but no
//! Quote/0 configuration or frame content is stored there. A factory-erased
//! LittleFS partition is formatted once; a nonblank mount failure is surfaced
//! to recovery code and is never reformatted automatically.

use crate::config::Config;
use crate::frame::{digest_hex, Frame, Sha256Digest};
use core::fmt;
use esp_idf_svc::fs::littlefs::Littlefs;
use esp_idf_svc::io::vfs::MountedLittlefs;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const MOUNT_PATH: &str = "/littlefs";
pub const PARTITION_LABEL: &str = "littlefs";

const CONFIG_PATH: &str = "config/device.v1.json";
const STATE_PATH: &str = "state/device.v1.json";
const FRAME_DIRECTORY: &str = "frames";

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct FrameMeta {
    pub generation: String,
    pub source: String,
    pub accepted_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct DesiredFrame {
    pub sha256: String,
    pub meta: FrameMeta,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct PullValidators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub last_success_ms: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct DeviceState {
    #[serde(default = "state_schema")]
    pub schema: u32,
    pub desired: Option<DesiredFrame>,
    pub displayed_content_sha256: Option<String>,
    pub displayed_composed_sha256: Option<String>,
    #[serde(default)]
    pub pull: PullValidators,
    pub firmware_version: Option<String>,
    pub ota_pending: Option<String>,
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
    _mount: MountedLittlefs<Littlefs<()>>,
    root: PathBuf,
}

impl Store {
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

    pub fn load_config(&self) -> Result<Option<Config>, StoreError> {
        let config: Option<Config> = self.read_json_optional(CONFIG_PATH)?;
        if let Some(config) = &config {
            config
                .validate()
                .map_err(|error| StoreError(error.to_string()))?;
        }
        Ok(config)
    }

    pub fn save_config(&self, config: &Config) -> Result<(), StoreError> {
        config
            .validate()
            .map_err(|error| StoreError(error.to_string()))?;
        self.write_json_atomic(CONFIG_PATH, config)
    }

    pub fn clear_config(&self) -> Result<(), StoreError> {
        self.remove_optional(CONFIG_PATH)
    }

    pub fn load_state(&self) -> Result<DeviceState, StoreError> {
        let state: DeviceState = self.read_json_optional(STATE_PATH)?.unwrap_or_default();
        if state.schema != state_schema() {
            return Err(StoreError("unsupported device-state schema".to_owned()));
        }
        Ok(state)
    }

    pub fn save_state(&self, state: &DeviceState) -> Result<(), StoreError> {
        if state.schema != state_schema() {
            return Err(StoreError("unsupported device-state schema".to_owned()));
        }
        self.write_json_atomic(STATE_PATH, state)
    }

    /// Persist an immutable content-addressed frame before a state file refers
    /// to it. A power loss may leave an orphan frame, but never a state pointer
    /// to a partial frame.
    pub fn save_frame(&self, frame: &Frame) -> Result<Sha256Digest, StoreError> {
        let digest = frame.sha256();
        let path = self.frame_path(&digest);
        if !path.exists() {
            write_atomic(&path, frame.as_bytes())?;
        }
        Ok(digest)
    }

    pub fn load_frame(&self, digest: &Sha256Digest) -> Result<Option<Frame>, StoreError> {
        let path = self.frame_path(digest);
        if !path.exists() {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;
        Frame::from_bytes(&bytes)
            .map(Some)
            .map_err(|_| StoreError("stored frame has invalid length".to_owned()))
    }

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
        fs::create_dir_all(self.path("tls"))?;
        Ok(())
    }

    fn remove_abandoned_temporary_files(&self) -> Result<(), StoreError> {
        for directory in ["config", "state", FRAME_DIRECTORY, "tls"] {
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

    fn remove_optional(&self, relative: &str) -> Result<(), StoreError> {
        match fs::remove_file(self.path(relative)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn frame_path(&self, digest: &Sha256Digest) -> PathBuf {
        self.path(&format!("{FRAME_DIRECTORY}/{}.bin", digest_hex(digest)))
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
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

fn esp_error(error: esp_idf_sys::EspError) -> StoreError {
    StoreError(error.to_string())
}

const fn state_schema() -> u32 {
    1
}
