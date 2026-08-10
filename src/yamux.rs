//! Blocking, resumable Yamux streams over any `Read + Write` transport.
//!
//! Quote/0 reserves stream 0 for its newline-delimited JSON-RPC control
//! protocol. Payloads are copied directly between the transport and a stream
//! caller's buffer; the session retains only a partial 12-byte header and a
//! remaining-byte count, so it can pause at every transport read boundary.

use std::io::{self, Read, Write};

const VERSION: u8 = 0;
const TYPE_DATA: u8 = 0;
const TYPE_WINDOW_UPDATE: u8 = 1;
const TYPE_PING: u8 = 2;
const FLAG_ACK: u16 = 0x2;
const HEADER_LEN: usize = 12;
const MAX_FRAME_BYTES: u32 = 16 * 1024;

/// A synchronous Yamux session. Streams borrow the session, so callers can
/// choose the blocking/threading policy appropriate for their FreeRTOS tasks.
pub struct Yamux<T> {
    transport: T,
    header: [u8; HEADER_LEN],
    header_used: usize,
    current: Option<Frame>,
}

#[derive(Clone, Copy)]
struct Frame {
    kind: u8,
    flags: u16,
    stream_id: u32,
    remaining: u32,
}

impl<T: Read + Write> Yamux<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            header: [0; HEADER_LEN],
            header_used: 0,
            current: None,
        }
    }

    /// Return a blocking stream handle. Stream 0 is the Quote/0 NDJSON RPC
    /// stream; other IDs are available to future protocol users.
    pub fn stream(&mut self, id: u32) -> YamuxStream<'_, T> {
        YamuxStream { session: self, id }
    }

    fn read_for(&mut self, id: u32, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            if let Some(frame) = self.current {
                if frame.remaining == 0 {
                    self.current = None;
                    self.write_header(TYPE_WINDOW_UPDATE, 0, frame.stream_id, 0)?;
                    continue;
                }
                if frame.kind != TYPE_DATA || frame.stream_id != id {
                    self.discard_frame(frame)?;
                    continue;
                }
                let count = buffer.len().min(frame.remaining as usize);
                let read = self.transport.read(&mut buffer[..count])?;
                if read == 0 {
                    return Ok(0);
                }
                let mut updated = frame;
                updated.remaining -= read as u32;
                self.current = Some(updated);
                return Ok(read);
            }
            self.read_next_header()?;
        }
    }

    fn discard_frame(&mut self, frame: Frame) -> io::Result<()> {
        let mut discard = [0; 64];
        let count = discard.len().min(frame.remaining as usize);
        let read = self.transport.read(&mut discard[..count])?;
        if read == 0 {
            return Ok(());
        }
        let mut updated = frame;
        updated.remaining -= read as u32;
        self.current = Some(updated);
        Ok(())
    }

    fn read_next_header(&mut self) -> io::Result<()> {
        while self.header_used < HEADER_LEN {
            let read = self.transport.read(&mut self.header[self.header_used..])?;
            if read == 0 {
                return Ok(());
            }
            self.header_used += read;
        }
        self.header_used = 0;
        if self.header[0] != VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported Yamux version",
            ));
        }
        let frame = Frame {
            kind: self.header[1],
            flags: u16::from_be_bytes([self.header[2], self.header[3]]),
            stream_id: u32::from_be_bytes([
                self.header[4],
                self.header[5],
                self.header[6],
                self.header[7],
            ]),
            remaining: u32::from_be_bytes([
                self.header[8],
                self.header[9],
                self.header[10],
                self.header[11],
            ]),
        };
        if frame.kind == TYPE_DATA && frame.remaining > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Yamux frame too large",
            ));
        }
        if frame.kind == TYPE_PING && frame.flags & FLAG_ACK == 0 {
            self.write_header(TYPE_PING, FLAG_ACK, 0, frame.remaining)?;
            return Ok(());
        }
        if frame.kind == TYPE_DATA && frame.remaining == 0 {
            self.write_header(TYPE_WINDOW_UPDATE, 0, frame.stream_id, 0)?;
            return Ok(());
        }
        if frame.kind == TYPE_DATA {
            self.current = Some(frame);
        }
        Ok(())
    }

    fn write_for(&mut self, id: u32, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_FRAME_BYTES as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Yamux write too large",
            ));
        }
        self.write_header(TYPE_DATA, 0, id, bytes.len() as u32)?;
        self.transport.write_all(bytes)?;
        Ok(bytes.len())
    }

    fn write_header(
        &mut self,
        kind: u8,
        flags: u16,
        stream_id: u32,
        length: u32,
    ) -> io::Result<()> {
        let mut header = [0; HEADER_LEN];
        header[0] = VERSION;
        header[1] = kind;
        header[2..4].copy_from_slice(&flags.to_be_bytes());
        header[4..8].copy_from_slice(&stream_id.to_be_bytes());
        header[8..12].copy_from_slice(&length.to_be_bytes());
        self.transport.write_all(&header)
    }
}

/// A stream-backed standard I/O handle.
pub struct YamuxStream<'a, T> {
    session: &'a mut Yamux<T>,
    id: u32,
}

impl<T: Read + Write> Read for YamuxStream<'_, T> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.session.read_for(self.id, buffer)
    }
}

impl<T: Read + Write> Write for YamuxStream<'_, T> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.session.write_for(self.id, bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.session.transport.flush()
    }
}
