//! Wire protocol v1 codec (see PLAN.md). All integers are big-endian.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

/// Default TCP + UDP port.
pub const DEFAULT_PORT: u16 = 48777;
/// UDP discovery request payload.
pub const DISCOVER_MSG: &[u8] = b"VPMIC_DISCOVER_V1";
/// Service name in the discovery reply.
pub const SERVICE_NAME: &str = "vibe-phone-mic";
pub const PROTOCOL_VERSION: u8 = 1;
/// Max frame payload size (1 MiB).
pub const MAX_PAYLOAD: u32 = 1 << 20;

pub const FRAME_HELLO: u8 = 0x01;
pub const FRAME_AUDIO: u8 = 0x02;
pub const FRAME_STATUS: u8 = 0x03;
pub const FRAME_BYE: u8 = 0x04;

/// Valid HELLO ranges.
pub const MIN_RATE: u32 = 8000;
pub const MAX_RATE: u32 = 192_000;
pub const MAX_CHANNELS: u16 = 8;

/// Handshake mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Stream = 0,
    Probe = 1,
}

/// Build the 8-byte client handshake.
pub fn encode_handshake(mode: Mode) -> [u8; 8] {
    [b'V', b'P', b'M', b'C', PROTOCOL_VERSION, mode as u8, 0, 0]
}

/// Parse a client handshake (used by test/fake servers).
pub fn decode_handshake(buf: &[u8; 8]) -> Option<Mode> {
    if &buf[0..4] != b"VPMC" || buf[4] != PROTOCOL_VERSION || buf[6] != 0 || buf[7] != 0 {
        return None;
    }
    match buf[5] {
        0 => Some(Mode::Stream),
        1 => Some(Mode::Probe),
        _ => None,
    }
}

fn default_rate() -> u32 {
    48000
}
fn default_channels() -> u16 {
    1
}
fn default_format() -> String {
    "s16le".into()
}
fn default_frame_ms() -> u32 {
    10
}
fn default_version() -> u32 {
    1
}
fn default_volume() -> u32 {
    100
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    #[serde(default)]
    pub name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub format: String,
    #[serde(default = "default_frame_ms")]
    pub frame_ms: u32,
    #[serde(default = "default_version")]
    pub version: u32,
}

