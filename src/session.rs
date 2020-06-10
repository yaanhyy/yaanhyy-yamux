use crate::header::{self, Header, StreamId, encode, decode, Tag, HEADER_SIZE, Flags, ACK, FIN, SYN, CONNECTION_ID};
use super::{Config, DEFAULT_CREDIT, WindowUpdateMode};
use futures::prelude::*;
use futures::{future, select, join};
use async_std::sync::Mutex;
use std::{fmt, sync::{Arc}, task::{Context, Poll}, pin::Pin};
use std::collections::HashMap;
use crate::stream::{self, Stream, Flag};
use crate::frame::Frame;
use yaanhyy_secio::codec::{SecureHalfConnRead, SecureHalfConnWrite};
use yaanhyy_secio::identity::Keypair;
use yaanhyy_secio::config::SecioConfig;
use yaanhyy_secio::handshake::handshake;
use crate::io::receive_frame;
use futures::future::Future;
use futures::{channel::{mpsc, oneshot}};
use pin_utils::pin_mut;
use std::time::Duration;
use async_std::task;
use futures::future::Either;


/// How the connection is used.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
pub enum Mode {
    /// Client to server connection.
    Client,
    /// Server to client connection.
    Server
}

/// Possible actions as a result of incoming frame handling.
#[derive(Debug)]
pub enum Action {
    /// Nothing to be done.
    None,
    /// A new stream has been opened by the remote.
    New(Stream),
    /// A window update should be sent to the remote.
    Update(Frame),
    /// A ping should be answered.
    Ping(Frame),
    /// A stream should be reset.
    Reset(Frame),
    /// The connection should be terminated.
    Terminate(Frame)
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

/// `Control` to `Connection` commands.
#[derive(Debug)]
pub enum ControlCommand {
    /// Open a new stream to the remote end.
    OpenStream(oneshot::Sender<Result<Stream, String>>),
    /// Open a new stream to the remote end.
    GetStream((u32, oneshot::Sender<Result<Stream, String>>)),
    /// Open a new stream to the remote end.
    SubscribeStream(oneshot::Sender<Result<mpsc::Receiver<Stream>, String>>),
    /// Close the whole connection.
    CloseConnection(())
}

/// `Stream` to `Connection` commands.
#[derive(Debug)]
pub enum StreamCommand {
    /// A new frame should be sent to the remote.
    SendFrame(Frame),
    /// Close a stream.
    CloseStream { id: StreamId, ack: bool }
}

/// The connection identifier.
///
/// Randomly generated, this is mainly intended to improve log output.
#[derive(Clone, Copy)]
pub struct Id(pub u32);

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


pub struct SecioSessionWriter<S> {
    pub socket: Arc<Mutex<SecureHalfConnWrite<S>>>,
    pub stream_receiver: mpsc::Receiver<StreamCommand>,
}

pub struct SecioSessionReader<S> {
    pub id: Id,
    pub mode: Mode,
    pub config: Arc<Config>,
    pub socket: Arc<Mutex<SecureHalfConnRead<S>>>,
    pub stream_sender: mpsc::Sender<StreamCommand>,
    pub remote_stream_sender: Option<mpsc::Sender<Stream>>,
    pub next_id: u32,
    pub streams: HashMap<u32, Stream>,
    pub garbage: Vec<StreamId>, // see `Connection::garbage_collect()`
    pub shutdown: Shutdown,
    pub is_closed: bool,
}

impl <S: AsyncWrite + Send + Unpin + 'static>SecioSessionWriter<S> {
    pub fn new(socket: SecureHalfConnWrite<S>, receiver: mpsc::Receiver<StreamCommand>) -> Self
    {
        SecioSessionWriter {
            socket: Arc::new(Mutex::new(socket)),
            stream_receiver: receiver
        }
    }

