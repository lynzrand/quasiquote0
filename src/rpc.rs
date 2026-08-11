//! Quote/0 stream-0 JSON-RPC control methods.

use crate::config::Config;
use crate::device::Device;
use crate::jsonrpc::{Error, Handler, Request};
use crate::store::{DesiredFrame, FrameMeta};
use crate::yamux::{Yamux, YamuxStream};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

pub const PROTOCOL_VERSION: u32 = 1;

static FRAME_UPLOAD_RUNNING: AtomicBool = AtomicBool::new(false);
static OTA_RUNNING: AtomicBool = AtomicBool::new(false);

pub struct DeviceRpc {
    device: Arc<Mutex<Device>>,
    yamux: Yamux,
    reboot_requested: bool,
}

/// USB-visible recovery mode. A damaged LittleFS volume is never silently
/// reformatted; the host can still identify the failure and reflash or invoke
/// an explicit recovery operation once that method is added.
pub enum ControlRpc {
    Ready(DeviceRpc),
    Recovery { error: String },
}

impl ControlRpc {
    pub fn mount(yamux: Yamux) -> Self {
        match Device::mount() {
            Ok(mut device) => match device.confirm_ota() {
                Ok(()) => Self::Ready(DeviceRpc::new(Arc::new(Mutex::new(device)), yamux)),
                Err(error) => Self::Recovery {
                    error: error.to_string(),
                },
            },
            Err(error) => Self::Recovery {
                error: error.to_string(),
            },
        }
    }

    /// Shared durable state for the single display/pull owner. It is absent in
    /// recovery mode, where LittleFS must not be touched further.
    pub fn device_handle(&self) -> Option<Arc<Mutex<Device>>> {
        match self {
            Self::Ready(rpc) => Some(rpc.device.clone()),
            Self::Recovery { .. } => None,
        }
    }
}

impl DeviceRpc {
    pub fn new(device: Arc<Mutex<Device>>, yamux: Yamux) -> Self {
        Self {
            device,
            yamux,
            reboot_requested: false,
        }
    }

