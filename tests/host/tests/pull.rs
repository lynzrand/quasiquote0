//! Run the production pull client with an SDK HTTP double for bodyless replies.
#![allow(dead_code)]
extern crate self as embedded_svc;
extern crate self as esp_idf_svc;
extern crate self as esp_idf_sys;

pub use quasiquote0_host_tests::{config, device, frame, store};
#[path = "../../../src/pull.rs"]
mod pull;

use std::cell::Cell;
thread_local! {
    static HTTP_STATUS: Cell<u16> = const { Cell::new(304) };
}

pub fn esp_crt_bundle_attach() {}
/// # Safety
/// SDK-compatible double; no pointers are dereferenced.
pub unsafe fn esp_http_client_get_and_clear_last_tls_error(
    _: usize,
    _: *mut i32,
    _: *mut i32,
) -> i32 {
    0
}
/// # Safety
/// SDK-compatible double; no handle is dereferenced.
pub unsafe fn esp_http_client_get_errno(_: usize) -> i32 {
    0
}

pub mod handle {
    pub trait RawHandle {
        fn handle(&self) -> usize;
    }
}
pub mod http {
    pub enum Method {
        Get,
    }
    pub mod client {
        use std::{io, time::Duration};
        #[derive(Default)]
        pub struct Configuration {
            pub timeout: Option<Duration>,
            pub crt_bundle_attach: Option<fn()>,
            pub follow_redirects_policy: FollowRedirectsPolicy,
            pub sdk_reserved: u8,
        }
        #[derive(Default)]
        pub enum FollowRedirectsPolicy {
            #[default]
            FollowNone,
        }
        pub struct EspHttpConnection;
        impl crate::handle::RawHandle for EspHttpConnection {
            fn handle(&self) -> usize {
                0
            }
        }
        impl EspHttpConnection {
            pub fn new(_: &Configuration) -> io::Result<Self> {
                Ok(Self)
            }
            pub fn initiate_request(
                &mut self,
                _: super::Method,
                _: &str,
                _: &[(&str, &str)],
            ) -> io::Result<()> {
                Ok(())
            }
            pub fn initiate_response(&mut self) -> io::Result<()> {
                Ok(())
            }
            pub fn status(&self) -> u16 {
                crate::HTTP_STATUS.get()
            }
            pub fn header(&self, _: &str) -> Option<&str> {
                None
            }
            pub fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Ok(0)
            }
        }
    }
}

#[test]
fn bodyless_success_clears_previous_errors_without_writing_flash() {
    use std::{fs, io::Write, sync::Mutex};
    // Review found that 204/304 reset retry backoff but left last_error stale.
    for code in [204, 304] {
        HTTP_STATUS.set(code);
        let root =
            std::env::temp_dir().join(format!("quote0-pull-success-{}-{code}", std::process::id()));
        let mut device = device::Device::open_at(root.clone()).unwrap();
        let source = frame::Frame::white();
        let mut writer = device.begin_frame_write(source.sha256()).unwrap();
        writer.write_all(source.as_bytes()).unwrap();
        device.commit_frame(writer.finish().unwrap()).unwrap();
        device.record_pull_error("previous timeout".into());
        let state = root.join("state/device.v1.json");
        let saved = fs::read(&state).unwrap();
        fs::create_dir(root.join("state/device.v1.json.new")).unwrap();
        let device = Mutex::new(device);
        pull::update(&device).unwrap();
        assert_eq!(device.lock().unwrap().pull_error(), None);
        assert_eq!(
            device.lock().unwrap().desired_digest().unwrap(),
            source.sha256()
        );
        assert_eq!(fs::read(&state).unwrap(), saved);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn rejected_response_does_not_clear_the_previous_error() {
    use std::{fs, sync::Mutex};
    HTTP_STATUS.set(500);
    let root = std::env::temp_dir().join(format!("quote0-pull-failure-{}", std::process::id()));
    let mut device = device::Device::open_at(root.clone()).unwrap();
    device.record_pull_error("previous timeout".into());
    let device = Mutex::new(device);
    assert_eq!(
        pull::update(&device).unwrap_err(),
        "HTTP pull returned status 500"
    );
    assert_eq!(
        device.lock().unwrap().pull_error(),
        Some("previous timeout")
    );
    assert!(device.lock().unwrap().state().desired.is_none());
    fs::remove_dir_all(root).unwrap();
}
