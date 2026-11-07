//! Exercise the production OTA receiver without writing a physical partition.
extern crate self as esp_idf_sys;

pub use quasiquote0_host_tests::frame;
#[path = "../../../src/ota.rs"]
mod ota;

use std::{cell::RefCell, io::Cursor};

#[derive(Default)]
struct State {
    missing_slot: bool,
    write_error: bool,
    end_error: bool,
    began: usize,
    aborted: usize,
    ended: usize,
    selected: usize,
    bytes: Vec<u8>,
}
thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

#[allow(non_camel_case_types)]
pub type esp_ota_handle_t = u32;
pub struct Partition {
    pub label: [u8; 16],
}
static PARTITION: Partition = Partition {
    label: [111, 116, 97, 95, 49, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
};

#[macro_export]
macro_rules! esp {
    ($call:expr) => {
        if $call == 0 {
            Ok(())
        } else {
            Err("SDK failure")
        }
    };
}

/// # Safety
/// SDK-compatible double returning immutable partition storage.
pub unsafe fn esp_ota_get_next_update_partition(_: *const Partition) -> *const Partition {
    STATE.with_borrow(|state| {
        if state.missing_slot {
            core::ptr::null()
        } else {
            &PARTITION
        }
    })
}

/// # Safety
/// `handle` must point to writable storage for an OTA handle.
pub unsafe fn esp_ota_begin(_: *const Partition, _: usize, handle: *mut u32) -> i32 {
    *handle = 1;
    STATE.with_borrow_mut(|state| state.began += 1);
    0
}

/// # Safety
/// `data` must remain readable for `length` bytes.
pub unsafe fn esp_ota_write(_: u32, data: *const core::ffi::c_void, length: usize) -> i32 {
    STATE.with_borrow_mut(|state| {
        if state.write_error {
            return -1;
        }
        state
            .bytes
            .extend_from_slice(core::slice::from_raw_parts(data.cast::<u8>(), length));
        0
    })
}
/// # Safety
/// SDK-compatible double; no handle is dereferenced.
pub unsafe fn esp_ota_abort(_: u32) -> i32 {
    STATE.with_borrow_mut(|state| state.aborted += 1);
    0
}
/// # Safety
/// SDK-compatible double; no handle is dereferenced.
pub unsafe fn esp_ota_end(_: u32) -> i32 {
    STATE.with_borrow_mut(|state| {
        state.ended += 1;
        if state.end_error {
            -1
        } else {
            0
        }
    })
}
/// # Safety
/// SDK-compatible double; no partition is dereferenced.
pub unsafe fn esp_ota_set_boot_partition(_: *const Partition) -> i32 {
    STATE.with_borrow_mut(|state| state.selected += 1);
    0
}
/// # Safety
/// SDK-compatible double; no hardware is accessed.
pub unsafe fn esp_ota_mark_app_valid_cancel_rollback() -> i32 {
    0
}
/// # Safety
/// SDK-compatible double returning immutable partition storage.
pub unsafe fn esp_ota_get_running_partition() -> *const Partition {
    &PARTITION
}

fn reset() {
    STATE.with_borrow_mut(|state| *state = State::default());
}
fn digest(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).into()
}

#[test]
fn verified_stream_is_staged_without_selecting_a_boot_partition() {
    reset();
    let bytes = vec![0xa5; 1207]; // Exercise multiple chunks and a short tail.
    ota::receive(&mut Cursor::new(&bytes), bytes.len(), digest(&bytes)).unwrap();
    STATE.with_borrow(|state| {
        assert_eq!(state.bytes, bytes);
        assert_eq!(
            (state.began, state.ended, state.aborted, state.selected),
            (1, 1, 0, 0)
        );
    });
    // Durable pending-marker publication belongs between these two calls.
    ota::select_staged_partition().unwrap();
    STATE.with_borrow(|state| assert_eq!(state.selected, 1));
}

#[test]
fn mismatched_digest_aborts_before_validation_and_boot_selection() {
    reset();
    assert!(ota::receive(&mut Cursor::new([0xa5; 1207]), 1207, [0; 32])
        .unwrap_err()
        .contains("digest"));
    STATE.with_borrow(|state| {
        assert_eq!((state.aborted, state.ended, state.selected), (1, 0, 0));
    });
}

#[test]
fn interrupted_body_and_flash_errors_abort_the_staged_image() {
    for write_error in [false, true] {
        reset();
        STATE.with_borrow_mut(|state| state.write_error = write_error);
        let error = ota::receive(&mut Cursor::new([0xa5; 128]), 1207, [0; 32]).unwrap_err();
        assert!(error.contains(if write_error { "SDK" } else { "ended" }));
        STATE.with_borrow(|state| {
            assert_eq!((state.aborted, state.ended, state.selected), (1, 0, 0));
        });
    }
}

#[test]
fn empty_images_and_missing_slots_do_not_start_a_flash_write() {
    reset();
    assert!(ota::receive(&mut Cursor::new([]), 0, [0; 32])
        .unwrap_err()
        .contains("non-zero"));
    STATE.with_borrow_mut(|state| state.missing_slot = true);
    assert!(ota::receive(&mut Cursor::new([0]), 1, [0; 32])
        .unwrap_err()
        .contains("inactive"));
    assert!(ota::select_staged_partition().is_err());
    STATE.with_borrow(|state| assert_eq!((state.began, state.selected), (0, 0)));
}

#[test]
fn sdk_image_rejection_leaves_boot_selection_unchanged() {
    reset();
    STATE.with_borrow_mut(|state| state.end_error = true);
    assert!(
        ota::receive(&mut Cursor::new([0xa5; 1207]), 1207, digest(&[0xa5; 1207]))
            .unwrap_err()
            .contains("SDK")
    );
    STATE.with_borrow(|state| assert_eq!((state.ended, state.selected), (1, 0)));
}

#[test]
fn boot_confirmation_and_running_slot_use_the_sdk() {
    reset();
    assert_eq!(ota::running_partition_label().as_deref(), Some("ota_1"));
    ota::confirm_running_image().unwrap();
}
