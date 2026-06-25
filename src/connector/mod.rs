pub mod sync;

#[cfg(feature = "async")]
mod async_impl;

#[cfg(feature = "async")]
pub use async_impl::*;

use crate::packet::Packet;
use std::fmt;
use std::io;

pub struct Connector<P> {
    port: P,
}

impl<P> Connector<P> {
    /// Create a new Connector from an already opened SerialPort.
    pub fn new(port: P) -> Self {
        Connector { port }
    }

    fn parse_to_working_area(p: Packet) -> Result<WorkingArea, ConnectorError> {
        match p.get_data()[0] {
            0 => Ok(WorkingArea::China900Mhz),
            1 => Ok(WorkingArea::China800Mhz),
            2 => Ok(WorkingArea::US),
            3 => Ok(WorkingArea::EU),
            4 => Ok(WorkingArea::Korea),
            _ => Err(ConnectorError::InvalidWorkingArea),
        }
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

impl WorkingArea {
    pub fn packet_to_64(&self, p: Packet) -> f64 {
        match self {
            WorkingArea::China900Mhz => return (p.get_data()[0] as f64) * 0.25 + 920.125,
            WorkingArea::China800Mhz => return (p.get_data()[0] as f64) * 0.25 + 840.125,
            WorkingArea::US => return (p.get_data()[0] as f64) * 0.50 + 902.25,
            WorkingArea::EU => return (p.get_data()[0] as f64) * 0.2 + 865.1,
            WorkingArea::Korea => return (p.get_data()[0] as f64) * 0.2 + 917.1,
        }
    }
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

pub(crate) fn calculate_transmit_power(p:Packet) -> f64{
    let data = p.get_data();
    ((data[0] as u16) * 256 + (data[1] as u16)) as f64 / 100.0
}