    pub fn take_reboot_request(&mut self) -> bool {
        core::mem::take(&mut self.reboot_requested)
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, Error> {
        match method {
            "system.ping" => value(PingResult {
                service: "quasiquote0",
            }),
            "device.hello" => value(self.hello()),
            "device.status" => value(self.status()?),
            "device.reboot" => {
                self.reboot_requested = true;
                value(RebootResult { rebooting: true })
            }
            "config.get" => value(ConfigGetResult {
                config: self.device.lock().unwrap().config().redacted(),
            }),
            "config.set" => self.config_set(params),
            "wifi.clear" => {
                self.device
                    .lock()
                    .unwrap()
                    .clear_wifi()
                    .map_err(device_error)?;
                value(WifiClearResult { cleared: true })
            }
            "frame.begin" => self.frame_begin(params),
            "ota.begin" => self.ota_begin(params),
            "ota.status" => value(OtaStatusResult {
                running_partition: crate::ota::running_partition_label(),
                pending: self.device.lock().unwrap().state().ota_pending.clone(),
                in_progress: OTA_RUNNING.load(Ordering::SeqCst),
            }),
            "frame.metadata" | "frame.getMetadata" => value(self.frame_metadata()),
            "diagnostics.get" => value(self.diagnostics()?),
            _ => Err(Error::method_not_found()),
        }
    }

    fn hello(&self) -> HelloResult {
        HelloResult {
            protocol: PROTOCOL_VERSION,
            firmware: env!("CARGO_PKG_VERSION"),
            device: DeviceDescription {
                model: "Quote/0",
                panel: "UC8251D",
            },
            capabilities: Capabilities {
                framebuffer: FramebufferCapabilities {
                    width: crate::frame::WIDTH,
                    height: crate::frame::HEIGHT,
                    depth: 1,
                    packed_bytes: crate::frame::FRAME_BYTES,
                    format: "mono1",
                    white_is: 1,
                    status_bar_width: crate::status_bar::WIDTH_PIXELS,
                },
                usb: UsbCapabilities {
                    jsonrpc: true,
                    ndjson: true,
                    yamux: true,
                    max_line_bytes: crate::jsonrpc::MAX_LINE,
                    device_selected_upload_streams: true,
                },
                network: NetworkCapabilities {
                    pull: false,
                    push: false,
                },
            },
        }
    }

    fn status(&self) -> Result<StatusResult, Error> {
        let battery = crate::battery::read();
        let device = self.device.lock().unwrap();
        let (total, used) = device.filesystem_info().map_err(device_error)?;
        let state = device.state();
        Ok(StatusResult {
            protocol: PROTOCOL_VERSION,
            firmware: env!("CARGO_PKG_VERSION"),
            configured: !device.config().wifi.ssid.is_empty(),
            battery: BatteryStatus {
                millivolts: battery.millivolts,
                percent: battery.percent,
                usb_present: battery.usb_present,
            },
            filesystem: FilesystemStatus {
                total_bytes: total,
                used_bytes: used,
            },
            display: DisplayStatus {
                desired_sha256: state.desired.as_ref().map(|desired| desired.sha256.clone()),
                displayed_content_sha256: state.displayed_content_sha256.clone(),
                displayed_composed_sha256: state.displayed_composed_sha256.clone(),
            },
        })
    }

    fn config_set(&mut self, params: &Value) -> Result<Value, Error> {
        let mut next = parse_config(params)?;
        let mut device = self.device.lock().unwrap();
        preserve_redacted_secrets(device.config(), &mut next);
        device.replace_config(next).map_err(device_error)?;
        value(ConfigSetResult { accepted: true })
    }

    fn frame_metadata(&self) -> FrameMetadataResult {
        let device = self.device.lock().unwrap();
        let state = device.state();
        FrameMetadataResult {
            desired: state.desired.clone(),
            displayed_content_sha256: state.displayed_content_sha256.clone(),
            displayed_composed_sha256: state.displayed_composed_sha256.clone(),
        }
    }

    fn diagnostics(&self) -> Result<DiagnosticsResult, Error> {
        let device = self.device.lock().unwrap();
        let (total, used) = device.filesystem_info().map_err(device_error)?;
        Ok(DiagnosticsResult {
            filesystem_total_bytes: total,
            filesystem_used_bytes: used,
            configured_pull: device.config().pull.enabled,
            ota_pending: device.state().ota_pending.clone(),
        })
    }

    fn frame_begin(&self, params: &Value) -> Result<Value, Error> {
        if FRAME_UPLOAD_RUNNING.swap(true, Ordering::SeqCst) {
            return Err(Error::invalid_params(
                "a frame upload is already in progress",
            ));
        }
        let params: FrameBeginParams = serde_json::from_value(params.clone()).map_err(|error| {
            FRAME_UPLOAD_RUNNING.store(false, Ordering::SeqCst);
            Error::invalid_params(error.to_string())
        })?;
        let expected = parse_sha256(&params.sha256).inspect_err(|_| {
            FRAME_UPLOAD_RUNNING.store(false, Ordering::SeqCst);
        })?;
        let stream = self.yamux.open().map_err(|error| {
            FRAME_UPLOAD_RUNNING.store(false, Ordering::SeqCst);
            transport_error(error)
        })?;
        let stream_id = stream.id();
        let device = self.device.clone();
        let metadata = FrameMeta {
            generation: params.generation,
            source: params.source.unwrap_or_else(|| "usb".to_owned()),
            accepted_at_ms: params.accepted_at_ms,
        };
        thread::Builder::new()
            .name("frame-upload".into())
            .stack_size(8 * 1024)
            .spawn(move || receive_frame_upload(device, stream, expected, metadata))
            .map_err(|error| {
                FRAME_UPLOAD_RUNNING.store(false, Ordering::SeqCst);
                transport_error(error)
            })?;
        value(FrameBeginResult {
            stream: stream_id,
            expected_bytes: crate::frame::FRAME_BYTES,
        })
    }

    fn ota_begin(&self, params: &Value) -> Result<Value, Error> {
        if OTA_RUNNING.swap(true, Ordering::SeqCst) {
            return Err(Error::invalid_params(
                "an OTA upload is already in progress",
            ));
        }
        let params: OtaBeginParams = serde_json::from_value(params.clone()).map_err(|error| {
            OTA_RUNNING.store(false, Ordering::SeqCst);
            Error::invalid_params(error.to_string())
        })?;
        let expected = parse_sha256(&params.sha256).inspect_err(|_| {
            OTA_RUNNING.store(false, Ordering::SeqCst);
        })?;
        if params.bytes == 0 {
            OTA_RUNNING.store(false, Ordering::SeqCst);
            return Err(Error::invalid_params("bytes must be non-zero"));
        }
        let stream = self.yamux.open().map_err(|error| {
            OTA_RUNNING.store(false, Ordering::SeqCst);
            transport_error(error)
        })?;
        let stream_id = stream.id();
        let device = self.device.clone();
        thread::Builder::new()
            .name("ota-upload".into())
            .stack_size(8 * 1024)
            .spawn(move || {
                receive_ota_upload(device, stream, params.bytes, expected, params.version)
            })
            .map_err(|error| {
                OTA_RUNNING.store(false, Ordering::SeqCst);
                transport_error(error)
            })?;
        value(OtaBeginResult {
            stream: stream_id,
            expected_bytes: params.bytes,
        })
    }
}

fn receive_frame_upload(
    device: Arc<Mutex<Device>>,
    mut stream: YamuxStream,
    expected: crate::frame::Sha256Digest,
    metadata: FrameMeta,
) {
    let result = (|| -> Result<(), String> {
        let mut writer = device
            .lock()
            .unwrap()
            .begin_frame_write(expected)
            .map_err(|error| error.to_string())?;
        let mut remaining = crate::frame::FRAME_BYTES;
        let mut buffer = [0u8; 512];
        while remaining != 0 {
            let chunk_len = remaining.min(buffer.len());
            let count = stream
                .read(&mut buffer[..chunk_len])
                .map_err(|error| error.to_string())?;
            if count == 0 {
                return Err("frame stream ended before the announced length".to_owned());
            }
            writer
                .write_all(&buffer[..count])
                .map_err(|error| error.to_string())?;
            remaining -= count;
        }
        // Exact length is the end-of-upload marker. Do not wait for the host
        // to close; that lets a sender reuse its own scheduling independently.
        stream.close();
        let digest = writer.finish().map_err(|error| error.to_string())?;
        device
            .lock()
            .unwrap()
            .commit_frame(digest, metadata)
            .map_err(|error| error.to_string())
    })();
    if result.is_err() {
        stream.close();
    }
    FRAME_UPLOAD_RUNNING.store(false, Ordering::SeqCst);
}

fn receive_ota_upload(
    device: Arc<Mutex<Device>>,
    mut stream: YamuxStream,
    expected_bytes: usize,
    expected: crate::frame::Sha256Digest,
    version: Option<String>,
) {
    let result = (|| -> Result<(), String> {
        crate::ota::receive(&mut stream, expected_bytes, expected)?;
        // Exact length ended the receiver's work; do not wait for peer EOF.
        stream.close();
        device
            .lock()
            .unwrap()
            .mark_ota_pending(version)
            .map_err(|error| error.to_string())?;
        crate::ota::select_staged_partition()?;
        Ok(())
    })();
    if result.is_ok() {
        // The response to `ota.begin` has already been flushed. Give queued
        // USB bytes a chance to leave before booting the selected slot.
        esp_idf_hal::delay::FreeRtos::delay_ms(250);
        unsafe { esp_idf_sys::esp_restart() };
    } else {
        stream.close();
    }
    OTA_RUNNING.store(false, Ordering::SeqCst);
}

impl Handler for DeviceRpc {
    fn handle(&mut self, request: Request) -> Result<Value, Error> {
        self.dispatch(&request.method, &request.params)
    }
}

impl Handler for ControlRpc {
    fn handle(&mut self, request: Request) -> Result<Value, Error> {
        match self {
            Self::Ready(rpc) => rpc.handle(request),
            Self::Recovery { error } => match request.method.as_str() {
                "system.ping" => value(PingResult {
                    service: "quasiquote0-recovery",
                }),
                "device.hello" => value(RecoveryHelloResult {
                    protocol: PROTOCOL_VERSION,
                    firmware: env!("CARGO_PKG_VERSION"),
                    recovery: true,
                }),
                "device.status" | "diagnostics.get" => value(RecoveryStatusResult {
                    recovery: true,
                    filesystem_error: error.clone(),
                }),
                _ => Err(Error {
                    code: -32010,
                    message: "LittleFS recovery required".to_owned(),
                }),
            },
        }
    }

