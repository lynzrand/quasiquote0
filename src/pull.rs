//! Raw canonical-frame HTTP(S) pull client.

use crate::config::PullConfig;
use crate::store::PullValidators;
use core::time::Duration;
use embedded_svc::http::Method;
use esp_idf_svc::http::client::{Configuration, EspHttpConnection, FollowRedirectsPolicy};
use std::io::Write;

pub struct Response {
    pub status: u16,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub announced_sha256: Option<String>,
}

/// Stream a single response body to `output`, retaining no response payload.
pub fn fetch(
    config: &PullConfig,
    validators: &PullValidators,
    output: &mut impl Write,
) -> Result<Response, String> {
    let https = config.url.starts_with("https://");
    let client_config = Configuration {
        timeout: Some(Duration::from_secs(config.timeout_seconds)),
        crt_bundle_attach: if https {
            Some(esp_idf_sys::esp_crt_bundle_attach)
        } else {
            None
        },
        // Redirects would need an explicit scheme/host policy. Treat every
        // redirect as a rejected server response until that policy exists.
        follow_redirects_policy: FollowRedirectsPolicy::FollowNone,
        ..Default::default()
    };
    let mut connection =
        EspHttpConnection::new(&client_config).map_err(|error| error.to_string())?;
    let authorization =
        (!config.bearer_token.is_empty()).then(|| format!("Bearer {}", config.bearer_token));
    let mut headers = vec![
        (
            "Accept",
            "application/vnd.quasiquote0.frame, application/octet-stream",
        ),
        (
            "User-Agent",
            concat!("quasiquote0/", env!("CARGO_PKG_VERSION")),
        ),
    ];
    if let Some(etag) = validators.etag.as_deref() {
        headers.push(("If-None-Match", etag));
    }
    if let Some(last_modified) = validators.last_modified.as_deref() {
        headers.push(("If-Modified-Since", last_modified));
    }
    if let Some(authorization) = authorization.as_deref() {
        headers.push(("Authorization", authorization));
    }
    connection
        .initiate_request(Method::Get, &config.url, &headers)
        .map_err(|error| error.to_string())?;
    connection
        .initiate_response()
        .map_err(|error| error.to_string())?;
    let response = Response {
        status: connection.status(),
        etag: connection.header("ETag").map(str::to_owned),
        last_modified: connection.header("Last-Modified").map(str::to_owned),
        announced_sha256: connection.header("X-Frame-SHA256").map(str::to_owned),
    };
    if response.status != 200 {
        return Ok(response);
    }
    let mut buffer = [0u8; 512];
    let mut total = 0;
    loop {
        let count = connection
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        total += count;
        if total > crate::frame::FRAME_BYTES {
            return Err("HTTP frame body exceeds 5,624 bytes".to_owned());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| error.to_string())?;
    }
    if total != crate::frame::FRAME_BYTES {
        return Err("HTTP frame body is shorter than 5,624 bytes".to_owned());
    }
    Ok(response)
}
