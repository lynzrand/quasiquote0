//! Blocking Yamux streams driven by a dedicated transport thread.
//!
//! Standard Yamux reserves stream 0 for the session. Quote/0 intentionally
//! extends the protocol by exposing stream 0 as its fixed JSON-RPC stream.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

const VERSION: u8 = 0;
const DATA: u8 = 0;
const WINDOW_UPDATE: u8 = 1;
const PING: u8 = 2;
const GO_AWAY: u8 = 3;
const SYN: u16 = 0x1;
const ACK: u16 = 0x2;
const FIN: u16 = 0x4;
const RST: u16 = 0x8;
const HEADER_LEN: usize = 12;
/// Standard Yamux peers assume this credit when a stream is opened with data.
/// Quote/0 normally opens streams with a `WINDOW_UPDATE | SYN` carrying its
/// real bounded receive capacity instead.
const STANDARD_INITIAL_WINDOW: u32 = 256 * 1024;
const MAX_DATA_FRAME: usize = 16 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Maximum payload bytes retained in RAM for each stream.
    pub receive_buffer: usize,
    /// Maximum payload bytes queued for the USB writer across all streams.
    pub outbound_buffer: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            receive_buffer: 4 * 1024,
            outbound_buffer: 4 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Client,
    Server,
}

/// A running Yamux connection. Cloning it does not clone the transport.
#[derive(Clone)]
pub struct Yamux {
    shared: Arc<Shared>,
}

struct Shared {
    connection: Mutex<ConnectionState>,
    inbound_ready: Condvar,
    outbound_space: Condvar,
}

struct ConnectionState {
    streams: HashMap<u32, Arc<StreamShared>>,
    inbound: VecDeque<u32>,
    outbound: VecDeque<Outbound>,
    outbound_bytes: usize,
    next_stream_id: u32,
    mode: Mode,
    config: Config,
    closed: bool,
}

struct Outbound {
    header: Header,
    payload: Vec<u8>,
}

struct StreamShared {
    state: Mutex<StreamState>,
    readable: Condvar,
    writable: Condvar,
    receive_space: Condvar,
}

struct StreamState {
    received: VecDeque<u8>,
    send_window: u32,
    receive_closed: bool,
    send_closed: bool,
    reset: bool,
    receive_capacity: usize,
}

impl StreamShared {
    fn control(receive_capacity: usize) -> Self {
        Self {
            state: Mutex::new(StreamState {
                received: VecDeque::new(),
                send_window: u32::MAX,
                receive_closed: false,
                send_closed: false,
                reset: false,
                receive_capacity,
            }),
            readable: Condvar::new(),
            writable: Condvar::new(),
            receive_space: Condvar::new(),
        }
    }

    /// `send_window` is credit advertised by the peer. Locally initiated
    /// streams begin with no credit and wait for the peer's ACK; inbound
    /// streams receive the opening stream's advertised credit.
    fn data(receive_capacity: usize, send_window: u32) -> Self {
        Self {
            state: Mutex::new(StreamState {
                received: VecDeque::new(),
                send_window,
                receive_closed: false,
                send_closed: false,
                reset: false,
                receive_capacity,
            }),
            readable: Condvar::new(),
            writable: Condvar::new(),
            receive_space: Condvar::new(),
        }
    }
}

impl Yamux {
    /// Start the connection driver on a FreeRTOS-backed Rust thread.
    pub fn start<T>(transport: T, mode: Mode) -> io::Result<Self>
    where
        T: Read + Write + Send + 'static,
    {
        Self::start_with_config(transport, mode, Config::default())
    }

