//! Panel ownership, composition, and refresh acknowledgement.
use crate::{
    device::Device,
    display::Epd,
    frame::{digest_hex, parse_digest, Sha256Digest},
    refresh_policy::{RefreshKey, RefreshTracker},
    rtc_display, rtc_schedule,
    status_bar::StatusBar,
    wall_clock,
};
use std::sync::Mutex;

pub(super) struct Renderer<'d> {
    display: Option<Epd<'d>>,
    refreshes: RefreshTracker,
    last_composed: Option<Sha256Digest>,
}

impl<'d> Renderer<'d> {
    pub fn new(display: Option<Epd<'d>>, device: Option<&Mutex<Device>>) -> Self {
        let mut renderer = Self {
            display,
            refreshes: RefreshTracker::default(),
            last_composed: rtc_display::load(),
        };
        // A retained, unchanged image with its bar disabled needs no timer-wake
        // repaint. Partial refresh baselines remain strictly per boot.
        if crate::power::guard().timer_wake {
            if let Some(device) = device {
                let device = device.lock().unwrap();
                let state = device.state();
                if !device.config().display.status_bar
                    && state.displayed_status_bar == Some(false)
                    && state.firmware_version.as_deref() == Some(env!("CARGO_PKG_VERSION"))
                {
                    if let Ok(content) = device.desired_digest() {
                        if state.displayed_content_sha256.as_deref() == Some(&digest_hex(&content))
                            && renderer.last_composed.is_none()
                        {
                            renderer.last_composed = state
                                .displayed_composed_sha256
                                .as_deref()
                                .and_then(|digest| parse_digest(digest).ok());
                        }
                    }
                }
            }
        }
        renderer
    }

    /// Recompose from the producer's retained frame for content/layout changes or
    /// a new minute. A blank unconfigured device still gets the heartbeat strip.
    pub fn refresh(&mut self, device: &Mutex<Device>, status: StatusBar) {
        let Some(display) = self.display.as_mut() else {
            return;
        };
        let refreshes = &mut self.refreshes;
        let seconds = status.unix_seconds.unwrap_or_else(wall_clock::unix_seconds);
        let now_ms = rtc_schedule::now_ms();
        let (mut frame, key) = {
            let mut device = device.lock().unwrap();
            let content = match device.desired_digest() {
                Ok(content) => content,
                Err(error) => {
                    refreshes.failed(now_ms);
                    device.display_error = Some(error.to_string());
                    return;
                }
            };
            let key = RefreshKey::new(content, device.config().display.status_bar, seconds);
            if !refreshes.is_due(key, now_ms) {
                return;
            }
            let frame = match device.load_desired_frame() {
                Ok(frame) => frame.unwrap_or_default(),
                Err(error) => {
                    refreshes.failed(now_ms);
                    device.display_error = Some(error.to_string());
                    return;
                }
            };
            (frame, key)
        };
        if key.status_bar {
            status.overlay(&mut frame);
        }
        let composed = frame.sha256();
        let unchanged = self.last_composed == Some(composed);
        let started = rtc_schedule::now_ms();
        let mut mode = "full";
        let mut incremental = None;
        let mut partial_error = None;
        let transfer = if unchanged {
            mode = "unchanged";
            Ok(())
        } else {
            self.last_composed = None;
            rtc_display::save(None);
            let projected = frame.project();
            let status_only =
                cfg!(feature = "experimental-partial") && refreshes.can_refresh_status_only(key);
            if status_only {
                mode = if cfg!(feature = "experimental-fast") {
                    "fast-window"
                } else if cfg!(feature = "experimental-differential") {
                    "partial-differential"
                } else {
                    "partial-window"
                };
                incremental = Some(display.display_status_strip(&projected));
            }
            #[cfg(feature = "experimental-differential")]
            if !status_only && refreshes.can_refresh_differential() {
                mode = if cfg!(feature = "experimental-fast") {
                    "fast-differential"
                } else {
                    "differential"
                };
                incremental = Some(display.display_differential(&projected, false));
            }
            match incremental {
                Some(Ok(())) => Ok(()),
                Some(Err(error)) => {
                    partial_error = Some(error.to_owned());
                    mode = "full-fallback";
                    display.display(&projected)
                }
                None => display.display(&projected),
            }
        };
        let elapsed = if unchanged {
            0
        } else {
            rtc_schedule::now_ms().saturating_sub(started)
        };
        let result = transfer.map_err(str::to_owned).and_then(|()| {
            device
                .lock()
                .unwrap()
                .mark_displayed(
                    &key.content,
                    &composed,
                    env!("CARGO_PKG_VERSION"),
                    seconds,
                    key.status_bar,
                    !unchanged,
                )
                .map_err(|error| error.to_string())
        });
        match result {
            Ok(()) => {
                if unchanged {
                    refreshes.unchanged(key);
                } else if incremental.is_some() && partial_error.is_none() {
                    refreshes.partial_succeeded(key);
                } else {
                    refreshes.succeeded(key);
                }
                self.last_composed = Some(composed);
                rtc_display::save(Some(composed));
                let mut device = device.lock().unwrap();
                device.display_refresh_mode = mode;
                device.display_refresh_ms = elapsed;
                device.partial_refresh_error = partial_error;
            }
            Err(error) => {
                self.last_composed = None;
                rtc_display::save(None);
                refreshes.failed(rtc_schedule::now_ms());
                device.lock().unwrap().display_error = Some(error);
            }
        }
    }
}
