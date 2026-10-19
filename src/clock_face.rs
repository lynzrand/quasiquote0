//! Minute clock for the status bar. UTC is stored in the RTC; display is UTC+8.

use serde::Serialize;

pub const UTC_OFFSET_MINUTES: i64 = 8 * 60;
pub const MIN_UNIX_SECONDS: i64 = 1_577_836_800; // 2020-01-01 UTC
pub const MAX_UNIX_SECONDS: i64 = 4_102_444_800; // 2100-01-01 UTC

pub fn valid_time(seconds: i64) -> bool {
    (MIN_UNIX_SECONDS..MAX_UNIX_SECONDS).contains(&seconds)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ClockFace {
    WaitingForTime,
    Minute { hour: u8, minute: u8 },
}

impl ClockFace {
    pub fn at(unix_seconds: i64) -> Self {
        if !valid_time(unix_seconds) {
            return Self::WaitingForTime;
        }
        let local = (unix_seconds + UTC_OFFSET_MINUTES * 60).rem_euclid(24 * 60 * 60);
        Self::Minute {
            hour: (local / 3600) as u8,
            minute: ((local / 60) % 60) as u8,
        }
    }

    pub fn text(self) -> String {
        match self {
            Self::WaitingForTime => "--:--".to_owned(),
            Self::Minute { hour, minute } => format!("{hour:02}:{minute:02}"),
        }
    }
}

pub fn seconds_until_next_minute(unix_seconds: i64) -> u64 {
    (60 - unix_seconds.rem_euclid(60)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hong_kong_midnight_rollover_and_minute_boundaries() {
        let midnight_utc = MIN_UNIX_SECONDS;
        assert_eq!(ClockFace::at(midnight_utc).text(), "08:00");
        assert_eq!(ClockFace::at(midnight_utc + 16 * 3600 - 1).text(), "23:59");
        assert_eq!(ClockFace::at(midnight_utc + 16 * 3600).text(), "00:00");
        assert_eq!(seconds_until_next_minute(midnight_utc), 60);
        assert_eq!(seconds_until_next_minute(midnight_utc + 59), 1);
    }

    #[test]
    fn seconds_do_not_change_the_display_face() {
        assert_eq!(
            ClockFace::at(MIN_UNIX_SECONDS),
            ClockFace::at(MIN_UNIX_SECONDS + 59)
        );
        assert_ne!(
            ClockFace::at(MIN_UNIX_SECONDS + 59),
            ClockFace::at(MIN_UNIX_SECONDS + 60)
        );
        assert_eq!(ClockFace::at(0), ClockFace::WaitingForTime);
    }
}
