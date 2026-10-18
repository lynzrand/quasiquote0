//! Physical USB provisioning and recovery, deliberately separate from web APIs.
use crate::{
    app::App,
    bootstrap_line::{Command, Lines},
    usb_serial::UsbSerial,
};
use serde_json::json;
use std::sync::Arc;
pub fn serve(mut usb: UsbSerial<'_>, app: Arc<App>) {
    let mut lines = Lines::default();
    let mut bytes = [0u8; 64];
    loop {
        let count = match usb.read(&mut bytes, 100) {
            Ok(n) => n,
            Err(_) => continue,
        };
        for byte in &bytes[..count] {
            let Some(command) = lines.push(*byte) else {
                continue;
            };
            let result = match command {
                Ok(Command::Status) => Ok(json!({"status":app.status(true)})),
                Ok(Command::Wifi { ssid, password }) => app
                    .set_wifi(ssid, password)
                    .map(|_| json!({"accepted":true})),
                Ok(Command::ClearWifi) => app
                    .set_wifi(String::new(), String::new())
                    .map(|_| json!({"accepted":true})),
                Err(error) => Err(error.to_owned()),
            };
            let response = match result {
                Ok(result) => json!({"ok":true,"result":result}),
                Err(error) => json!({"ok":false,"error":error}),
            };
            let mut line = serde_json::to_vec(&response).unwrap();
            line.push(b'\n');
            let _ = usb.write_all(&line);
        }
    }
}