    pub fn start_with_config<T>(transport: T, mode: Mode, config: Config) -> io::Result<Self>
    where
        T: Read + Write + Send + 'static,
    {
        if config.receive_buffer == 0 || config.outbound_buffer == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Yamux buffers must be non-zero",
            ));
        }
        let control = Arc::new(StreamShared::control(config.receive_buffer));
        let shared = Arc::new(Shared {
            connection: Mutex::new(ConnectionState {
                streams: HashMap::from([(0, control)]),
                inbound: VecDeque::new(),
                outbound: VecDeque::new(),
                outbound_bytes: 0,
                next_stream_id: match mode {
                    Mode::Client => 1,
                    Mode::Server => 2,
                },
                mode,
                config,
                closed: false,
            }),
            inbound_ready: Condvar::new(),
            outbound_space: Condvar::new(),
        });
        let driver_shared = shared.clone();
        thread::Builder::new()
            .name("yamux-usb".into())
            .stack_size(12 * 1024)
            .spawn(move || drive(transport, driver_shared))
            .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))?;
        Ok(Self { shared })
    }

    /// Quote/0's reserved NDJSON JSON-RPC stream.
    pub fn control_stream(&self) -> YamuxStream {
        self.stream(0).expect("control stream is always present")
    }

    /// Open a standard outbound stream (odd IDs for clients, even for servers).
    pub fn open(&self) -> io::Result<YamuxStream> {
        let (id, stream) = {
            let mut connection = self.shared.connection.lock().unwrap();
            if connection.closed {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "Yamux closed"));
            }
            let id = connection.next_stream_id;
            connection.next_stream_id = id
                .checked_add(2)
                .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "stream IDs exhausted"))?;
            let receive_capacity = connection.config.receive_buffer;
            let stream = Arc::new(StreamShared::data(receive_capacity, 0));
            connection.streams.insert(id, stream.clone());
            connection.outbound.push_back(Outbound {
                // This opening credit is intentionally the actual per-stream
                // bounded queue, not Yamux's protocol-level 256 KiB default.
                header: Header::new(WINDOW_UPDATE, SYN, id, receive_capacity as u32),
                payload: Vec::new(),
            });
            (id, stream)
        };
        Ok(YamuxStream {
            id,
            connection: self.shared.clone(),
            stream,
        })
    }

    /// Park until the peer opens another standard stream.
    pub fn accept(&self) -> io::Result<YamuxStream> {
        let id = {
            let mut connection = self.shared.connection.lock().unwrap();
            loop {
                if let Some(id) = connection.inbound.pop_front() {
                    break id;
                }
                if connection.closed {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "Yamux closed"));
                }
                connection = self.shared.inbound_ready.wait(connection).unwrap();
            }
        };
        self.stream(id)
    }

    fn stream(&self, id: u32) -> io::Result<YamuxStream> {
        let stream = self
            .shared
            .connection
            .lock()
            .unwrap()
            .streams
            .get(&id)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown Yamux stream"))?;
        Ok(YamuxStream {
            id,
            connection: self.shared.clone(),
            stream,
        })
    }
}

/// Independently owned blocking stream handle.
#[derive(Clone)]
pub struct YamuxStream {
    id: u32,
    connection: Arc<Shared>,
    stream: Arc<StreamShared>,
}

impl Read for YamuxStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let mut state = self.stream.state.lock().unwrap();
        loop {
            if !state.received.is_empty() {
                let count = buffer.len().min(state.received.len());
                for slot in &mut buffer[..count] {
                    *slot = state.received.pop_front().unwrap();
                }
                self.stream.receive_space.notify_all();
                drop(state);
                if self.id != 0 {
                    queue(
                        &self.connection,
                        Header::new(WINDOW_UPDATE, 0, self.id, count as u32),
                        Vec::new(),
                    );
                }
                return Ok(count);
            }
            if state.reset {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "Yamux stream reset",
                ));
            }
            if state.receive_closed {
                return Ok(0);
            }
            state = self.stream.readable.wait(state).unwrap();
        }
    }
}

impl Write for YamuxStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let max_payload = self
            .connection
            .connection
            .lock()
            .unwrap()
            .config
            .outbound_buffer
            .min(MAX_DATA_FRAME);
        let mut state = self.stream.state.lock().unwrap();
        loop {
            if state.reset || state.send_closed {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Yamux stream closed",
                ));
            }
            let available = if self.id == 0 {
                max_payload
            } else {
                state.send_window.min(max_payload as u32) as usize
            };
            if available != 0 {
                let count = bytes.len().min(available);
                if self.id != 0 {
                    state.send_window -= count as u32;
                }
                drop(state);
                queue(
                    &self.connection,
                    Header::new(DATA, 0, self.id, count as u32),
                    bytes[..count].to_vec(),
                );
                return Ok(count);
            }
            state = self.stream.writable.wait(state).unwrap();
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl YamuxStream {
    /// Yamux stream ID selected by the local endpoint for `Yamux::open`.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Stop both halves of this application stream. The peer receives FIN;
    /// later payload is drained by the transport but never buffered.
    pub fn close(&self) {
        let mut state = self.stream.state.lock().unwrap();
        if !state.send_closed {
            state.send_closed = true;
            state.receive_closed = true;
            drop(state);
            queue(
                &self.connection,
                Header::new(WINDOW_UPDATE, FIN, self.id, 0),
                Vec::new(),
            );
        }
    }
}