impl Default for Hello {
    fn default() -> Self {
        Hello {
            name: String::new(),
            sample_rate: default_rate(),
            channels: default_channels(),
            format: default_format(),
            frame_ms: default_frame_ms(),
            version: default_version(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    #[serde(default)]
    pub muted: bool,
    #[serde(default = "default_volume")]
    pub volume: u32,
    #[serde(default)]
    pub input: String,
}

impl Default for Status {
    fn default() -> Self {
        Status { muted: false, volume: 100, input: String::new() }
    }
}

/// BYE: the phone closes this stream on purpose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bye {
    #[serde(default)]
    pub reason: String,
}

pub const BYE_REPLACED: &str = "replaced";
pub const BYE_STOPPED: &str = "stopped";

impl Hello {
    /// Parse and validate a HELLO payload: must be a JSON object with sane
    /// sampleRate (8000..=192000) and channels (1..=8) and a format string.
    pub fn parse(payload: &[u8]) -> Result<Hello, String> {
        let v: serde_json::Value = serde_json::from_slice(payload).map_err(|e| format!("bad HELLO json: {e}"))?;
        if !v.is_object() {
            return Err("bad HELLO: not a JSON object".into());
        }
        let h: Hello = serde_json::from_value(v).map_err(|e| format!("bad HELLO: {e}"))?;
        if !(MIN_RATE..=MAX_RATE).contains(&h.sample_rate) {
            return Err(format!("bad HELLO: sampleRate {} out of range {MIN_RATE}..={MAX_RATE}", h.sample_rate));
        }
        if !(1..=MAX_CHANNELS).contains(&h.channels) {
            return Err(format!("bad HELLO: channels {} out of range 1..={MAX_CHANNELS}", h.channels));
        }
        Ok(h)
    }
}

/// UDP discovery reply JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryReply {
    pub service: String,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

impl DiscoveryReply {
    pub fn parse(buf: &[u8]) -> Option<DiscoveryReply> {
        let r: DiscoveryReply = serde_json::from_slice(buf).ok()?;
        (r.service == SERVICE_NAME).then_some(r)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Hello(Hello),
    Audio { seq: u32, capture_time_micros: u64, samples: Vec<i16> },
    Status(Status),
    Bye(Bye),
    /// Unknown frame type: ignored by the client for forward compatibility.
    Unknown { kind: u8, payload: Vec<u8> },
}

fn proto_err(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

impl Frame {
    /// Encode a frame including its 5-byte header.
    pub fn encode(&self) -> Vec<u8> {
        let (kind, payload) = match self {
            Frame::Hello(h) => (FRAME_HELLO, serde_json::to_vec(h).expect("json")),
            Frame::Status(s) => (FRAME_STATUS, serde_json::to_vec(s).expect("json")),
            Frame::Bye(b) => (FRAME_BYE, serde_json::to_vec(b).expect("json")),
            Frame::Audio { seq, capture_time_micros, samples } => {
                let mut p = Vec::with_capacity(12 + samples.len() * 2);
                p.extend_from_slice(&seq.to_be_bytes());
                p.extend_from_slice(&capture_time_micros.to_be_bytes());
                for s in samples {
                    p.extend_from_slice(&s.to_le_bytes());
                }
                (FRAME_AUDIO, p)
            }
            Frame::Unknown { kind, payload } => (*kind, payload.clone()),
        };
        let mut out = Vec::with_capacity(5 + payload.len());
        out.push(kind);
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&payload);
        out
    }

    /// Decode a frame from its type byte and payload.
    pub fn decode(kind: u8, payload: &[u8]) -> io::Result<Frame> {
        match kind {
            FRAME_HELLO => Hello::parse(payload).map(Frame::Hello).map_err(proto_err),
            FRAME_BYE => Ok(Frame::Bye(serde_json::from_slice(payload).unwrap_or(Bye { reason: String::new() }))),
            FRAME_STATUS => serde_json::from_slice(payload)
                .map(Frame::Status)
                .map_err(|e| proto_err(format!("bad STATUS json: {e}"))),
            FRAME_AUDIO => {
                if payload.len() < 12 || !(payload.len() - 12).is_multiple_of(2) {
                    return Err(proto_err(format!("bad AUDIO payload length {}", payload.len())));
                }
                let seq = u32::from_be_bytes(payload[0..4].try_into().unwrap());
                let ts = u64::from_be_bytes(payload[4..12].try_into().unwrap());
                let samples = payload[12..]
                    .chunks_exact(2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]))
                    .collect();
                Ok(Frame::Audio { seq, capture_time_micros: ts, samples })
            }
            other => Ok(Frame::Unknown { kind: other, payload: payload.to_vec() }),
        }
    }

    pub fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        w.write_all(&self.encode())
    }
}

/// Error message for a peer that does not speak the protocol.
pub const NOT_VPM: &str = "Not a Vibe Phone Mic server";

/// Read the first frame of a connection, which must be HELLO. Anything else
/// (wrong type byte, e.g. an unrelated service on the port) is reported as
/// "not a Vibe Phone Mic server" before trusting the length field.
pub fn read_first_frame<R: Read>(r: &mut R) -> io::Result<Option<Hello>> {
    let mut kind = [0u8; 1];
    match r.read(&mut kind) {
        Ok(0) => return Ok(None),
        Ok(_) => {}
        Err(e) => return Err(e),
    }
    if kind[0] != FRAME_HELLO {
        return Err(proto_err(format!(
            "{NOT_VPM} (expected HELLO, got frame type 0x{:02x})",
            kind[0]
        )));
    }
    let mut rest = Chain { first: Some(kind[0]), inner: r };
    match read_frame(&mut rest)? {
        Some(Frame::Hello(h)) => Ok(Some(h)),
        _ => Err(io::ErrorKind::UnexpectedEof.into()),
    }
}

