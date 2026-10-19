//! Versioned device configuration and validation.

use core::fmt;
use serde::{Deserialize, Serialize};

pub const CONFIG_SCHEMA: u32 = 1;
pub const DEFAULT_PULL_INTERVAL_SECONDS: u64 = 15 * 60;
pub const DEFAULT_PULL_TIMEOUT_SECONDS: u64 = 15;
pub const MIN_BATTERY_PULL_INTERVAL_SECONDS: u64 = 60;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(from = "ConfigInput")]
pub struct Config {
    pub schema: u32,
    pub wifi: WifiConfig,
    pub pull: PullConfig,
    pub display: DisplayConfig,
}

// Accept the retired power object when upgrading a saved v1 configuration.
// It never affects behavior and is omitted when writing the current format.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigInput {
    #[serde(default = "default_schema")]
    schema: u32,
    #[serde(default)]
    wifi: WifiConfig,
    #[serde(default)]
    pull: PullConfig,
    #[serde(default)]
    display: DisplayConfig,
    #[serde(default, rename = "power")]
    _legacy_power: Option<serde::de::IgnoredAny>,
}

impl From<ConfigInput> for Config {
    fn from(input: ConfigInput) -> Self {
        Self {
            schema: input.schema,
            wifi: input.wifi,
            pull: input.pull,
            display: input.display,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WifiConfig {
    #[serde(default)]
    pub ssid: String,
    #[serde(default)]
    pub password: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PullConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default = "default_pull_interval")]
    pub interval_seconds: u64,
    #[serde(default = "default_pull_timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub bearer_token: String,
    #[serde(default)]
    pub allow_insecure_http: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DisplayConfig {
    #[serde(default = "default_status_bar")]
    pub status_bar: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigError(pub &'static str);

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for ConfigError {}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema: CONFIG_SCHEMA,
            wifi: WifiConfig::default(),
            pull: PullConfig::default(),
            display: DisplayConfig::default(),
        }
    }
}

impl Default for PullConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            interval_seconds: default_pull_interval(),
            timeout_seconds: default_pull_timeout(),
            bearer_token: String::new(),
            allow_insecure_http: false,
        }
    }
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            status_bar: default_status_bar(),
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema != CONFIG_SCHEMA {
            return Err(ConfigError("unsupported config schema"));
        }
        if self.wifi.ssid.as_bytes().len() > 32 {
            return Err(ConfigError("wifi.ssid exceeds 32 bytes"));
        }
        if self.wifi.password.as_bytes().len() > 64 {
            return Err(ConfigError("wifi.password exceeds 64 bytes"));
        }
        if self.pull.timeout_seconds == 0 {
            return Err(ConfigError("pull.timeout_seconds must be non-zero"));
        }
        if self.pull.enabled {
            if self.wifi.ssid.is_empty() {
                return Err(ConfigError("wifi.ssid is required when pull is enabled"));
            }
            if self.pull.url.is_empty() {
                return Err(ConfigError("pull.url is required when pull is enabled"));
            }
            if !self.pull.allow_insecure_http && !self.pull.url.starts_with("https://") {
                return Err(ConfigError("pull.url must use https"));
            }
            if self.pull.allow_insecure_http
                && !(self.pull.url.starts_with("https://") || self.pull.url.starts_with("http://"))
            {
                return Err(ConfigError("pull.url must use http or https"));
            }
            if self.pull.interval_seconds < MIN_BATTERY_PULL_INTERVAL_SECONDS {
                return Err(ConfigError("pull.interval_seconds is too short"));
            }
        }
        Ok(())
    }

    /// Keep redacted credentials when a management client edits other fields.
    pub fn preserve_secrets(&mut self, previous: &Self) {
        if self.wifi.password == "***" {
            self.wifi.password = previous.wifi.password.clone();
        }
        if self.pull.bearer_token == "***" {
            self.pull.bearer_token = previous.pull.bearer_token.clone();
        }
    }

    /// Return a configuration that is safe to expose over management APIs.
    pub fn redacted(&self) -> Self {
        let mut output = self.clone();
        output.wifi.password = redact(&output.wifi.password);
        output.pull.bearer_token = redact(&output.pull.bearer_token);
        output
    }
}

const fn default_schema() -> u32 {
    CONFIG_SCHEMA
}
const fn default_pull_interval() -> u64 {
    DEFAULT_PULL_INTERVAL_SECONDS
}
const fn default_pull_timeout() -> u64 {
    DEFAULT_PULL_TIMEOUT_SECONDS
}
const fn default_status_bar() -> bool {
    true
}

fn redact(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        "***".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_but_do_not_enable_networking() {
        let config = Config::default();
        assert!(!config.pull.enabled);
        assert!(config.display.status_bar);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn enabled_pull_requires_https_by_default() {
        let mut config = Config::default();
        config.wifi.ssid = "test".to_owned();
        config.pull.enabled = true;
        config.pull.url = "http://example.test/frame".to_owned();
        assert_eq!(
            config.validate(),
            Err(ConfigError("pull.url must use https"))
        );
    }

    #[test]
    fn old_power_setting_is_ignored_without_losing_credentials() {
        let config: Config = serde_json::from_str(
            r#"{
            "wifi":{"ssid":"saved-network","password":"saved-secret"},
            "power":{"stay_awake_on_usb":false}
        }"#,
        )
        .unwrap();
        assert_eq!(config.wifi.ssid, "saved-network");
        assert_eq!(config.wifi.password, "saved-secret");
        assert!(config.validate().is_ok());
        let current = serde_json::to_value(&config).unwrap();
        assert!(current.get("power").is_none());
        assert_eq!(serde_json::from_value::<Config>(current).unwrap(), config);
        assert_eq!(
            serde_json::from_str::<Config>(r#"{"power":{}}"#).unwrap(),
            Config::default()
        );
        assert!(serde_json::from_str::<Config>(r#"{"pulll":{}}"#).is_err());
    }

    #[test]
    fn redaction_does_not_expose_secrets() {
        let mut config = Config::default();
        config.wifi.password = "password".to_owned();
        config.pull.bearer_token = "token".to_owned();
        let redacted = config.redacted();
        assert_eq!(redacted.wifi.password, "***");
        assert_eq!(redacted.pull.bearer_token, "***");
    }
}
