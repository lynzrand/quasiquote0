//! Pull deadlines survive clock-only deep-sleep wakes without flash writes.

use crate::pull_schedule::PullSchedule;

// Declared by ESP-IDF's esp_hw_support/include/esp_rtc_time.h. Unlike
// esp_timer uptime, this counter survives deep sleep; setting UTC cannot move it.
extern "C" {
    fn esp_rtc_get_time_us() -> u64;
}

pub fn now_ms() -> u64 {
    unsafe { esp_rtc_get_time_us() / 1_000 }
}

// Integer-only storage is valid even before initialization. The linker keeps
// .rtc_noinit out of the normal RAM heap and retains it through deep sleep.
#[link_section = ".rtc_noinit"]
static mut SAVED: [u64; 4] = [0; 4];
const MAGIC: u64 = 0x5155_4f54_4553_0001;

pub fn load() -> PullSchedule {
    if unsafe { esp_idf_sys::esp_reset_reason() }
        != esp_idf_sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP
    {
        return PullSchedule::default();
    }
    // Only the scheduler task accesses this slot, before/after sleep.
    let [magic, due, failures, checksum] = unsafe { core::ptr::addr_of!(SAVED).read_volatile() };
    if magic != MAGIC || checksum != (magic ^ due ^ failures) || failures > u8::MAX as u64 {
        return PullSchedule::default();
    }
    PullSchedule::restore(due, failures as u8)
}

pub fn save(schedule: &PullSchedule) {
    let due = schedule.next_due_ms();
    let failures = u64::from(schedule.consecutive_failures());
    unsafe {
        core::ptr::addr_of_mut!(SAVED).write_volatile([
            MAGIC,
            due,
            failures,
            MAGIC ^ due ^ failures,
        ]);
    }
}
