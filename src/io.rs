use crate::header;

#[derive(Clone, Debug, PartialEq, Eq)]
/// The stages of reading a new `Frame`.
pub enum ReadState {
    /// Initial reading state.
    Init,
    /// Reading the frame header.
    Header {
        offset: usize,
        buffer: [u8; header::HEADER_SIZE]
    },
    /// Reading the frame body.
    Body {
        header: header::Header,
        offset: usize,
        buffer: Vec<u8>
    }
}
