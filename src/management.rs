//! Browser routes use ordinary HTTP and one exclusive mutation/transfer lease.
use crate::{
    activity::OPERATIONS,
    app::App,
    config::Config,
    frame::{Sha256Digest, FRAME_BYTES},
    store::FrameMeta,
};
use embedded_svc::http::{server::Request, Method};
use esp_idf_hal::io::{Read as EmbeddedRead, Write as EmbeddedWrite};
use esp_idf_svc::{
    http::server::{EspHttpConnection, EspHttpServer},
    io::EspIOError,
};
use serde_json::{json, Value};
use std::{
    io,
    io::Write,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
const MAX_CONFIG: usize = 4096;

pub fn start(app: Arc<App>) -> Result<EspHttpServer<'static>, EspIOError> {
    let mut server = EspHttpServer::new(&esp_idf_svc::http::server::Configuration {
        stack_size: 12 * 1024,
        max_sessions: 4,
        max_open_sockets: 3,
        max_uri_handlers: 10,
        ..Default::default()
    })?;
    let page = app.clone();
    server.fn_handler("/", Method::Get, move |request| {
        if !page.usb_powered() {
            return reply(
                request,
                503,
                json!({"error":"Connect USB power to manage the device"}),
            );
        }
        let headers = [
            ("Content-Type", "text/html; charset=utf-8"),
            ("Cache-Control", "no-store"),
            ("X-Content-Type-Options", "nosniff"),
        ];
        request
            .into_response(200, Some("OK"), &headers)?
            .write_all(include_bytes!("web/index.html"))
    })?;
    for (path, method, action) in [
        ("/api/status", Method::Get, Action::Status),
        ("/api/config", Method::Get, Action::ConfigGet),
        ("/api/config", Method::Post, Action::ConfigSet),
        ("/api/frame", Method::Put, Action::Frame),
        ("/api/firmware", Method::Put, Action::Firmware),
        ("/api/reboot", Method::Post, Action::Reboot),
        ("/api/pull", Method::Post, Action::Pull),
    ] {
        let app = app.clone();
        server.fn_handler(path, method, move |request| handle(request, &app, action))?;
    }
    Ok(server)
}
#[derive(Clone, Copy)]
enum Action {
    Status,
    ConfigGet,
    ConfigSet,
    Frame,
    Firmware,
    Reboot,
    Pull,
}
fn handle(
    mut request: Request<&mut EspHttpConnection<'_>>,
    app: &Arc<App>,
    action: Action,
) -> Result<(), EspIOError> {
    if !app.usb_powered() {
        return reply(
            request,
            503,
            json!({"error":"Connect USB power to manage the device"}),
        );
    }
    if !app.authorized(request.header("X-Device-Token")) {
        return reply(
            request,
            401,
            json!({"error":"Enter the device access code"}),
        );
    }
    if matches!(action, Action::Status) {
        return reply(request, 200, app.status(false));
    }
    if matches!(action, Action::ConfigGet) {
        return reply(request, 200, json!({"config":app.config().redacted()}));
    }
    if matches!(action, Action::ConfigSet) {
        let length = match content_length(&request, MAX_CONFIG) {
            Ok(len) => len,
            Err(e) => return reply(request, 400, json!({"error":e})),
        };
        let mut bytes = vec![0; length];
        if let Err(error) =
            Body::new(&mut request, app, Duration::from_secs(10)).read_exact_into(&mut bytes)
        {
            return reply(request, 400, json!({"error":error.to_string()}));
        }
        let result = serde_json::from_slice::<Config>(&bytes)
            .map_err(|e| e.to_string())
            .and_then(|config| app.replace_config(config));
        return match result {
            Ok(()) => reply(request, 200, json!({"accepted":true})),
            Err(error) => reply(request, 400, json!({"error":error})),
        };
    }
    let Some(_operation) = OPERATIONS.try_acquire() else {
        return reply(
            request,
            409,
            json!({"error":"Another operation is in progress"}),
        );
    };
    match action {
        Action::Frame | Action::Firmware => {
            let max = if matches!(action, Action::Frame) {
                FRAME_BYTES
            } else {
                0x1e0000
            };
            let length = match content_length(&request, max) {
                Ok(n) => n,
                Err(e) => return reply(request, 400, json!({"error":e})),
            };
            if matches!(action, Action::Frame) && length != FRAME_BYTES {
                return reply(
                    request,
                    400,
                    json!({"error":"Frame must be exactly 5624 bytes"}),
                );
            }
            let digest = match parse_digest(request.header("X-Content-SHA256").unwrap_or("")) {
                Ok(d) => d,
                Err(e) => return reply(request, 400, json!({"error":e})),
            };
            let Some(device) = &app.device else {
                return reply(request, 503, json!({"error":"Storage recovery required"}));
            };
            let result = (|| -> Result<Value, String> {
                let mut body = Body::new(
                    &mut request,
                    app,
                    Duration::from_secs(if matches!(action, Action::Frame) {
                        15
                    } else {
                        120
                    }),
                );
                if matches!(action, Action::Frame) {
                    let mut writer = device
                        .lock()
                        .unwrap()
                        .begin_frame_write(digest)
                        .map_err(|e| e.to_string())?;
                    let mut remaining = length;
                    let mut buffer = [0u8; 512];
                    while remaining > 0 {
                        let count =
                            std::io::Read::read(&mut body, &mut buffer[..remaining.min(512)])
                                .map_err(|e| e.to_string())?;
                        if count == 0 {
                            return Err("Upload ended early".into());
                        }
                        writer
                            .write_all(&buffer[..count])
                            .map_err(|e| e.to_string())?;
                        remaining -= count;
                    }
                    if !app.usb_powered() {
                        return Err("USB power removed; upload discarded".into());
                    }
                    let digest = writer.finish().map_err(|e| e.to_string())?;
                    device
                        .lock()
                        .unwrap()
                        .commit_frame(
                            digest,
                            FrameMeta {
                                source: "browser".into(),
                                generation: String::new(),
                                accepted_at_ms: crate::rtc_schedule::now_ms(),
                            },
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(json!({"accepted":true,"sha256":crate::frame::digest_hex(&digest)}))
                } else {
                    crate::ota::receive(&mut body, length, digest)?;
                    if !app.usb_powered() {
                        return Err("USB power removed; update not selected".into());
                    }
                    device
                        .lock()
                        .unwrap()
                        .mark_ota_pending(None)
                        .map_err(|e| e.to_string())?;
                    crate::ota::select_staged_partition()?;
                    Ok(json!({"accepted":true,"rebooting":true}))
                }
            })();
            match result {
                Ok(value) => {
                    reply(request, 200, value)?;
                    if matches!(action, Action::Firmware) {
                        app.reboot_requested.store(true, Ordering::Release);
                    }
                    Ok(())
                }
                Err(error) => reply(request, 400, json!({"error":error})),
            }
        }
        Action::Reboot => {
            reply(request, 200, json!({"rebooting":true}))?;
            app.reboot_requested.store(true, Ordering::Release);
            Ok(())
        }
        Action::Pull => {
            if !app.config().pull.enabled {
                return reply(
                    request,
                    400,
                    json!({"error":"Enable a pull endpoint first"}),
                );
            }
            app.pull_requested.store(true, Ordering::Release);
            reply(request, 202, json!({"scheduled":true}))
        }
        _ => unreachable!(),
    }
}
fn reply(
    request: Request<&mut EspHttpConnection<'_>>,
    code: u16,
    value: Value,
) -> Result<(), EspIOError> {
    let bytes = serde_json::to_vec(&value).unwrap();
    request
        .into_response(
            code,
            None,
            &[
                ("Content-Type", "application/json"),
                ("Cache-Control", "no-store"),
                ("Connection", "close"),
            ],
        )?
        .write_all(&bytes)
}
fn content_length(
    request: &Request<&mut EspHttpConnection<'_>>,
    max: usize,
) -> Result<usize, &'static str> {
    request
        .header("Content-Length")
        .and_then(|n| n.parse().ok())
        .filter(|n| *n > 0 && *n <= max)
        .ok_or("Missing or invalid Content-Length")
}
fn parse_digest(value: &str) -> Result<Sha256Digest, &'static str> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("X-Content-SHA256 must be 64 lowercase hex digits");
    }
    let mut digest = [0u8; 32];
    for (i, slot) in digest.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).unwrap();
    }
    Ok(digest)
}
struct Body<'a, T> {
    request: T,
    app: &'a App,
    deadline: Instant,
}
impl<'a, T: EmbeddedRead> Body<'a, T> {
    fn new(request: T, app: &'a App, timeout: Duration) -> Self {
        Self {
            request,
            app,
            deadline: Instant::now() + timeout,
        }
    }
    fn read_exact_into(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        io::Read::read_exact(self, bytes)
    }
}
impl<T: EmbeddedRead> io::Read for Body<'_, T> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if !self.app.usb_powered() {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "USB power removed",
            ));
        }
        if Instant::now() > self.deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Upload deadline exceeded",
            ));
        }
        self.request
            .read(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::Other, format!("{error:?}")))
    }
}
