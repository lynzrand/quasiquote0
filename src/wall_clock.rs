//! Shared RTC wall clock for USB synchronization and the minute heartbeat.

use crate::clock_face::{self, ClockFace};
use serde_json::{json, Value};

pub fn unix_seconds() -> i64 {
    let mut now = esp_idf_sys::timeval::default();
    unsafe { esp_idf_sys::gettimeofday(&mut now, core::ptr::null_mut()) };
    now.tv_sec
}

pub fn set(seconds: i64) -> Result<(), &'static str> {
    if !clock_face::valid_time(seconds) {
        return Err("unix_seconds must be between 2020 and 2100");
    }
    let now = esp_idf_sys::timeval {
        tv_sec: seconds,
        tv_usec: 0,
    };
    if unsafe { esp_idf_sys::settimeofday(&now, core::ptr::null()) } != 0 {
        return Err("RTC time could not be set");
    }
    Ok(())
}

pub fn status() -> Value {
    let seconds = unix_seconds();
    json!({"unix_seconds":seconds, "time":ClockFace::at(seconds).text(),
        "synchronized":clock_face::valid_time(seconds),
        "utc_offset_minutes":clock_face::UTC_OFFSET_MINUTES})
}