fn queue(shared: &Arc<Shared>, header: Header, payload: Vec<u8>) {
    let payload_len = payload.len();
    let mut connection = shared.connection.lock().unwrap();
    while payload_len != 0
        && connection.outbound_bytes + payload_len > connection.config.outbound_buffer
        && !connection.closed
    {
        connection = shared.outbound_space.wait(connection).unwrap();
    }
    if connection.closed {
        return;
    }
    connection.outbound_bytes += payload_len;
    connection.outbound.push_back(Outbound { header, payload });
}

#[derive(Clone, Copy)]
struct Header {
    kind: u8,
    flags: u16,
    stream_id: u32,
    length: u32,
}

impl Header {
    fn new(kind: u8, flags: u16, stream_id: u32, length: u32) -> Self {
        Self {
            kind,
            flags,
            stream_id,
            length,
        }
    }

    fn decode(bytes: &[u8; HEADER_LEN]) -> io::Result<Self> {
        if bytes[0] != VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported Yamux version",
            ));
        }
        Ok(Self {
            kind: bytes[1],
            flags: u16::from_be_bytes([bytes[2], bytes[3]]),
            stream_id: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            length: u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        })
    }

    fn encode(self) -> [u8; HEADER_LEN] {
        let mut bytes = [0; HEADER_LEN];
        bytes[0] = VERSION;
        bytes[1] = self.kind;
        bytes[2..4].copy_from_slice(&self.flags.to_be_bytes());
        bytes[4..8].copy_from_slice(&self.stream_id.to_be_bytes());
        bytes[8..12].copy_from_slice(&self.length.to_be_bytes());
        bytes
    }
}

struct PendingPayload {
    header: Header,
    remaining: u32,
    stream: Option<Arc<StreamShared>>,
}

fn drive<T: Read + Write>(mut transport: T, shared: Arc<Shared>) {
    let result = drive_inner(&mut transport, &shared);
    close_all(&shared, result.err());
}

fn drive_inner<T: Read + Write>(transport: &mut T, shared: &Arc<Shared>) -> io::Result<()> {
    let mut header_bytes = [0; HEADER_LEN];
    let mut header_used = 0;
    let mut pending: Option<PendingPayload> = None;
    let mut chunk = [0; 512];
    loop {
        while let Some(outbound) = {
            let mut connection = shared.connection.lock().unwrap();
            let outbound = connection.outbound.pop_front();
            if let Some(outbound) = &outbound {
                connection.outbound_bytes -= outbound.payload.len();
                shared.outbound_space.notify_all();
            }
            outbound
        } {
            transport.write_all(&outbound.header.encode())?;
            transport.write_all(&outbound.payload)?;
        }

        if let Some(payload) = pending.as_mut() {
            let count = if let Some(stream) = &payload.stream {
                let mut state = stream.state.lock().unwrap();
                while state.received.len() == state.receive_capacity
                    && !state.reset
                    && !state.receive_closed
                {
                    state = stream.receive_space.wait(state).unwrap();
                }
                if state.reset || state.receive_closed {
                    chunk.len().min(payload.remaining as usize)
                } else {
                    chunk
                        .len()
                        .min(payload.remaining as usize)
                        .min(state.receive_capacity - state.received.len())
                }
            } else {
                chunk.len().min(payload.remaining as usize)
            };
            match transport.read(&mut chunk[..count]) {
                Ok(0) => continue,
                Ok(read) => {
                    if let Some(stream) = &payload.stream {
                        let mut state = stream.state.lock().unwrap();
                        if !state.reset {
                            state.received.extend(&chunk[..read]);
                            stream.readable.notify_all();
                        }
                    }
                    payload.remaining -= read as u32;
                    if payload.remaining == 0 {
                        if payload.header.flags & FIN != 0 {
                            if let Some(stream) = &payload.stream {
                                stream.state.lock().unwrap().receive_closed = true;
                                stream.readable.notify_all();
                            }
                        }
                        pending = None;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                Err(error) => return Err(error),
            }
            continue;
        }

        match transport.read(&mut header_bytes[header_used..]) {
            Ok(0) => continue,
            Ok(read) => header_used += read,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
            Err(error) => return Err(error),
        }
        if header_used != HEADER_LEN {
            continue;
        }
        header_used = 0;
        let header = Header::decode(&header_bytes)?;
        match header.kind {
            DATA => {
                let stream = resolve_data_stream(shared, header)?;
                if header.length == 0 {
                    finish_flags(&stream, header.flags);
                } else {
                    pending = Some(PendingPayload {
                        header,
                        remaining: header.length,
                        stream,
                    });
                }
            }
            WINDOW_UPDATE => handle_window_update(shared, header)?,
            PING if header.stream_id == 0 && header.flags & ACK == 0 => {
                queue(shared, Header::new(PING, ACK, 0, header.length), Vec::new());
            }
            PING => {}
            GO_AWAY if header.stream_id == 0 => return Ok(()),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid Yamux header",
                ))
            }
        }
    }
}