    pub async fn window_update_frame_send(&mut self, id: StreamId, credit: u32, flag: Flag) -> Result<(),String>{
        let mut frame = Frame::window_update(id, credit);
        if  flag == Flag::Syn {
            frame.header_mut().syn();
        } else if flag == Flag::Ack {
            frame.header_mut().ack();
        }
        println!("{}: sending initial {:?}", id.val(), frame.header());
        let header = encode(&frame.header);
        (*self.socket.lock().await).send(& mut header.to_vec()).await
    }

    pub async fn data_frame_send(&mut self, stream: Stream, data: Vec<u8>) -> Result<(), String>{
 //       let stream = self.streams.get(&stream_id.val());
//        if let Some(stream) = stream {
            let mut send_frame = Frame::data(stream.id(), data)?;
            if stream.flag  == Flag::Ack {
                send_frame.header.ack();
            } else if  stream.flag == Flag::Syn {
                send_frame.header.syn();
            }
            let mut header = encode(&send_frame.header);
            (*self.socket.lock().await).send(& mut header.to_vec()).await?;
            (*self.socket.lock().await).send(& mut send_frame.body).await?;
            return Ok(())
  //      }
        //Err("stream not founded".to_string())
    }

    pub async fn frame_send(&mut self, mut frame: Frame) -> Result<(), String>{

        let mut header = encode(&frame.header);
        (*self.socket.lock().await).send(& mut header.to_vec()).await?;
        if !frame.body.is_empty() {
            (*self.socket.lock().await).send(&mut frame.body).await?;
        }
        return Ok(())
    }

    pub async fn send_process(&mut self) {
        loop {
            println!("start send process");
            let res = self.stream_receiver.next().await;
            match res {
//                Some(StreamCommand::SendFrame(frame)) => {
//                    println!("window_update_frame_send");
//                    self.window_update_frame_send(stream.id(), stream.config.receive_window, Flag::Syn).await.unwrap()
//
//                },
                Some(StreamCommand::SendFrame(frame)) => {
                    self.frame_send(frame).await;
                }
                _ => {
                    println!("receive nothing");
                },
            }
        }
    }
}