    fn after_response(&mut self) {
        if let Self::Ready(rpc) = self {
            if rpc.take_reboot_request() {
                unsafe { esp_idf_sys::esp_restart() };
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ConfigSetParams {
    Wrapped { config: Config },
    Direct(Config),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameBeginParams {
    sha256: String,
    #[serde(default)]
    generation: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    accepted_at_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OtaBeginParams {
    bytes: usize,
    sha256: String,
    #[serde(default)]
    version: Option<String>,
}

fn parse_config(params: &Value) -> Result<Config, Error> {
    serde_json::from_value::<ConfigSetParams>(params.clone())
        .map(|params| match params {
            ConfigSetParams::Wrapped { config } => config,
            ConfigSetParams::Direct(config) => config,
        })
        .map_err(|error| Error::invalid_params(error.to_string()))
}

fn preserve_redacted_secrets(previous: &Config, next: &mut Config) {
    if next.wifi.password == "***" {
        next.wifi.password = previous.wifi.password.clone();
    }
    if next.pull.bearer_token == "***" {
        next.pull.bearer_token = previous.pull.bearer_token.clone();
    }
}

fn device_error(error: impl core::fmt::Display) -> Error {
    Error {
        code: -32001,
        message: error.to_string(),
    }
}

fn value(value: impl Serialize) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|error| Error {
        code: -32603,
        message: error.to_string(),
    })
}

#[derive(Serialize)]
struct PingResult {
    service: &'static str,
}

#[derive(Serialize)]
struct RecoveryHelloResult {
    protocol: u32,
    firmware: &'static str,
    recovery: bool,
}

#[derive(Serialize)]
struct RecoveryStatusResult {
    recovery: bool,
    filesystem_error: String,
}

#[derive(Serialize)]
struct HelloResult {
    protocol: u32,
    firmware: &'static str,
    device: DeviceDescription,
    capabilities: Capabilities,
}

#[derive(Serialize)]
struct DeviceDescription {
    model: &'static str,
    panel: &'static str,
}

#[derive(Serialize)]
struct Capabilities {
    framebuffer: FramebufferCapabilities,
    usb: UsbCapabilities,
    network: NetworkCapabilities,
}

#[derive(Serialize)]
struct FramebufferCapabilities {
    width: usize,
    height: usize,
    depth: u8,
    packed_bytes: usize,
    format: &'static str,
    white_is: u8,
    status_bar_width: usize,
}

#[derive(Serialize)]
struct UsbCapabilities {
    jsonrpc: bool,
    ndjson: bool,
    yamux: bool,
    max_line_bytes: usize,
    device_selected_upload_streams: bool,
}

#[derive(Serialize)]
struct NetworkCapabilities {
    pull: bool,
    push: bool,
}

#[derive(Serialize)]
struct RebootResult {
    rebooting: bool,
}

#[derive(Serialize)]
struct ConfigGetResult {
    config: Config,
}

#[derive(Serialize)]
struct ConfigSetResult {
    accepted: bool,
}

#[derive(Serialize)]
struct WifiClearResult {
    cleared: bool,
}

#[derive(Serialize)]
struct StatusResult {
    protocol: u32,
    firmware: &'static str,
    configured: bool,
    battery: BatteryStatus,
    filesystem: FilesystemStatus,
    display: DisplayStatus,
}

#[derive(Serialize)]
struct BatteryStatus {
    millivolts: Option<u32>,
    percent: Option<u32>,
    usb_present: bool,
}

#[derive(Serialize)]
struct FilesystemStatus {
    total_bytes: usize,
    used_bytes: usize,
}

#[derive(Serialize)]
struct DisplayStatus {
    desired_sha256: Option<String>,
    displayed_content_sha256: Option<String>,
    displayed_composed_sha256: Option<String>,
}

#[derive(Serialize)]
struct FrameMetadataResult {
    desired: Option<DesiredFrame>,
    displayed_content_sha256: Option<String>,
    displayed_composed_sha256: Option<String>,
}

#[derive(Serialize)]
struct FrameBeginResult {
    stream: u32,
    expected_bytes: usize,
}

#[derive(Serialize)]
struct OtaBeginResult {
    stream: u32,
    expected_bytes: usize,
}

#[derive(Serialize)]
struct OtaStatusResult {
    running_partition: Option<String>,
    pending: Option<String>,
    in_progress: bool,
}

#[derive(Serialize)]
struct DiagnosticsResult {
    filesystem_total_bytes: usize,
    filesystem_used_bytes: usize,
    configured_pull: bool,
    ota_pending: Option<String>,
}

fn parse_sha256(value: &str) -> Result<crate::frame::Sha256Digest, Error> {
    if value.len() != 64 {
        return Err(Error::invalid_params(
            "sha256 must be 64 lowercase hex characters",
        ));
    }
    let mut output = [0; 32];
    for (slot, pair) in output.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let high =
            hex(pair[0]).ok_or_else(|| Error::invalid_params("sha256 is not lowercase hex"))?;
        let low =
            hex(pair[1]).ok_or_else(|| Error::invalid_params("sha256 is not lowercase hex"))?;
        *slot = high << 4 | low;
    }
    Ok(output)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn transport_error(error: impl core::fmt::Display) -> Error {
    Error {
        code: -32002,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_redacted_existing_secrets() {
        let mut previous = Config::default();
        previous.wifi.password = "original-password".to_owned();
        previous.pull.bearer_token = "original-token".to_owned();
        let mut next = previous.redacted();
        preserve_redacted_secrets(&previous, &mut next);
        assert_eq!(next.wifi.password, "original-password");
        assert_eq!(next.pull.bearer_token, "original-token");
    }

    #[test]
    fn accepts_wrapped_and_direct_configuration() {
        assert!(parse_config(&serde_json::json!({"config": Config::default()})).is_ok());
        assert!(parse_config(&serde_json::json!(Config::default())).is_ok());
    }
}
