use crate::header::{self, Header, StreamId, encode, decode, Tag};
use super::{Config, DEFAULT_CREDIT};
use futures::prelude::*;
use std::{fmt, sync::Arc, task::{Context, Poll}};
use crate::stream::{self, Stream};
use crate::frame::Frame;

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
pub struct Id(u32);

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

    fn next_stream_id(&mut self) -> Result<StreamId, String> {
        let proposed = StreamId::new(self.next_id);
        self.next_id = self.next_id + 2;
        match self.mode {
            Mode::Client => assert!(proposed.is_client()),
            Mode::Server => assert!(proposed.is_server())
        }
        Ok(proposed)
    }

    pub async fn open_stream(&mut self) -> Result<Stream, String> {
        let id = self.next_stream_id()?;

        if !self.config.lazy_open {
            let mut frame = Frame::window_update(id, self.config.receive_window);
            frame.header_mut().syn();
            println!("{}: sending initial {:?}", self.id.0, frame.header());
            let header = encode(&frame.header);
            let res = self.socket.write_all(&header).await;
            if let Ok(_) = res {
                self.socket.write_all(&frame.body).await;
            }
        }

        let stream = {
            let config = self.config.clone();
            let window = self.config.receive_window;
            let mut stream = Stream::new(id, self.id, config, window, DEFAULT_CREDIT);
            if self.config.lazy_open {
                stream.set_flag(stream::Flag::Syn)
            }
            stream
        };

        let mut buffer =  [0u8; header::HEADER_SIZE];

        self.socket.read_exact(&mut buffer).await;
        println!("buffer:{:?}", buffer);
        let header =
            match decode(&buffer) {
                Ok(hd) => hd,
                Err(e) => return Err("decode header fail".to_string()),
            };
        println!("header:{:?}", header);

        let send_frame = Frame::data(stream.id(), "hello yamux".to_string().into_bytes())?;

        let header = encode(&send_frame.header);
        let res = self.socket.write_all(&header).await;
        if let Ok(_) = res {
            self.socket.write_all(&send_frame.body).await;
        }

        let mut buffer =  [0u8; header::HEADER_SIZE];

        self.socket.read_exact(&mut buffer).await;
        println!("buffer:{:?}", buffer);
        let header =
            match decode(&buffer) {
                Ok(hd) => hd,
                Err(e) => return Err("decode header fail".to_string()),
            };

        println!("header:{:?}", header);

        if header.tag == Tag::Data {
            let mut buffer = vec![0u8; header.length as usize];
            self.socket.read_exact(&mut buffer).await;
            println!("buffer data:{}", std::str::from_utf8(&buffer).unwrap());
        }


        let mut buffer =  [0u8; header::HEADER_SIZE];

        self.socket.read_exact(&mut buffer).await;
        println!("buffer:{:?}", buffer);
        let header =
            match decode(&buffer) {
                Ok(hd) => hd,
                Err(e) => return Err("decode header fail".to_string()),
            };
        println!("header:{:?}", header);



        Ok(stream)
    }
}

#[test]
fn yamux_server_test() {
    async_std::task::block_on(async move {
        let listener = async_std::net::TcpListener::bind("127.0.0.1:8980").await.unwrap();
        let connec = listener.accept().await.unwrap().0;
        let mut session = Session::new(connec, Config::default(), Mode::Server);
        let stream = session.open_stream().await;

        if let Ok(mut stream) = stream {
            let id = stream.id();
            let mut msg = "ok".to_string();

//            let len = msg.len();
//            stream.write_all(msg.as_bytes()).await.unwrap();
//            println!("C: {}: sent {} bytes", id, len);
//            stream.close().await.unwrap();
//            let mut data = Vec::new();
//            stream.read_to_end(&mut data).await.unwrap();
//            println!("C: {}: received {} bytes,{:?}", id, data.len(), data);
            // result.push(data)
        } else {
            println!("open_stream fail" );
        }
    });
}

#[test]
fn yamux_client_test() {
    async_std::task::block_on(async move {
        let connec = async_std::net::TcpStream::connect("127.0.0.1:8980").await.unwrap();
        let mut session = Session::new(connec, Config::default(), Mode::Client);
        let stream = session.open_stream().await;

        if let Ok(mut stream) = stream {
            let id = stream.id();
            let mut msg = "ok".to_string();

//            let len = msg.len();
//            stream.write_all(msg.as_bytes()).await.unwrap();
//            println!("C: {}: sent {} bytes", id, len);
//            stream.close().await.unwrap();
//            let mut data = Vec::new();
//            stream.read_to_end(&mut data).await.unwrap();
//            println!("C: {}: received {} bytes,{:?}", id, data.len(), data);
            // result.push(data)
        } else {
            println!("open_stream fail" );
        }
        loop {}
    });
}