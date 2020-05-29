use crate::header::{Header, StreamId};
use std::{convert::TryInto, num::TryFromIntError};

/// A Yamux message frame consisting of header and body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub header: Header,
    pub body: Vec<u8>
}

impl Frame {
    pub fn new(header: Header) -> Self {
        Frame { header, body: Vec::new() }
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

   pub fn header_mut(&mut self) -> &mut Header {
        &mut self.header
    }
}


impl Frame {
    /// Create a new data frame.
    pub fn data(id: StreamId, b: Vec<u8>) -> Result<Self, TryFromIntError> {
        Ok(Frame {
            header: Header::data(id, b.len().try_into()?),
            body: b
        })
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn body_len(&self) -> u32 {
        // Safe cast since we construct `Frame::<Data>`s only with
        // `Vec<u8>` of length [0, u32::MAX] in `Frame::data` above.
        self.body().len() as u32
    }

    pub fn into_body(self) -> Vec<u8> {
        self.body
    }

    /// Create a new window update frame.
    pub fn window_update(id: StreamId, credit: u32) -> Self {
        Frame {
            header: Header::window_update(id, credit),
            body: Vec::new()
        }
    }

    /// Create a new go away frame.
    pub fn term() -> Self {
        Frame {
            header: Header::term(),
            body: Vec::new()
        }
    }

    pub fn protocol_error() -> Self {
        Frame {
            header: Header::protocol_error(),
            body: Vec::new()
        }
    }

    pub fn internal_error() -> Self {
        Frame {
            header: Header::internal_error(),
            body: Vec::new()
        }
    }
}
