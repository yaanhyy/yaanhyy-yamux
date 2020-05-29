use crate::header::{Header, StreamId};
use super::Config;
use futures::prelude::*;
use std::{fmt, sync::Arc, task::{Context, Poll}};

/// How the connection is used.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
pub enum Mode {
    /// Client to server connection.
    Client,
    /// Server to client connection.
    Server
}

/// This enum captures the various stages of shutting down the connection.
#[derive(Debug)]
enum Shutdown {
    /// We are open for business.
    NotStarted,
    /// We have received a `ControlCommand::Close` and are shutting
    /// down operations. The `Sender` will be informed once we are done.
    InProgress,
    /// The shutdown is complete and we are closed for good.
    Complete
}

/// The connection identifier.
///
/// Randomly generated, this is mainly intended to improve log output.
#[derive(Clone, Copy)]
pub(crate) struct Id(u32);

impl Id {
    /// Create a random connection ID.
    pub(crate) fn random() -> Self {
        Id(rand::random())
    }
}

/// A Yamux connection object.
///
/// Wraps the underlying I/O resource and makes progress via its
/// [`Connection::next_stream`] method which must be called repeatedly
/// until `Ok(None)` signals EOF or an error is encountered.
pub struct Session<S> {
    id: Id,
    mode: Mode,
    config: Arc<Config>,
    socket: S,
    next_id: u32,

    garbage: Vec<StreamId>, // see `Connection::garbage_collect()`
    shutdown: Shutdown,
    is_closed: bool
}

impl <S: AsyncRead + AsyncWrite  + Send + Unpin + 'static>Session<S> {
    pub fn new(socket: S, cfg: Config, mode: Mode) -> Self
    {
        let id = Id::random();
        log::debug!("new connection: {:?} ({:?})", id.0, mode);

        //let socket = frame::Io::new(id, socket, cfg.max_buffer_size);
        Session {
            id,
            mode,
            config: Arc::new(cfg),
            socket,

            next_id: match mode {
                Mode::Client => 1,
                Mode::Server => 2
            },
            garbage: Vec::new(),
            shutdown: Shutdown::NotStarted,
            is_closed: false
        }
    }
}