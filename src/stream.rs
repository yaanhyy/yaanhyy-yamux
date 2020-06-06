use super::session::Id;
use super::header::{StreamId, Header};
use super::frame::{Frame};
use super::Config;
use std::{fmt, sync::Arc, task::{Context, Poll}};
use futures::{channel::{mpsc, oneshot}};

/// The state of a Yamux stream.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Open bidirectionally.
    Open,
    /// Open for incoming messages.
    SendClosed,
    /// Open for outgoing messages.
    RecvClosed,
    /// Closed (terminal state).
    Closed
}

/// Indicate if a flag still needs to be set on an outbound header.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Flag {
    /// No flag needs to be set.
    None,
    /// The stream was opened lazily, so set the initial SYN flag.
    Syn,
    /// The stream still needs acknowledgement, so set the ACK flag.
    Ack
}


/// A multiplexed Yamux stream.
///
/// Streams are created either outbound via [`crate::Control::open_stream`]
/// or inbound via [`crate::Connection::next_stream`].
///
/// `Stream` implements [`AsyncRead`] and [`AsyncWrite`] and also
/// [`futures::stream::Stream`].
#[derive(Clone)]
pub struct Stream {
    pub id: StreamId,
    pub conn: Id,
    pub config: Arc<Config>,
    pub pending: Option<Frame>,
    //pub data_receiver: mpsc<Frame>,
    pub flag: Flag,
}

impl fmt::Debug for Stream {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Stream")
            .field("id", &self.id.val())
            .field("connection", &self.conn.0)
            .field("pending", &self.pending.is_some())
            .finish()
    }
}

impl fmt::Display for Stream {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "(Stream {}/{})", self.conn.0, self.id.val())
    }
}

impl Stream {
    pub fn new
    (id: StreamId, conn: Id, config: Arc<Config>) -> Self
    {
        Stream {
            id,
            conn,
            config,
            pending: None,
            flag: Flag::None,
      //      data_receiver: m
        }
    }

    /// Get this stream's identifier.
    pub fn id(&self) -> StreamId {
        self.id
    }

    /// Set the flag that should be set on the next outbound frame header.
    pub(crate) fn set_flag(&mut self, flag: Flag) {
        self.flag = flag
    }

    pub(crate) fn clone(&self) -> Self {
        Stream {
            id: self.id,
            conn: self.conn,
            config: self.config.clone(),

            pending: None,
            flag: self.flag,

        }
    }

    /// Set ACK or SYN flag if necessary.
    fn add_flag(&mut self, header: &mut Header) {
        match self.flag {
            Flag::None => (),
            Flag::Syn => {
                header.syn();
                self.flag = Flag::None
            }
            Flag::Ack => {
                header.ack();
                self.flag = Flag::None
            }
        }
    }
}