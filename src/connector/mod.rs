pub mod sync;

#[cfg(feature = "async")]
mod async_impl;

#[cfg(feature = "async")]
pub use async_impl::*;

use std::fmt;
use std::io;

pub struct Connector<P>
{
    port: P,
}

impl<P> Connector<P>{
    /// Create a new Connector from an already opened SerialPort.
    pub fn new(port: P) -> Self {
        Connector { port }
    }
}



#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum WorkingArea {
    China900Mhz,
    China800Mhz,
    US,
    EU,
    Korea,
}

#[derive(Debug)]
pub enum ConnectorError {
    Io(io::Error),
    Timeout,
    InvalidWorkingArea,
    NoPacketReceived,
    FailedSetting(String),
    SerialRead(String),
    ErrorStopMultiPolling(String),
}

impl fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectorError::Io(e) => write!(f, "IO error: {}", e),
            ConnectorError::Timeout => write!(f, "Timeout"),
            ConnectorError::InvalidWorkingArea => write!(f, "Invalid working area"),
            ConnectorError::NoPacketReceived => write!(f, "No packet received"),
            ConnectorError::SerialRead(msg) => write!(f, "Serial read error: {}", msg),
            ConnectorError::FailedSetting(msg) => write!(f, "Failed Setting: {}", msg),
            ConnectorError::ErrorStopMultiPolling(msg) => {
                write!(f, "Impossible to stop multiple polling [{msg}]")
            }
        }
    }
}

impl std::error::Error for ConnectorError {}

impl From<io::Error> for ConnectorError {
    fn from(err: io::Error) -> Self {
        ConnectorError::Io(err)
    }
}

pub(crate) fn clear_non_ascii(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii()).collect()
}

pub(crate) fn hexdump_line(prefix: &str, data: &[u8]) {
    let mut out = String::new();
    for b in data {
        out.push_str(format!("{:02X} ", b).as_str());
    }
    log::debug!("{} {}", prefix, out);
}
