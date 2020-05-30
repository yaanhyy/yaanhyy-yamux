use crate::header;

/// The stages of reading a new `Frame`.
enum ReadState {
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