impl <S: AsyncRead + Send + Unpin + 'static>SecioSessionReader<S> {
    pub fn new(socket: SecureHalfConnRead<S>, cfg: Config, mode: Mode,  stream_sender: mpsc::Sender<StreamCommand>) -> Self
    {
        let id = Id::random();
        log::debug!("new connection: {:?} ({:?})", id.0, mode);

        //let socket = frame::Io::new(id, socket, cfg.max_buffer_size);
        SecioSessionReader {
            id,
            mode,
            config: Arc::new(cfg),
            socket: Arc::new(Mutex::new(socket)),
            streams: HashMap::new(),
            next_id: match mode {
                Mode::Client => 1,
                Mode::Server => 2
            },
            garbage: Vec::new(),
            shutdown: Shutdown::NotStarted,
            is_closed: false,
            remote_stream_sender: None,
          //  control_receiver,
            stream_sender
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


    // Check if the given stream ID is valid w.r.t. the provided tag and our connection mode.
    fn is_valid_remote_id(&self, id: StreamId, tag: Tag) -> bool {
        if tag == Tag::Ping || tag == Tag::GoAway {
            return id.is_session()
        }
        match self.mode {
            Mode::Client => id.is_server(),
            Mode::Server => id.is_client()
        }
    }

    pub async fn open_secio_stream(&mut self) -> Result<Stream, String> {
        let id = self.next_stream_id()?;

        let mut stream = {
            let config = self.config.clone();
            let window = self.config.receive_window;
            let mut stream = Stream::new(id, self.id, config, self.stream_sender.clone());
            if self.config.lazy_open {
                stream.set_flag(stream::Flag::Syn)
            }
            stream
        };

        let (data_sender, data_recerver) = mpsc::channel(10);
        if !self.config.lazy_open {
            let mut frame = Frame::window_update(stream.id(), stream.config.receive_window);
            frame.header.syn();
            self.stream_sender.send(StreamCommand::SendFrame(frame)).await;
        }

        let mut stream_session_sender = stream.clone();
        stream_session_sender.data_sender = Some(data_sender);
        self.streams.insert(stream.id().val(), stream_session_sender);
        stream.data_receiver = Some(data_recerver);
        Ok(stream)
    }

    pub fn get_stream(&mut self, stream_id: u32) -> Result<Stream, String> {

        let remote_flag = self.is_valid_remote_id(StreamId::new(stream_id), Tag::WindowUpdate);
        if remote_flag {
            let stream = self.streams.get_mut(&stream_id);
            if let Some(stream) = stream {
                let (data_sender, data_recerver) = mpsc::channel(10);


                let mut stream_session_reader = (*stream).clone();

                stream.data_sender = Some(data_sender);
                stream.cache.clear();
                stream_session_reader.data_receiver = Some(data_recerver);
                Ok(stream_session_reader)
            } else {
                Err("not exist stream".to_string())
            }
        } else {
            return Err("not remote stream id".to_string())
        }
    }

    pub async fn subscribe_stream(&mut self, mut tx: oneshot::Sender<Result<mpsc::Receiver<Stream>, String>>)  {
        let (mut stream_sender, stream_recerver) = mpsc::channel(10);
        tx.send(Ok(stream_recerver));
        self.remote_stream_sender = Some(stream_sender.clone());
        for (mut stream_id, mut stream) in self.streams.iter_mut() {
            let id = StreamId::new(*stream_id);
            let remote_id_flag =  match self.mode {
                Mode::Client => id.is_server(),
                Mode::Server => id.is_client()
            };
            if remote_id_flag {
                let (data_sender, data_recerver) = mpsc::channel(10);


                let mut stream_session_reader = (*stream).clone();

                stream.data_sender = Some(data_sender);
                stream.cache.clear();
                stream_session_reader.data_receiver = Some(data_recerver);
                stream_sender.send(stream_session_reader).await;
            }

        }

    }



    pub async fn on_window_update_frame(&mut self, frame: Frame) ->  Action {
        let stream_id = frame.header().stream_id;

        if frame.header().flags.contains(header::RST) { // stream reset
            if let Some(s) = self.streams.get_mut(&stream_id.val()) {
                //to do. reset stream
            }
            return Action::None
        }

        let is_finish = frame.header().flags.contains(header::FIN); // half-close


        if frame.header().flags.contains(header::SYN) { // new stream
            if !self.is_valid_remote_id(stream_id, frame.header.tag) {
                log::error!("{}: invalid stream id {}", self.id.0, stream_id);
                return Action::Terminate(Frame::protocol_error())
            }
            if self.streams.contains_key(&stream_id.val()) {
                log::error!("{}/{}: stream already exists", self.id.0, stream_id);
                return Action::Terminate(Frame::protocol_error())
            }
            if self.streams.len() == self.config.max_num_streams {
                log::error!("{}: maximum number of streams reached", self.id.0);
                return Action::Terminate(Frame::protocol_error())
            }
            let mut stream = {
                let config = self.config.clone();
                //let sender = self.stream_sender.clone();
                let mut stream = Stream::new(stream_id, self.id, config, self.stream_sender.clone());
                stream.set_flag(stream::Flag::Ack);
                stream
            };
            if is_finish {
               // stream.shared().update_state(self.id, stream_id, State::RecvClosed);
            }
            if self.remote_stream_sender.is_some() {
                let (data_sender, data_recerver) = mpsc::channel(10);


                let mut stream_session_reader = stream.clone();

                stream.data_sender = Some(data_sender);
                stream_session_reader.data_receiver = Some(data_recerver);
                self.remote_stream_sender.clone().unwrap().send(stream_session_reader).await;
            }
            let stream_clone = stream.clone();
            self.streams.insert(stream_id.val(), stream);
            return Action::New(stream_clone)
        }

        if let Some(stream) = self.streams.get(&stream_id.val()) {
//            let mut shared = stream.shared();
//            shared.credit += frame.header().credit();
//            if is_finish {
//                shared.update_state(self.id, stream_id, State::RecvClosed);
//            }
//            if let Some(w) = shared.writer.take() {
//                w.wake()
//            }
        } else if !is_finish {
            log::debug!("{}/{}: window update for unknown stream", self.id.0, stream_id);
            let mut header = Header::data(stream_id, 0);
            header.rst();
            return Action::Reset(Frame::new(header))
        }

        Action::None
    }

    pub fn on_ping_frame(&mut self, frame: Frame) ->  Action {
        let stream_id = frame.header().stream_id;
        if frame.header().flags.contains(header::ACK) { // pong
            return Action::None
        }

        if stream_id == CONNECTION_ID || self.streams.contains_key(&stream_id.val()) {
            let mut hdr = Header::ping(frame.header().nonce());
            hdr.ack();
            return Action::Ping(Frame::new(hdr))
        }
        log::debug!("{}/{}: ping for unknown stream", self.id.0, stream_id);
        let mut header = Header::data(stream_id, 0);
        header.rst();
        Action::Reset(Frame::new(header))
    }

    async fn on_data(&mut self, frame: Frame) -> Action {
        let stream_id = frame.header().stream_id;

        if frame.header().flags.contains(header::RST) { // stream reset
            if let Some(s) = self.streams.get_mut(&stream_id.val()) {
                // let mut shared = s.shared();
                // shared.update_state(self.id, stream_id, State::Closed);
                // if let Some(w) = shared.reader.take() {
                //     w.wake()
                // }
                // if let Some(w) = shared.writer.take() {
                //     w.wake()
                // }
            }
            return Action::None
        }

        let is_finish = frame.header().flags.contains(header::FIN); // half-close

        if frame.header().flags.contains(header::SYN) { // new stream
            if !self.is_valid_remote_id(stream_id, Tag::Data) {
                log::error!("{}: invalid stream id {}", self.id.0, stream_id);
                return Action::Terminate(Frame::protocol_error())
            }
            if frame.body().len() > DEFAULT_CREDIT as usize {
                log::error!("{}/{}: 1st body of stream exceeds default credit", self.id.0, stream_id);
                return Action::Terminate(Frame::protocol_error())
            }
            if self.streams.contains_key(&stream_id.val()) {
                log::error!("{}/{}: stream already exists", self.id.0, stream_id);
                return Action::Terminate(Frame::protocol_error())
            }
            if self.streams.len() == self.config.max_num_streams {
                log::error!("{}: maximum number of streams reached", self.id.0);
                return Action::Terminate(Frame::internal_error())
            }
            let stream = {
                let config = self.config.clone();
                let credit = DEFAULT_CREDIT;
                let sender = self.stream_sender.clone();
                let mut stream = Stream::new(stream_id, self.id, config, self.stream_sender.clone());
                stream.set_flag(stream::Flag::Ack);
                stream
            };
            // {
            //     let mut shared = stream.shared();
            //     if is_finish {
            //         shared.update_state(self.id, stream_id, State::RecvClosed);
            //     }
            //     shared.window = shared.window.saturating_sub(frame.body_len());
            //     shared.buffer.push(frame.into_body());
            // }
            self.streams.insert(stream_id.val(), stream.clone());
            return Action::New(stream)
        }

        if let Some(stream) = self.streams.get_mut(&stream_id.val()) {
            // let mut shared = stream.shared();
            // if frame.body().len() > shared.window as usize {
            //     log::error!("{}/{}: frame body larger than window of stream", self.id, stream_id);
            //     return Action::Terminate(Frame::protocol_error())
            // }
            if is_finish {
                // shared.update_state(self.id, stream_id, State::RecvClosed);
            }
            let max_buffer_size = self.config.max_buffer_size;
            if stream.data_sender.is_some() {
                if !stream.cache.is_empty() {
                    stream.data_sender.as_mut().unwrap().send(stream.cache.clone()).await;
                    stream.cache.clear();
                }
                stream.data_sender.as_mut().unwrap().send(frame.header.length.to_be_bytes().to_vec()).await;
                stream.data_sender.as_mut().unwrap().send(frame.into_body()).await;
            } else {
                stream.cache.append(& mut frame.header.length.to_be_bytes().to_vec());
                stream.cache.append(& mut frame.into_body());
            }
            // if shared.buffer.len().map(move |n| n >= max_buffer_size).unwrap_or(true) {
            //     log::error!("{}/{}: buffer of stream grows beyond limit", self.id, stream_id);
            //     let mut header = Header::data(stream_id, 0);
            //     header.rst();
            //     return Action::Reset(Frame::new(header))
            // }
            // shared.window = shared.window.saturating_sub(frame.body_len());
            // shared.buffer.push(frame.into_body());
            // if let Some(w) = shared.reader.take() {
            //     w.wake()
            // }
            // if !is_finish
            //     && shared.window == 0
            //     && self.config.window_update_mode == WindowUpdateMode::OnReceive
            // {
            //     shared.window = self.config.receive_window;
            //     let frame = Frame::window_update(stream_id, self.config.receive_window);
            //     return Action::Update(frame)
            // }
        } else if !is_finish {
            log::debug!("{}/{}: data for unknown stream", self.id.0, stream_id);
            let mut header = Header::data(stream_id, 0);
            header.rst();
            return Action::Reset(Frame::new(header))
        }

        Action::None
    }

    pub async fn on_frame(& mut self, frame: Frame) {
        let action = match frame.header.tag {
            Tag::Ping => {
                self.on_ping_frame(frame)
            }
            Tag::GoAway=> {
                Action::None
            }
            Tag::WindowUpdate=> {
                self.on_window_update_frame(frame).await
            }
            Tag::Data => {
                self.on_data(frame).await
            }
        };
        match action {
            Action::None => {}
            Action::New(stream) => {
                println!("{}: new inbound {} of ", self.id.0, stream.id);
                //return Ok(Some(stream))
            }
            Action::Update(f) => {
                log::trace!("{}/{}: sending update", self.id.0, f.header.stream_id);
                //self.socket.get_mut().send(&f).await.or(Err(ConnectionError::Closed))?
            }
            Action::Ping(f) => {
                log::trace!("{}/{}: pong", self.id.0, f.header.stream_id);
                //let header = encode(&f.header);
                let res = self.stream_sender.send(StreamCommand::SendFrame(f)).await;
                // self.socket.get_mut().send(&f).await.or(Err(ConnectionError::Closed))?
            }
            Action::Reset(f) => {
                log::trace!("{}/{}: sending reset", self.id.0, f.header().stream_id);
                // self.socket.get_mut().send(&f).await.or(Err(ConnectionError::Closed))?
            }
            Action::Terminate(f) => {
                log::trace!("{}: sending term", self.id.0);
                // self.socket.get_mut().send(&f).await.or(Err(ConnectionError::Closed))?
            }
        }

    }

//     pub async fn select_receive_deal(&mut self,  control_receiver: &mpsc::Receiver<ControlCommand>) -> Either<Result<Frame ,String>, Option<ControlCommand>>{
//         let mut receiver = self.control_receiver.next();
//         let remote_frame_future = self.receive_frame().fuse();
//
//
//         let res = select! {
//                         res = remote_frame_future => {
//                             println!("received frame");
//                             Either::Left(res)
////                             if let Ok(frame) = res {
////                                 self.on_frame(frame).await;
////                                 println!("deal frame");
////                             }
//                         },
//                         res = receiver => {
//                            println!("task two completed first");
//                            Either::Right(res)
////                            match res {
////                                Some(ControlCommand::OpenStream(rx)) => {
////                                    let open_stream = self.open_secio_stream().await;
////                                    if open_stream.is_ok() {
////                                        rx.send(open_stream);
////                                    }
////                                }
////                                _ => (),
////                            }
//                         },
//                  };
//         res
//
//     }


    pub async fn receive_loop(&mut self, mut control_receiver: mpsc::Receiver<ControlCommand>) {
//        let receiver = control_receiver.next().await;
//        match receiver {
//            Some(ControlCommand::OpenStream(mut rx)) => {
//                let res = self.open_secio_stream().await;
//                if res.is_ok() {
//                    rx.send(res);
//                }
//
//            },
//            _ => (),
//        }
    loop {

//            let remote_frame_future =  self.receive_frame();
//            let res = remote_frame_future.await;
//            if let Ok(frame) = res {
//                self.on_frame(frame).await;
//                println!("deal frame");
//            }


       //     let res = self.select_receive_deal(&control_receiver).await;
        let mut receiver = control_receiver.next();
        let remote_frame_future = receive_frame(self.socket.clone()).fuse();
        pin_mut!(remote_frame_future, receiver);

        let res = select! {
        //select! {
                         res = remote_frame_future => {
                             println!("received frame");
                             Either::Left(res)
//                              if let Ok(frame) = res {
//                                  self.on_frame(frame).await;
//                                  println!("deal frame");
//                              }
                         },
                         res = receiver => {
                            println!("task two completed first");
                            Either::Right(res)
                            // match res {
                            //     Some(ControlCommand::OpenStream(tx)) => {
                            //         let open_stream = self.open_secio_stream().await;
                            //         if open_stream.is_ok() {
                            //             tx.send(open_stream);
                            //         }
                            //     }
                            //     _ => (),
                            // }
                         },
                  };

           match res {
               Either::Left(res) => {
                   if let Ok(frame) = res {
                       self.on_frame(frame).await;
                       println!("deal frame");
                   }
               },
               Either::Right(res) => {
                   match res {
                       Some(ControlCommand::OpenStream(mut rx)) => {
                           let open_stream = self.open_secio_stream().await;
                           rx.send(open_stream);

                       },
                       Some(ControlCommand::GetStream((stream_id, mut tx))) => {
                           let stream = self.get_stream(stream_id);
                           tx.send(stream);


                       },
                       Some(ControlCommand::SubscribeStream(( mut tx))) => {
                           self.subscribe_stream(tx).await;
                       },
                       _ => (),
                   }
               },
               _ => {println!("select res:{:?}", res)},
           }

        }
    }
}


/// Open a new stream to the remote.
pub async fn open_stream(mut sender: mpsc::Sender<ControlCommand>) -> Result<Stream, String> {
    let (tx, mut rx) = oneshot::channel();
    let res = sender.send(ControlCommand::OpenStream(tx)).await;
    if let Ok(res) = res {
        let res = rx.await;
        if let Ok(res) = res {
            return res
        }
    }
    Err("fail".to_string())
}


/// get a new stream to the remote.
pub async fn get_stream(index:u32, mut sender: mpsc::Sender<ControlCommand>) -> Result<Stream, String> {
    let (tx, mut rx) = oneshot::channel();
    let res = sender.send(ControlCommand::GetStream((index, tx))).await;
    if let Ok(res) = res {
        let res = rx.await;
        if let Ok(res) = res {
            return res
        }
    }
    Err("fail".to_string())
}

/// subscribe a new stream to the remote.
pub async fn subscribe_stream(mut sender: mpsc::Sender<ControlCommand>) -> Result<mpsc::Receiver<Stream>, String> {
    let (tx, mut rx) = oneshot::channel();
    let res = sender.send(ControlCommand::SubscribeStream(tx)).await;
    if let Ok(res) = res {
        let res = rx.await;
        if let Ok(res) = res {
            return res
        }
    }
    Err("fail".to_string())
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
        let (control_sender, control_receiver) = mpsc::channel(10);
        let (stream_sender, stream_receiver) = mpsc::channel(10);
        let mut res = handshake(connec, config).await;
        if let Ok((mut secure_conn_writer, mut secure_conn_reader )) = res {
            let mut session_reader = SecioSessionReader::new(secure_conn_reader, Config::default(), Mode::Client,  stream_sender);
            let mut session_writer = SecioSessionWriter::new(secure_conn_writer, stream_receiver);
             let res = session_reader.open_secio_stream().await;


            if let Ok(id) = res {
                session_writer.data_frame_send(id , "hello yamux".to_string().into_bytes()).await;
                let mut msg = "ok".to_string();
                println!("open_stream success");
            } else {
                println!("open_stream fail" );
            }
            // remote_frame_future.await;
        //    let receive_local_frame_future = session.send_frame();
        //   let broker = async_std::task::spawn(receive_local_frame_future);

            session_reader.receive_loop(control_receiver).await;

           // broker.await;
        }
    });
}

