pub mod header;
pub mod frame;
pub mod session;
pub mod stream;
pub mod io;

const DEFAULT_CREDIT: u32 = 256 * 1024; // as per yamux specification

/// Specifies when window update frames are sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowUpdateMode {
    /// Send window updates as soon as a [`Stream`]'s receive window drops to 0.
    ///
    /// This ensures that the sender can resume sending more data as soon as possible
    /// but a slow reader on the receiving side may be overwhelmed, i.e. it accumulates
    /// data in its buffer which may reach its limit (see `set_max_buffer_size`).
    /// In this mode, window updates merely prevent head of line blocking but do not
    /// effectively exercise back pressure on senders.
    OnReceive,

    /// Send window updates only when data is read on the receiving end.
    ///
    /// This ensures that senders do not overwhelm receivers and keeps buffer usage
    /// low. However, depending on the protocol, there is a risk of deadlock, namely
    /// if both endpoints want to send data larger than the receivers window and they
    /// do not read before finishing their writes. Use this mode only if you are sure
    /// that this will never happen, i.e. if
    ///
    /// - Endpoints *A* and *B* never write at the same time, *or*
    /// - Endpoints *A* and *B* write at most *n* frames concurrently such that the sum
    ///   of the frame lengths is less or equal to the available credit of *A* and *B*
    ///   respectively.
    OnRead
}

/// Yamux configuration.
///
/// The default configuration values are as follows:
///
/// - receive window = 256 KiB
/// - max. buffer size (per stream) = 1 MiB
/// - max. number of streams = 8192
/// - window update mode = on receive
/// - read after close = true
/// - lazy open = false
#[derive(Debug, Clone)]
pub struct Config {
    receive_window: u32,
    max_buffer_size: usize,
    max_num_streams: usize,
    window_update_mode: WindowUpdateMode,
    read_after_close: bool,
    lazy_open: bool
}

impl Default for Config {
    fn default() -> Self {
        Config {
            receive_window: DEFAULT_CREDIT,
            max_buffer_size: 1024 * 1024,
            max_num_streams: 8192,
            window_update_mode: WindowUpdateMode::OnReceive,
            read_after_close: true,
            lazy_open: false
        }
    }
}