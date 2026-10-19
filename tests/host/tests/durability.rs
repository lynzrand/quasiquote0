use quasiquote0_host_tests::{device::Device, frame::Frame, frame::Sha256Digest};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
};
static NEXT: AtomicU32 = AtomicU32::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "quote0-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
    fn device(&self) -> Device {
        Device::open_at(self.0.clone()).unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// Exercise the streamed writer and durable publication used by HTTP uploads.
fn publish(device: &mut Device, frame: &Frame) -> Result<Sha256Digest, Box<dyn std::error::Error>> {
    let mut writer = device.begin_frame_write(frame.sha256())?;
    writer.write_all(frame.as_bytes())?;
    let digest = writer.finish()?;
    device.commit_frame(digest)?;
    Ok(digest)
}

#[test]
fn rejected_upload_preserves_the_durable_frame() {
    let root = Scratch::new();
    let mut device = root.device();
    let old = Frame::white();
    publish(&mut device, &old).unwrap();
    let mut writer = device.begin_frame_write(old.sha256()).unwrap();
    writer.write_all(Frame::black().as_bytes()).unwrap();
    assert!(writer.finish().is_err());
    drop(device);
    assert_eq!(
        root.device()
            .load_desired_frame()
            .unwrap()
            .unwrap()
            .as_bytes(),
        old.as_bytes()
    );
}

#[test]
fn failed_state_write_does_not_publish_new_desired_state_in_ram() {
    let root = Scratch::new();
    let mut device = root.device();
    let old = Frame::white();
    publish(&mut device, &old).unwrap();
    let state_path = root.0.join("state/device.v1.json");
    let saved = fs::read(&state_path).unwrap();
    fs::remove_file(state_path).unwrap();
    fs::remove_dir(root.0.join("state")).unwrap();
    fs::write(root.0.join("state"), b"block state writes").unwrap();
    assert!(publish(&mut device, &Frame::black()).is_err());
    assert_eq!(
        device.state().desired.as_ref().unwrap().sha256,
        old.sha256_hex()
    );
    fs::remove_file(root.0.join("state")).unwrap();
    fs::create_dir(root.0.join("state")).unwrap();
    fs::write(root.0.join("state/device.v1.json"), saved).unwrap();
    drop(device);
    assert_eq!(
        root.device()
            .load_desired_frame()
            .unwrap()
            .unwrap()
            .as_bytes(),
        old.as_bytes()
    );
}

#[test]
fn old_saved_state_and_configuration_keep_credentials_and_content() {
    let root = Scratch::new();
    let mut device = root.device();
    let digest = publish(&mut device, &Frame::black()).unwrap();
    drop(device);
    let state_path = root.0.join("state/device.v1.json");
    let mut state: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    state["desired"]["meta"] =
        serde_json::json!({"generation":"v1","source":"browser","accepted_at_ms":123});
    state["pull"] = serde_json::json!({"etag":"old-etag","last_modified":null,"last_success_ms":123,"last_error":null});
    fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
    fs::write(
        root.0.join("config/device.v1.json"),
        br#"{
        "schema":1,"wifi":{"ssid":"saved-network","password":"saved-secret"},
        "power":{"stay_awake_on_usb":false}
    }"#,
    )
    .unwrap();
    let mut device = root.device();
    assert_eq!(device.state().schema, 1);
    assert_eq!(device.desired_digest().unwrap(), digest);
    assert_eq!(
        device.load_desired_frame().unwrap().unwrap().as_bytes(),
        Frame::black().as_bytes()
    );
    assert_eq!(device.state().pull.etag.as_deref(), Some("old-etag"));
    assert_eq!(device.config().wifi.password, "saved-secret");
    device.replace_config(device.config().clone()).unwrap();
    let current = fs::read_to_string(root.0.join("config/device.v1.json")).unwrap();
    assert!(!current.contains("stay_awake_on_usb"));
    assert_eq!(root.device().config().wifi.ssid, "saved-network");
}

#[test]
fn failed_ota_marker_write_leaves_ram_and_flash_unchanged() {
    let root = Scratch::new();
    let mut device = root.device();
    publish(&mut device, &Frame::white()).unwrap();
    let state_path = root.0.join("state/device.v1.json");
    fs::create_dir(root.0.join("state/device.v1.json.new")).unwrap();
    assert!(device.mark_ota_pending().is_err());
    assert!(device.state().ota_pending.is_none());
    let state: serde_json::Value = serde_json::from_slice(&fs::read(state_path).unwrap()).unwrap();
    assert!(state["ota_pending"].is_null());
}

#[test]
fn a_corrupted_retained_frame_is_rejected() {
    let root = Scratch::new();
    let mut device = root.device();
    let digest = publish(&mut device, &Frame::white()).unwrap();
    fs::write(
        root.0.join(format!(
            "frames/{}.bin",
            quasiquote0_host_tests::frame::digest_hex(&digest)
        )),
        Frame::black().as_bytes(),
    )
    .unwrap();
    assert!(device.load_desired_frame().is_err());
}

#[test]
fn minute_acknowledgement_updates_ram_without_rewriting_flash() {
    let root = Scratch::new();
    let mut device = root.device();
    let content = publish(&mut device, &Frame::white()).unwrap();
    let first_composed = Frame::white().sha256();
    device
        .mark_displayed(&content, &first_composed, "test", 1_800_000_000, true)
        .unwrap();
    let state_path = root.0.join("state/device.v1.json");
    let persisted = fs::read(&state_path).unwrap();
    // A heartbeat still succeeds if a state write would fail.
    fs::create_dir(root.0.join("state/device.v1.json.new")).unwrap();
    let next_composed = Frame::black().sha256();
    device
        .mark_displayed(&content, &next_composed, "test", 1_800_000_060, true)
        .unwrap();
    assert_eq!(device.display_clock, Some(1_800_000_060));
    assert_eq!(device.display_refreshes, 2);
    assert_eq!(
        device.state().displayed_composed_sha256.as_deref(),
        Some(Frame::black().sha256_hex().as_str())
    );
    assert_eq!(fs::read(state_path).unwrap(), persisted);
}