/// Re-inject one already-read byte in front of a reader.
struct Chain<'a, R: Read> {
    first: Option<u8>,
    inner: &'a mut R,
}

impl<R: Read> Read for Chain<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if let Some(b) = self.first.take() {
            buf[0] = b;
            return Ok(1);
        }
        self.inner.read(buf)
    }
}

/// Read one frame. Returns `Ok(None)` on a clean EOF at a frame boundary.
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Option<Frame>> {
    let mut hdr = [0u8; 5];
    let mut got = 0;
    while got < hdr.len() {
        match r.read(&mut hdr[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let len = u32::from_be_bytes(hdr[1..5].try_into().unwrap());
    if len > MAX_PAYLOAD {
        return Err(proto_err(format!("payload too large: {len} bytes")));
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload)?;
    Frame::decode(hdr[0], &payload).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn handshake_roundtrip() {
        let h = encode_handshake(Mode::Stream);
        assert_eq!(h, [b'V', b'P', b'M', b'C', 1, 0, 0, 0]);
        assert_eq!(decode_handshake(&h), Some(Mode::Stream));
        let p = encode_handshake(Mode::Probe);
        assert_eq!(p[5], 1);
        assert_eq!(decode_handshake(&p), Some(Mode::Probe));
        let mut bad = h;
        bad[0] = b'X';
        assert_eq!(decode_handshake(&bad), None);
        let mut badv = h;
        badv[4] = 2;
        assert_eq!(decode_handshake(&badv), None);
    }

    #[test]
    fn audio_frame_layout_is_big_endian_header_le_pcm() {
        let f = Frame::Audio { seq: 0x01020304, capture_time_micros: 0x0A0B0C0D0E0F1011, samples: vec![1, -2] };
        let b = f.encode();
        assert_eq!(b[0], FRAME_AUDIO);
        assert_eq!(&b[1..5], &16u32.to_be_bytes());
        assert_eq!(&b[5..9], &[1, 2, 3, 4]);
        assert_eq!(&b[9..17], &[0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11]);
        assert_eq!(&b[17..21], &[0x01, 0x00, 0xFE, 0xFF]);
        assert_eq!(read_frame(&mut Cursor::new(b)).unwrap(), Some(f));
    }

    #[test]
    fn hello_and_status_roundtrip() {
        let hello = Hello { name: "Pixel 7".into(), sample_rate: 44100, channels: 2, ..Default::default() };
        let enc = Frame::Hello(hello.clone()).encode();
        let json: serde_json::Value = serde_json::from_slice(&enc[5..]).unwrap();
        assert_eq!(json["sampleRate"], 44100);
        assert_eq!(json["frameMs"], 10);
        let status = Status { muted: true, volume: 150, input: "Built-in microphone".into() };
        let mut buf = enc;
        buf.extend(Frame::Status(status.clone()).encode());
        let mut c = Cursor::new(buf);
        assert_eq!(read_frame(&mut c).unwrap(), Some(Frame::Hello(hello)));
        assert_eq!(read_frame(&mut c).unwrap(), Some(Frame::Status(status)));
        assert_eq!(read_frame(&mut c).unwrap(), None);
    }

    #[test]
    fn decodes_phone_json_exactly_as_specified() {
        let p = br#"{"name":"Pixel 7","sampleRate":48000,"channels":1,"format":"s16le","frameMs":10,"version":1}"#;
        let Frame::Hello(h) = Frame::decode(FRAME_HELLO, p).unwrap() else { panic!() };
        assert_eq!(h.name, "Pixel 7");
        assert_eq!(h.sample_rate, 48000);
        let s = br#"{"muted":false,"volume":100,"input":"Built-in microphone"}"#;
        let Frame::Status(s) = Frame::decode(FRAME_STATUS, s).unwrap() else { panic!() };
        assert_eq!(s.volume, 100);
        assert!(!s.muted);
    }

    #[test]
    fn rejects_oversized_and_truncated() {
        let mut b = vec![FRAME_AUDIO];
        b.extend_from_slice(&(MAX_PAYLOAD + 1).to_be_bytes());
        assert!(read_frame(&mut Cursor::new(b)).is_err());
        let f = Frame::Status(Status::default()).encode();
        assert!(read_frame(&mut Cursor::new(&f[..f.len() - 1])).is_err());
        assert!(Frame::decode(FRAME_AUDIO, &[0; 11]).is_err());
        assert!(Frame::decode(FRAME_AUDIO, &[0; 13]).is_err());
        assert!(Frame::decode(FRAME_HELLO, b"not json").is_err());
    }

    #[test]
    fn hello_validation() {
        let ok = |j: &str| Frame::decode(FRAME_HELLO, j.as_bytes());
        assert!(ok(r#"{"sampleRate":44100,"channels":2,"format":"s16le"}"#).is_ok());
        for bad in [
            "[]",
            "[48000,1,\"s16le\"]",
            "42",
            r#"{"channels":1,"format":"s16le"}"#,
            r#"{"sampleRate":48000,"format":"s16le"}"#,
            r#"{"sampleRate":48000,"channels":1}"#,
            r#"{"sampleRate":48000,"channels":257,"format":"s16le"}"#,
            r#"{"sampleRate":48000,"channels":0,"format":"s16le"}"#,
            r#"{"sampleRate":48000,"channels":9,"format":"s16le"}"#,
            r#"{"sampleRate":7999,"channels":1,"format":"s16le"}"#,
            r#"{"sampleRate":192001,"channels":1,"format":"s16le"}"#,
            r#"{"sampleRate":-1,"channels":1,"format":"s16le"}"#,
        ] {
            let e = ok(bad).unwrap_err().to_string();
            assert!(e.contains("HELLO"), "{bad}: {e}");
        }
        assert!(ok(r#"{"sampleRate":8000,"channels":8,"format":"s16le"}"#).is_ok());
        assert!(ok(r#"{"sampleRate":192000,"channels":1,"format":"s16le"}"#).is_ok());
    }

    #[test]
    fn bye_roundtrip() {
        let f = Frame::Bye(Bye { reason: BYE_REPLACED.into() });
        let b = f.encode();
        assert_eq!(b[0], 0x04);
        assert_eq!(&b[5..], br#"{"reason":"replaced"}"#);
        assert_eq!(read_frame(&mut Cursor::new(b)).unwrap(), Some(f));
        // Garbage BYE payload still means BYE.
        assert!(matches!(Frame::decode(FRAME_BYE, b"x"), Ok(Frame::Bye(_))));
    }

    #[test]
    fn first_frame_must_be_hello() {
        let hello = Frame::Hello(Hello::default()).encode();
        assert!(read_first_frame(&mut Cursor::new(hello)).unwrap().is_some());
        let e = read_first_frame(&mut Cursor::new(b"HTTP/1.1 400 Bad Request\r\n".to_vec())).unwrap_err();
        assert!(e.to_string().contains(NOT_VPM), "{e}");
        let e = read_first_frame(&mut Cursor::new(Frame::Status(Status::default()).encode())).unwrap_err();
        assert!(e.to_string().contains("HELLO"), "{e}");
        assert!(read_first_frame(&mut Cursor::new(Vec::new())).unwrap().is_none());
    }

    #[test]
    fn unknown_frames_are_passed_through() {
        let f = Frame::Unknown { kind: 0x7f, payload: vec![1, 2, 3] };
        assert_eq!(read_frame(&mut Cursor::new(f.encode())).unwrap(), Some(f));
    }

    #[test]
    fn discovery_reply_parse() {
        let r = DiscoveryReply::parse(br#"{"service":"vibe-phone-mic","version":1,"name":"Pixel","port":48777}"#).unwrap();
        assert_eq!(r.name, "Pixel");
        assert_eq!(r.port, 48777);
        assert!(DiscoveryReply::parse(br#"{"service":"other","port":1}"#).is_none());
        assert!(DiscoveryReply::parse(b"garbage").is_none());
    }
}
