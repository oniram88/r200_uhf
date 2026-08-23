use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Frame(#[from] FrameError),

    #[error("command error: {0}")]
    Command(#[from] CommandError),

    #[error("invalid payload: {0}")]
    InvalidPayload(String),
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame too short ({0} bytes, minimum 7)")]
    TooShort(usize),

    #[error("bad header byte: expected 0xAA, got 0x{0:02X}")]
    BadHeader(u8),

    #[error("unknown frame type: 0x{0:02X}")]
    UnknownFrameType(u8),

    #[error("checksum mismatch: expected 0x{expected:02X}, got 0x{actual:02X}")]
    ChecksumMismatch { expected: u8, actual: u8 },

    #[error("truncated frame: declared length {declared} but got {available} data bytes")]
    Truncated { declared: usize, available: usize },

    #[error("missing frame end marker: expected 0xDD, got 0x{0:02X}")]
    MissingEndMarker(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandError(pub u8);

impl CommandError {
    pub const SUCCESS: Self = Self(0x00);

    #[must_use]
    pub const fn is_success(self) -> bool {
        self.0 == 0x00
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            0x00 => write!(f, "success"),
            0x09 => write!(f, "read failure"),
            0x10 => write!(f, "write failure"),
            0x12 => write!(f, "kill failure"),
            0x13 => write!(f, "lock failure"),
            0x14 => write!(f, "block permalock failure"),
            0x15 => write!(f, "inventory failure"),
            0x16 => write!(f, "access failure"),
            0x17 => write!(f, "command error"),
            0x80 => write!(f, "not initialized"),
            b @ 0xA0..=0xEF => write!(f, "tag access error (0x{b:02X})"),
            other => write!(f, "unknown error (0x{other:02X})"),
        }
    }
}

impl std::error::Error for CommandError {}