fn resolve_data_stream(
    shared: &Arc<Shared>,
    header: Header,
) -> io::Result<Option<Arc<StreamShared>>> {
    if header.stream_id == 0 {
        return Ok(shared.connection.lock().unwrap().streams.get(&0).cloned());
    }
    if header.flags & SYN != 0 {
        return create_inbound(shared, header.stream_id, STANDARD_INITIAL_WINDOW);
    }
    let stream = shared
        .connection
        .lock()
        .unwrap()
        .streams
        .get(&header.stream_id)
        .cloned();
    if stream.is_none() {
        queue(
            shared,
            Header::new(WINDOW_UPDATE, RST, header.stream_id, 0),
            Vec::new(),
        );
    }
    Ok(stream)
}

fn create_inbound(
    shared: &Arc<Shared>,
    id: u32,
    opening_credit: u32,
) -> io::Result<Option<Arc<StreamShared>>> {
    let mut connection = shared.connection.lock().unwrap();
    let valid = match connection.mode {
        Mode::Client => id != 0 && id % 2 == 0,
        Mode::Server => id % 2 == 1,
    };
    if !valid || connection.streams.contains_key(&id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Yamux stream ID",
        ));
    }
    let receive_capacity = connection.config.receive_buffer;
    let stream = Arc::new(StreamShared::data(receive_capacity, opening_credit));
    connection.streams.insert(id, stream.clone());
    connection.inbound.push_back(id);
    connection.outbound.push_back(Outbound {
        header: Header::new(WINDOW_UPDATE, ACK, id, receive_capacity as u32),
        payload: Vec::new(),
    });
    shared.inbound_ready.notify_all();
    Ok(Some(stream))
}

fn handle_window_update(shared: &Arc<Shared>, header: Header) -> io::Result<()> {
    let (stream, opened) = if header.flags & SYN != 0 {
        (
            create_inbound(shared, header.stream_id, header.length)?,
            true,
        )
    } else {
        (
            shared
                .connection
                .lock()
                .unwrap()
                .streams
                .get(&header.stream_id)
                .cloned(),
            false,
        )
    };
    if let Some(stream) = stream {
        let mut state = stream.state.lock().unwrap();
        if header.flags & RST != 0 {
            state.reset = true;
        } else if !opened {
            state.send_window = state.send_window.saturating_add(header.length);
        }
        if header.flags & FIN != 0 {
            state.receive_closed = true;
        }
        stream.readable.notify_all();
        stream.writable.notify_all();
        stream.receive_space.notify_all();
    }
    Ok(())
}

fn finish_flags(stream: &Option<Arc<StreamShared>>, flags: u16) {
    if let Some(stream) = stream {
        let mut state = stream.state.lock().unwrap();
        state.receive_closed |= flags & FIN != 0;
        state.reset |= flags & RST != 0;
        stream.readable.notify_all();
        stream.writable.notify_all();
        stream.receive_space.notify_all();
    }
}

fn close_all(shared: &Arc<Shared>, _error: Option<io::Error>) {
    let streams = {
        let mut connection = shared.connection.lock().unwrap();
        connection.closed = true;
        shared.inbound_ready.notify_all();
        shared.outbound_space.notify_all();
        connection.streams.values().cloned().collect::<Vec<_>>()
    };
    for stream in streams {
        let mut state = stream.state.lock().unwrap();
        state.reset = true;
        stream.readable.notify_all();
        stream.writable.notify_all();
        stream.receive_space.notify_all();
    }
}
