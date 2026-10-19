//! Authenticated portable export and validated, reboot-applied restore.
use super::{body::Body, content_length, reply, HttpRequest, HttpResult};
use crate::{
    app::App,
    backup::{self, Backup},
};
use esp_idf_hal::io::Write as EmbeddedWrite;
use serde_json::json;
use std::{io::Read, sync::atomic::Ordering, time::Duration};

pub(super) fn export(request: HttpRequest<'_, '_>, app: &App) -> HttpResult {
    let Some(device) = &app.device else {
        return reply(request, 503, json!({"error":"Storage recovery required"}));
    };
    let snapshot = (|| -> Result<_, String> {
        let device = device.lock().unwrap();
        let config = serde_json::to_vec(device.config()).map_err(|error| error.to_string())?;
        if config.len() > backup::MAX_CONFIG_BYTES {
            return Err("Backup settings exceed 4096 bytes".into());
        }
        let frame = device.backup_frame().map_err(|error| error.to_string())?;
        Ok((config, frame))
    })();
    let (config, frame) = match snapshot {
        Ok(snapshot) => snapshot,
        Err(error) => return reply(request, 400, json!({"error":error})),
    };
    let mut response = request.into_response(
        200,
        Some("OK"),
        &[
            ("Content-Type", "application/json"),
            (
                "Content-Disposition",
                "attachment; filename=quote0-backup.json",
            ),
            ("Cache-Control", "no-store"),
            ("X-Content-Type-Options", "nosniff"),
        ],
    )?;
    response.write_all(format!("{{\"schema\":{},\"config\":", backup::SCHEMA).as_bytes())?;
    response.write_all(&config)?;
    if let Some(mut frame) = frame {
        response.write_all(
            format!(
                ",\"frame\":{{\"sha256\":\"{}\",\"hex\":\"",
                crate::frame::digest_hex(&frame.digest)
            )
            .as_bytes(),
        )?;
        let mut bytes = [0; 512];
        let mut hex = [0; 1024];
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        loop {
            if !app.usb_powered() {
                return Ok(());
            }
            // The file was verified before sending headers. Convert a later IO
            // failure to a truncated JSON document, which import cannot accept.
            let count = match frame.file.read(&mut bytes) {
                Ok(0) => break,
                Err(_) => return Ok(()),
                Ok(count) => count,
            };
            for (index, byte) in bytes[..count].iter().enumerate() {
                hex[index * 2] = DIGITS[(byte >> 4) as usize];
                hex[index * 2 + 1] = DIGITS[(byte & 15) as usize];
            }
            response.write_all(&hex[..count * 2])?;
        }
        response.write_all(b"\"}}")?;
    } else {
        response.write_all(b",\"frame\":null}")?;
    }
    Ok(())
}

pub(super) fn restore(mut request: HttpRequest<'_, '_>, app: &App) -> HttpResult {
    let length = match content_length(&request, backup::MAX_BYTES) {
        Ok(length) => length,
        Err(error) => return reply(request, 400, json!({"error":error})),
    };
    let Some(device) = &app.device else {
        return reply(request, 503, json!({"error":"Storage recovery required"}));
    };
    let result = (|| -> Result<(), String> {
        // Hold the device lock while parsing to keep simultaneous panel frame
        // allocations out of this bounded JSON parser's heap budget.
        let device = device.lock().unwrap();
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| "Not enough memory to import backup")?;
        bytes.resize(length, 0);
        Body::new(&mut request, app, Duration::from_secs(15))
            .read_exact_into(&mut bytes)
            .map_err(|error| error.to_string())?;
        let backup: Backup =
            serde_json::from_slice(&bytes).map_err(|error| format!("Invalid backup: {error}"))?;
        drop(bytes);
        if !app.usb_powered() {
            return Err("USB power removed; restore discarded".into());
        }
        device
            .stage_restore(backup)
            .map_err(|error| error.to_string())
    })();
    match result {
        Ok(()) => {
            // The durable journal has committed even if the client disconnects
            // before receiving the reply. Always finish applying that restore.
            app.reboot_requested.store(true, Ordering::Release);
            reply(request, 200, json!({"accepted":true,"rebooting":true}))
        }
        Err(error) => reply(request, 400, json!({"error":error})),
    }
}
