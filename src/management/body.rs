//! Adapt an SDK request body to bounded, USB-aware streaming I/O.
use crate::app::App;
use esp_idf_hal::io::Read as EmbeddedRead;
use std::{
    io,
    time::{Duration, Instant},
};

pub(super) struct Body<'a, T> {
    request: T,
    app: &'a App,
    deadline: Instant,
}
impl<'a, T: EmbeddedRead> Body<'a, T> {
    pub fn new(request: T, app: &'a App, timeout: Duration) -> Self {
        Self {
            request,
            app,
            deadline: Instant::now() + timeout,
        }
    }
    pub fn read_exact_into(&mut self, bytes: &mut [u8]) -> io::Result<()> {
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
