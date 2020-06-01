use crate::header::{self, Header, StreamId, encode, decode, Tag, HEADER_SIZE, Flags, ACK, FIN, SYN};
use super::{Config, DEFAULT_CREDIT};
use futures::prelude::*;
use std::{fmt, sync::Arc, task::{Context, Poll}};
use std::collections::HashMap;
use crate::stream::{self, Stream, Flag};
use crate::frame::Frame;
use yaanhyy_secio::codec::SecureConn;
use yaanhyy_secio::identity::Keypair;
use yaanhyy_secio::config::SecioConfig;
use yaanhyy_secio::handshake::handshake;
use crate::io::ReadState;
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
pub enum Shutdown {
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
pub struct RawSession<S> {
    id: Id,
    mode: Mode,
    config: Arc<Config>,
    socket: S,
    next_id: u32,

    garbage: Vec<StreamId>, // see `Connection::garbage_collect()`
    shutdown: Shutdown,
    is_closed: bool
}

pub struct SecioSession<S> {
    pub id: Id,
    pub mode: Mode,
    pub config: Arc<Config>,
    pub socket: SecureConn<S>,
    pub next_id: u32,
    pub streams: HashMap<u32, Stream>,
    pub garbage: Vec<StreamId>, // see `Connection::garbage_collect()`
    pub shutdown: Shutdown,
    pub state: ReadState,
    pub is_closed: bool
}

//impl <S: AsyncRead + AsyncWrite  + Send + Unpin + 'static>RawSession<S> {
//    pub fn new(socket: S, cfg: Config, mode: Mode) -> Self
//    {
//        let id = Id::random();
//        log::debug!("new connection: {:?} ({:?})", id.0, mode);
//
//        //let socket = frame::Io::new(id, socket, cfg.max_buffer_size);
//        RawSession {
//            id,
//            mode,
//            config: Arc::new(cfg),
//            socket,
//
//            next_id: match mode {
//                Mode::Client => 1,
//                Mode::Server => 2
//            },
//            garbage: Vec::new(),
//            shutdown: Shutdown::NotStarted,
//            is_closed: false
//        }
//    }
//
//    fn next_stream_id(&mut self) -> Result<StreamId, String> {
//        let proposed = StreamId::new(self.next_id);
//        self.next_id = self.next_id + 2;
//        match self.mode {
//            Mode::Client => assert!(proposed.is_client()),
//            Mode::Server => assert!(proposed.is_server())
//        }
//        Ok(proposed)
//    }
//
//    pub async fn open_raw_stream(&mut self) -> Result<Stream, String> {
//        let id = self.next_stream_id()?;
//
//        if !self.config.lazy_open {
//            let mut frame = Frame::window_update(id, self.config.receive_window);
//            frame.header_mut().syn();
//            println!("{}: sending initial {:?}", self.id.0, frame.header());
//            let header = encode(&frame.header);
//            let res = self.socket.write_all(&header).await;
//            if let Ok(_) = res {
//                self.socket.write_all(&frame.body).await;
//            }
//        }
//
//        let stream = {
//            let config = self.config.clone();
//            let window = self.config.receive_window;
//            let mut stream = Stream::new(id, self.id, config, window, DEFAULT_CREDIT);
//            if self.config.lazy_open {
//                stream.set_flag(stream::Flag::Syn)
//            }
//            stream
//        };
//
//        let mut buffer =  [0u8; header::HEADER_SIZE];
//
//        self.socket.read_exact(&mut buffer).await;
//        println!("buffer:{:?}", buffer);
//        let header =
//            match decode(&buffer) {
//                Ok(hd) => hd,
//                Err(e) => return Err("decode header fail".to_string()),
//            };
//        println!("receice header:{:?}", header);
////        let mut id = stream.id().val();
////        let wrong = StreamId::new(id+1);
//        let send_frame = Frame::data(stream.id(), "hello yamux".to_string().into_bytes())?;
//
//        let header = encode(&send_frame.header);
//        let res = self.socket.write_all(&header).await;
//        if let Ok(_) = res {
//            self.socket.write_all(&send_frame.body).await;
//        }
//
//        let mut buffer =  [0u8; header::HEADER_SIZE];
//
//        self.socket.read_exact(&mut buffer).await;
//        println!("buffer:{:?}", buffer);
//        let header =
//            match decode(&buffer) {
//                Ok(hd) => hd,
//                Err(e) => return Err("decode header fail".to_string()),
//            };
//
//        println!("receice header:{:?}", header);
//
//        if header.tag == Tag::Data {
//            let mut buffer = vec![0u8; header.length as usize];
//            self.socket.read_exact(&mut buffer).await;
//            println!("receice data:{}", std::str::from_utf8(&buffer).unwrap());
//        }
//
//
//        let mut buffer =  [0u8; header::HEADER_SIZE];
//
//        self.socket.read_exact(&mut buffer).await;
//        println!("buffer:{:?}", buffer);
//        let header =
//            match decode(&buffer) {
//                Ok(hd) => hd,
//                Err(e) => return Err("decode header fail".to_string()),
//            };
//        println!("header:{:?}", header);
//
//
//
//        Ok(stream)
//    }
//}

impl <S: AsyncRead + AsyncWrite  + Send + Unpin + 'static>SecioSession<S> {
    pub fn new(socket: SecureConn<S>, cfg: Config, mode: Mode) -> Self
    {
        let id = Id::random();
        log::debug!("new connection: {:?} ({:?})", id.0, mode);

        //let socket = frame::Io::new(id, socket, cfg.max_buffer_size);
        SecioSession {
            id,
            mode,
            config: Arc::new(cfg),
            socket,
            streams: HashMap::new(),
            next_id: match mode {
                Mode::Client => 1,
                Mode::Server => 2
            },
            state: ReadState::Init,
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

    pub async fn open_secio_stream(&mut self) -> Result<StreamId, String> {
        let id = self.next_stream_id()?;
        if !self.config.lazy_open {
            self.window_update_frame_send(id).await;
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


        self.streams.insert(stream.id().val(), stream.clone());
        Ok(stream.id())
    }

    pub async fn window_update_frame_send(&mut self, id: StreamId) -> Result<(),String>{
        let mut frame = Frame::window_update(id, self.config.receive_window);
        frame.header_mut().syn();
        println!("{}: sending initial {:?}", self.id.0, frame.header());
        let header = encode(&frame.header);
        self.socket.send(& mut header.to_vec()).await
    }

    pub async fn data_frame_send(&mut self, stream_id: StreamId, data: Vec<u8>) -> Result<(), String>{
        let stream = self.streams.get(&stream_id.val());
        if let Some(stream) = stream {
            let mut send_frame = Frame::data(stream.id(), data)?;
            if stream.flag  == Flag::Ack {
                send_frame.header.ack();
            } else if  stream.flag == Flag::Syn {
                send_frame.header.syn();
            }
            let mut header = encode(&send_frame.header);
            self.socket.send(& mut header.to_vec()).await?;
            self.socket.send(& mut send_frame.body).await?;
            return Ok(())
        }
        Err("stream not founded".to_string())
    }

    pub async fn receive_frame(&mut self) -> Result<Frame ,String> {

        loop {
            match self.state {
                ReadState::Init => {
                    self.state = ReadState::Header {
                        offset: 0,
                        buffer: [0; header::HEADER_SIZE]
                    };
                },
                ReadState::Header{ref mut offset, ref mut buffer} => {
                    let mut read_buf = self.socket.read().await?;
                    println!("header buffer:{:?}", buffer);
                    if read_buf.len() < (HEADER_SIZE-*offset) {
                        buffer[*offset..].copy_from_slice(read_buf.as_slice());
                        *offset += read_buf.len();
                        continue;
                    } else {
                        let rest = read_buf.split_off(HEADER_SIZE-*offset);
                        buffer[*offset..].copy_from_slice(read_buf.as_slice());

                        let header = match decode(&buffer) {
                            Ok(hd) => hd,
                            Err(e) => return Err("decode header fail".to_string()),
                        };
                        println!("receice header:{:?}", header);
                        match header.tag {
                            Tag::Data => {
                                let frame_len = header.length;
                                let data_buf = vec![0; frame_len as usize];
                                if rest.len() as u32 == frame_len {
                                    self.state = ReadState::Init;
                                    return Ok(Frame{header, body: rest});
                                } else if (rest.len() as u32) < frame_len {
                                    self.state = ReadState::Body { header, offset: rest.len(), buffer: rest};
                                } else {
                                    println!("have rest data in next frame");
                                }
                            },

                            _ => {
                                if !rest.is_empty() {
//                                    let mut buf = [0; header::HEADER_SIZE];
//                                    buf.copy_from_slice(rest.as_slice());
//                                    self.state = ReadState::Header {
//                                        offset: rest.len(),
//                                        buffer: buf
//                                    };
                                    println!("have rest data in next frame");
                                } else {
                                    self.state = ReadState::Init;
                                }
                                return Ok(Frame::new(header));
                            },
                        }
                    }
                }
                ReadState::Body {ref header, ref mut offset, ref mut buffer} => {
                    let mut read_buf = self.socket.read().await?;
                    if read_buf.len()  == (header.length  as usize - *offset) {
                        let h = header.clone();
                        buffer.append(& mut read_buf);
                        let buffer_clone = buffer.clone();
                        self.state = ReadState::Init;
                        return Ok(Frame{header: h, body: (*buffer_clone).to_vec()});
                    } else if read_buf.len() < (header.length  as usize - *offset) {
                        *offset += read_buf.len();
                        buffer.append(& mut read_buf);

                    } else {
                        println!("have rest data in next frame");
                    }
                }
            }
        }
    }


    pub async fn receive_loop(&mut self) {
        loop {
            let frame: Result<Frame ,String>  = self.receive_frame().await;
            if let Ok(frame) = frame {
                match frame.header.tag {
                    Tag::Ping => {

                    }
                    Tag::GoAway=> {

                    }
                    Tag::WindowUpdate=> {

                    }
                    Tag::Data => {

                    }
                }
            }
        }
    }
}

//#[test]
//fn yamux_server_test() {
//    async_std::task::block_on(async move {
//        let listener = async_std::net::TcpListener::bind("127.0.0.1:8980").await.unwrap();
//        let connec = listener.accept().await.unwrap().0;
//        let mut session = RawSession::new(connec, Config::default(), Mode::Server);
//        let stream = session.open_raw_stream().await;
//
//
//        if let Ok(mut stream) = stream {
//
//            let mut msg = "ok".to_string();
//
////            let len = msg.len();
////            stream.write_all(msg.as_bytes()).await.unwrap();
////            println!("C: {}: sent {} bytes", id, len);
////            stream.close().await.unwrap();
////            let mut data = Vec::new();
////            stream.read_to_end(&mut data).await.unwrap();
////            println!("C: {}: received {} bytes,{:?}", id, data.len(), data);
//            // result.push(data)
//        } else {
//            println!("open_stream fail" );
//        }
//    });
//}

#[test]
fn yamux_secio_client_open_stream_test() {
    async_std::task::block_on(async move {
        let connec = async_std::net::TcpStream::connect("127.0.0.1:8981").await.unwrap();
        let key1 = Keypair::generate_ed25519();
        let mut config = SecioConfig::new(key1);
        let mut res = handshake(connec, config).await;
        if let Ok(mut secure_conn) = res {
            let mut session = SecioSession::new(secure_conn, Config::default(), Mode::Client);
            let res = session.open_secio_stream().await;


            if let Ok(id) = res {
                session.data_frame_send(id , "hello yamux".to_string().into_bytes()).await;
                let mut msg = "ok".to_string();
                println!("open_stream success" );
            } else {
                println!("open_stream fail" );
            }
            let receive_process = session.receive_loop();
            receive_process.await;
        }
    });
}


//#[test]
//fn yamux_client_test() {
//    async_std::task::block_on(async move {
//        let connec = async_std::net::TcpStream::connect("127.0.0.1:8980").await.unwrap();
//        let mut session = RawSession::new(connec, Config::default(), Mode::Client);
//        let stream = session.open_raw_stream().await;
//
//        if let Ok(mut stream) = stream {
//            let id = stream.id();
//            let mut msg = "ok".to_string();
//
////            let len = msg.len();
////            stream.write_all(msg.as_bytes()).await.unwrap();
////            println!("C: {}: sent {} bytes", id, len);
////            stream.close().await.unwrap();
////            let mut data = Vec::new();
////            stream.read_to_end(&mut data).await.unwrap();
////            println!("C: {}: received {} bytes,{:?}", id, data.len(), data);
//            // result.push(data)
//        } else {
//            println!("open_stream fail" );
//        }
//        loop {}
//    });
//}

#[test]
fn slice_copy_test() {
    let mut src = vec![1, 2, 3, 4];
    let rest = src.split_off(2);
    let mut dst = [0, 0,0,0];
    dst[2..4].copy_from_slice(&rest[0..2]);
    println!("dst:{:?}", dst);
}