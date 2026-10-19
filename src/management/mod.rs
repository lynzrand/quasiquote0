//! Browser routes use ordinary HTTP and one exclusive mutation/transfer lease.
mod body;
mod uploads;

use crate::{activity::OPERATIONS, app::App, config::Config};
use body::Body;
use embedded_svc::http::{server::Request, Method};
use esp_idf_hal::io::Write as EmbeddedWrite;
use esp_idf_svc::{
    http::server::{EspHttpConnection, EspHttpServer},
    io::EspIOError,
};
use serde_json::{json, Value};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

type HttpRequest<'r, 'c> = Request<&'r mut EspHttpConnection<'c>>;
type HttpResult = Result<(), EspIOError>;
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
            .write_all(include_bytes!("../web/embedded.html"))
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
fn handle(request: HttpRequest<'_, '_>, app: &App, action: Action) -> HttpResult {
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
    // Configuration takes its lease inside App, after reading the bounded body.
    let _operation = if matches!(
        action,
        Action::Frame | Action::Firmware | Action::Reboot | Action::Pull
    ) {
        match OPERATIONS.try_acquire() {
            Some(operation) => Some(operation),
            None => {
                return reply(
                    request,
                    409,
                    json!({"error":"Another operation is in progress"}),
                )
            }
        }
    } else {
        None
    };
    match action {
        Action::Status => reply(request, 200, app.status(false)),
        Action::ConfigGet => reply(request, 200, json!({"config":app.config().redacted()})),
        Action::ConfigSet => save_config(request, app),
        Action::Frame => uploads::handle(request, app, uploads::Upload::Frame),
        Action::Firmware => uploads::handle(request, app, uploads::Upload::Firmware),
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
    }
}

fn save_config(mut request: HttpRequest<'_, '_>, app: &App) -> HttpResult {
    let length = match content_length(&request, MAX_CONFIG) {
        Ok(length) => length,
        Err(error) => return reply(request, 400, json!({"error":error})),
    };
    let mut bytes = vec![0; length];
    if let Err(error) =
        Body::new(&mut request, app, Duration::from_secs(10)).read_exact_into(&mut bytes)
    {
        return reply(request, 400, json!({"error":error.to_string()}));
    }
    let result = serde_json::from_slice::<Config>(&bytes)
        .map_err(|error| error.to_string())
        .and_then(|config| app.replace_config(config));
    match result {
        Ok(()) => reply(request, 200, json!({"accepted":true})),
        Err(error) => reply(request, 400, json!({"error":error})),
    }
}

fn reply(request: HttpRequest<'_, '_>, code: u16, value: Value) -> HttpResult {
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
fn content_length(request: &HttpRequest<'_, '_>, max: usize) -> Result<usize, &'static str> {
    request
        .header("Content-Length")
        .and_then(|n| n.parse().ok())
        .filter(|n| *n > 0 && *n <= max)
        .ok_or("Missing or invalid Content-Length")
}