pub async fn period_send( sender: mpsc::Sender<ControlCommand>) {
    let res = open_stream(sender).await;
    let mut index = 0;
    if let Ok(mut stream) = res {
        let mut stream_clone = stream.clone();
        let mut data_receiver = stream.data_receiver.unwrap();
        task::spawn(async move {
            loop {
                let buf = data_receiver.next().await;
                println!("receive:{:?}", buf);
                if !buf.is_some(){
                    break;
                }

            }
        });
        loop {
            index +=1;
            let frame = Frame::data(stream_clone.id(), format!("love and peace:{}", index).into_bytes()).unwrap();
            stream_clone.sender.send(StreamCommand::SendFrame(frame)).await;
            task::sleep(Duration::from_secs(10)).await;

        }
    } else {
        println!("fail open stream");
    }
}

pub async fn remote_stream_deal(mut sender: mpsc::Sender<ControlCommand>) {
    loop {
        let res = get_stream(2, sender.clone()).await;
        if let Ok(stream)= res{
            if !stream.cache.is_empty(){
                println!("get stream cache:{:?}", stream.cache);
            }
            let mut data_receiver = stream.data_receiver.unwrap();
            task::spawn(async move {
                loop {
                    let buf = data_receiver.next().await;
                    println!("remote send receive:{:?}", buf);
                }
            });
            break;
        } else {
            println!("get_stream fail :{:?}", res);
        }
        task::sleep(Duration::from_secs(10)).await;

    }
}


