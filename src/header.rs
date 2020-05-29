use std::fmt;
/// A tag is the runtime representation of a message type.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Tag {
    Data,
    WindowUpdate,
    Ping,
    GoAway
}

/// Possible flags set on a message.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Flags(u16);

impl Flags {
    pub fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn val(self) -> u16 {
        self.0
    }
}

const MAX_FLAG_VAL: u16 = 8;

/// Indicates the start of a new stream.
pub const SYN: Flags = Flags(1);

/// Acknowledges the start of a new stream.
pub const ACK: Flags = Flags(2);

/// Indicates the half-closing of a stream.
pub const FIN: Flags = Flags(4);

/// Indicates an immediate stream reset.
pub const RST: Flags = Flags(8);

/// The ID of a stream.
///
/// The value 0 denotes no particular stream but the whole session.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StreamId(u32);

pub const CONNECTION_ID: StreamId = StreamId(0);

impl StreamId {
    pub(crate) fn new(val: u32) -> Self {
        StreamId(val)
    }

    pub fn is_server(self) -> bool {
        self.0 % 2 == 0
    }

    pub fn is_client(self) -> bool {
        !self.is_server()
    }

    pub fn is_session(self) -> bool {
        self == CONNECTION_ID
    }

    pub fn val(self) -> u32 {
        self.0
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}


/// The message frame header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub tag: Tag,
    pub flags: Flags,
    pub stream_id: StreamId,
    pub length: u32,
}

impl Header {
    /// Create a new data frame header.
    pub fn data(id: StreamId, len: u32) -> Self {
        Header {
            version: 0,
            tag: Tag::Data,
            flags: Flags(0),
            stream_id: id,
            length: len,
        }
    }

    /// Create a new window update frame header.
    pub fn window_update(id: StreamId, credit: u32) -> Self {
        Header {
            version: 0,
            tag: Tag::WindowUpdate,
            flags: Flags(0),
            stream_id: id,
            length: credit,
        }
    }

    /// The credit this window update grants to the remote.
    pub fn credit(&self) -> u32 {
        self.length
    }

    /// Create a new ping frame header.
    pub fn ping(nonce: u32) -> Self {
        Header {
            version: 0,
            tag: Tag::Ping,
            flags: Flags(0),
            stream_id: StreamId(0),
            length: nonce,
        }
    }

    /// The nonce of this ping.
    pub fn nonce(&self) -> u32 {
        self.length
    }


    /// Terminate the session without indicating an error to the remote.
    pub fn term() -> Self {
        Self::go_away(0)
    }

    /// Terminate the session indicating a protocol error to the remote.
    pub fn protocol_error() -> Self {
        Self::go_away(1)
    }

    /// Terminate the session indicating an internal error to the remote.
    pub fn internal_error() -> Self {
        Self::go_away(2)
    }

    fn go_away(code: u32) -> Self {
        Header {
            version: 0,
            tag: Tag::GoAway,
            flags: Flags(0),
            stream_id: StreamId(0),
            length: code,
        }
    }
}

/// The serialised header size in bytes.
pub const HEADER_SIZE: usize = 12;

/// Encode a [`Header`] value.
pub fn encode(hdr: &Header) -> [u8; HEADER_SIZE] {
    let mut buf = [0; HEADER_SIZE];
    buf[0] = hdr.version;
    buf[1] = hdr.tag as u8;
    buf[2 .. 4].copy_from_slice(&hdr.flags.0.to_be_bytes());
    buf[4 .. 8].copy_from_slice(&hdr.stream_id.0.to_be_bytes());
    buf[8 .. HEADER_SIZE].copy_from_slice(&hdr.length.to_be_bytes());
    buf
}


/// Possible errors while decoding a message frame header.
#[non_exhaustive]
#[derive(Debug)]
pub enum HeaderDecodeError {
    /// Unknown version.
    Version(u8),
    /// An unknown frame type.
    Type(u8),
    /// Unknown flags.
    Flags(u16)
}

impl std::fmt::Display for HeaderDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            HeaderDecodeError::Version(v) => write!(f, "unknown version: {}", v),
            HeaderDecodeError::Type(t) => write!(f, "unknown frame type: {}", t),
            HeaderDecodeError::Flags(x) => write!(f, "unknown flags type: {}", x)
        }
    }
}


/// Decode a [`Header`] value.
pub fn decode(buf: &[u8; HEADER_SIZE]) -> Result<Header, HeaderDecodeError> {
    if buf[0] != 0 {
        return Err(HeaderDecodeError::Version(buf[0]))
    }

    let hdr = Header {
        version: buf[0],
        tag: match buf[1] {
            0 => Tag::Data,
            1 => Tag::WindowUpdate,
            2 => Tag::Ping,
            3 => Tag::GoAway,
            t => return Err(HeaderDecodeError::Type(t))
        },
        flags: Flags(u16::from_be_bytes([buf[2], buf[3]])),
        stream_id: StreamId(u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]])),
        length: u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]),
    };

    if hdr.flags.0 > MAX_FLAG_VAL {
        return Err(HeaderDecodeError::Flags(hdr.flags.0))
    }

    Ok(hdr)
}
