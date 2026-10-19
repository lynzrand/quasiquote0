//! Verify streamed uploads before durable publication or OTA selection.
use super::{body::Body, content_length, reply, HttpRequest, HttpResult};
use crate::{
    app::App,
    device::Device,
    frame::{self, Sha256Digest, FRAME_BYTES},
    ota,
};
use serde_json::json;
use std::{
    io::{Read, Write},
    sync::{atomic::Ordering, Mutex},
    time::Duration,
};

const OTA_SLOT_BYTES: usize = 0x1e0000;

#[derive(Clone, Copy)]
pub(super) enum Upload {
    Frame,
    Firmware,
}

pub(super) fn handle(mut request: HttpRequest<'_, '_>, app: &App, upload: Upload) -> HttpResult {
    let (maximum, timeout) = match upload {
        Upload::Frame => (FRAME_BYTES, 15),
        Upload::Firmware => (OTA_SLOT_BYTES, 120),
    };
    let length = match content_length(&request, maximum) {
        Ok(length) => length,
        Err(error) => return reply(request, 400, json!({"error":error})),
    };
    if matches!(upload, Upload::Frame) && length != FRAME_BYTES {
        return reply(
            request,
            400,
            json!({"error":"Frame must be exactly 5624 bytes"}),
        );
    }
    let announced = request.header("X-Content-SHA256").unwrap_or("");
    let digest = match frame::parse_digest(announced) {
        Ok(digest) if !announced.bytes().any(|byte| byte.is_ascii_uppercase()) => digest,
        _ => {
            return reply(
                request,
                400,
                json!({"error":"X-Content-SHA256 must be 64 lowercase hex digits"}),
            )
        }
    };
    let Some(device) = &app.device else {
        return reply(request, 503, json!({"error":"Storage recovery required"}));
    };
    let mut body = Body::new(&mut request, app, Duration::from_secs(timeout));
    let result = match upload {
        Upload::Frame => receive_frame(&mut body, app, device, digest),
        Upload::Firmware => receive_firmware(&mut body, app, device, length, digest),
    };
    match result {
        Ok(()) => {
            let value = match upload {
                Upload::Frame => json!({"accepted":true,"sha256":frame::digest_hex(&digest)}),
                Upload::Firmware => json!({"accepted":true,"rebooting":true}),
            };
            if matches!(upload, Upload::Firmware) {
                app.reboot_requested.store(true, Ordering::Release);
            }
            reply(request, 200, value)
        }
        Err(error) => reply(request, 400, json!({"error":error})),
    }
}

fn receive_frame(
    body: &mut impl Read,
    app: &App,
    device: &Mutex<Device>,
    digest: Sha256Digest,
) -> Result<(), String> {
    let mut writer = device
        .lock()
        .unwrap()
        .begin_frame_write(digest)
        .map_err(|error| error.to_string())?;
    let mut remaining = FRAME_BYTES;
    let mut buffer = [0u8; 512];
    while remaining > 0 {
        let count = body
            .read(&mut buffer[..remaining.min(512)])
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Err("Upload ended early".into());
        }
        writer
            .write_all(&buffer[..count])
            .map_err(|error| error.to_string())?;
        remaining -= count;
    }
    if !app.usb_powered() {
        return Err("USB power removed; upload discarded".into());
    }
    let digest = writer.finish().map_err(|error| error.to_string())?;
    device
        .lock()
        .unwrap()
        .commit_frame(digest)
        .map_err(|error| error.to_string())
}

fn receive_firmware(
    body: &mut impl Read,
    app: &App,
    device: &Mutex<Device>,
    length: usize,
    digest: Sha256Digest,
) -> Result<(), String> {
    ota::receive(body, length, digest)?;
    if !app.usb_powered() {
        return Err("USB power removed; update not selected".into());
    }
    device
        .lock()
        .unwrap()
        .mark_ota_pending()
        .map_err(|error| error.to_string())?;
    ota::select_staged_partition()
}