pub async fn remote_stream_subscribe(mut sender: mpsc::Sender<ControlCommand>) {
    let res = subscribe_stream( sender.clone()).await;
    if let Ok(mut remote_stream_receiver)= res{
        loop {


            let stream = remote_stream_receiver.next().await.unwrap();
            if !stream.cache.is_empty(){
                println!("get stream cache:{:?}", stream.cache);
            }
            let mut data_receiver = stream.data_receiver.unwrap();
            task::spawn(async move {
                loop {
                    let buf = data_receiver.next().await;
                    println!("remote send receive:{:?}", buf);
                }
            });
           // break;
        }
        task::sleep(Duration::from_secs(10)).await;

    } else {
        println!("subscribe_stream fail :{:?}", res);
    }
}


#[test]
fn yamux_secio_client_process_test() {
    async_std::task::block_on(async move {
        let connec = async_std::net::TcpStream::connect("127.0.0.1:8981").await.unwrap();
        let key1 = Keypair::generate_ed25519();
        let mut config = SecioConfig::new(key1);
        let (control_sender, control_receiver) = mpsc::channel(10);
        let (stream_sender, stream_receiver) = mpsc::channel(10);

        let mut res = handshake(connec, config).await;
        if let Ok((mut secure_conn_writer, mut secure_conn_reader )) = res {
            let mut session_reader = SecioSessionReader::new(secure_conn_reader, Config::default(), Mode::Client, stream_sender.clone());
            let mut session_writer = SecioSessionWriter::new(secure_conn_writer, stream_receiver);
            let deal_remote_stream = remote_stream_subscribe(control_sender.clone());
            let period_send = period_send( control_sender);
            let receive_process = session_reader.receive_loop( control_receiver);
            let send_process = session_writer.send_process();
            join!{receive_process, send_process, period_send, deal_remote_stream};
            // broker.await;
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