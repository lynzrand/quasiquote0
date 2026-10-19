//! Shared RTC wall clock for NTP synchronization and the minute heartbeat.

use crate::clock_face::{self, ClockFace};
use serde_json::{json, Value};

pub fn unix_seconds() -> i64 {
    let mut now = esp_idf_sys::timeval::default();
    unsafe { esp_idf_sys::gettimeofday(&mut now, core::ptr::null_mut()) };
    now.tv_sec
}

pub fn status() -> Value {
    let seconds = unix_seconds();
    json!({"unix_seconds":seconds, "time":ClockFace::at(seconds).text(),
        "synchronized":clock_face::valid_time(seconds),
        "utc_offset_minutes":clock_face::UTC_OFFSET_MINUTES})
}
