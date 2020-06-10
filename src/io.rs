use crate::header::{Header, HEADER_SIZE, decode, Tag};
use std::sync::{Arc};
use async_std::sync::Mutex;
use yaanhyy_secio::codec::{SecureHalfConnWrite, SecureHalfConnRead};
use futures::{AsyncRead, AsyncWrite};
use crate::frame::Frame;
#[derive(Clone, Debug, PartialEq, Eq)]
/// The stages of reading a new `Frame`.
pub enum ReadState {
    /// Initial reading state.
    Init,
    /// Reading the frame header.
    Header {
        offset: usize,
        buffer: [u8; HEADER_SIZE]
    },
    /// Reading the frame body.
    Body {
        header: Header,
        offset: usize,
        buffer: Vec<u8>
    }
}



pub async fn receive_frame<R>(socket: Arc<Mutex<SecureHalfConnRead<R>>>) -> Result<Frame ,String>
where R:  AsyncRead + Send + Unpin + 'static
{
    let mut state =  ReadState::Init;
    loop {
        match state {
            ReadState::Init => {
                state = ReadState::Header {
                    offset: 0,
                    buffer: [0; HEADER_SIZE]
                };
            },
            ReadState::Header{ref mut offset, ref mut buffer} => {
                let socket = socket.lock();
                let mut read_buf = (*socket.await).read().await?;
                println!("header buffer:{:?}", read_buf);
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
                                state = ReadState::Init;
                                return Ok(Frame{header, body: rest});
                            } else if (rest.len() as u32) < frame_len {
                                state = ReadState::Body { header, offset: rest.len(), buffer: rest};
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
                                state = ReadState::Init;
                            }
                            return Ok(Frame::new(header));
                        },
                    }
                }
            }
            ReadState::Body {ref header, ref mut offset, ref mut buffer} => {
                let socket = socket.lock();
                let mut read_buf = (*socket.await).read().await?;

                if read_buf.len()  == (header.length  as usize - *offset) {
                    let h = header.clone();
                    buffer.append(& mut read_buf);
                    let buffer_clone = buffer.clone();
                    state = ReadState::Init;
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