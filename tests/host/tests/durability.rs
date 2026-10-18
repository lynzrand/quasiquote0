use quasiquote0_host_tests::{device::Device, frame::Frame, store::FrameMeta};
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

#[test]
fn rejected_upload_preserves_the_durable_frame() {
    let root = Scratch::new();
    let mut device = root.device();
    let old = Frame::white();
    device.accept_frame(&old, FrameMeta::default()).unwrap();
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
    device.accept_frame(&old, FrameMeta::default()).unwrap();
    let state_path = root.0.join("state/device.v1.json");
    let saved = fs::read(&state_path).unwrap();
    fs::remove_file(state_path).unwrap();
    fs::remove_dir(root.0.join("state")).unwrap();
    fs::write(root.0.join("state"), b"block state writes").unwrap();
    assert!(device
        .accept_frame(&Frame::black(), FrameMeta::default())
        .is_err());
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
fn reset_leaves_a_valid_schema_for_subsequent_frame_writes() {
    let root = Scratch::new();
    let mut device = root.device();
    device
        .accept_frame(&Frame::black(), FrameMeta::default())
        .unwrap();
    device.factory_reset().unwrap();
    assert_eq!(device.state().schema, 1);
    device
        .accept_frame(&Frame::white(), FrameMeta::default())
        .unwrap();
    drop(device);
    assert!(root.device().load_desired_frame().unwrap().is_some());
}

#[test]
fn a_corrupted_retained_frame_is_rejected() {
    let root = Scratch::new();
    let mut device = root.device();
    let digest = device
        .accept_frame(&Frame::white(), FrameMeta::default())
        .unwrap();
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
