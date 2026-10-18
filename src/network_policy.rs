//! Radio mode is a consequence of USB power, provisioning, and pull deadlines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkMode {
    Off,
    Station,
    Setup,
}
pub fn desired(usb: bool, configured: bool, pull_due: bool) -> LinkMode {
    if usb {
        if configured {
            LinkMode::Station
        } else {
            LinkMode::Setup
        }
    } else if configured && pull_due {
        LinkMode::Station
    } else {
        LinkMode::Off
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usb_keeps_station_or_setup_available_without_enabling_battery_services() {
        assert_eq!(desired(true, true, false), LinkMode::Station);
        assert_eq!(desired(true, false, false), LinkMode::Setup);
        assert_eq!(desired(false, true, false), LinkMode::Off);
        assert_eq!(desired(false, false, true), LinkMode::Off);
        assert_eq!(desired(false, true, true), LinkMode::Station);
    }
}
