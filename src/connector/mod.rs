pub mod sync;

#[cfg(feature = "async")]
mod async_impl;

#[cfg(feature = "async")]
pub use async_impl::*;

use crate::Rfid;
use crate::packet::Packet;
use log::{debug, error, info};
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
        let data = p.get_data();
        if data.is_empty() {
            return Err(ConnectorError::InvalidResponse(
                "Empty working area response".into(),
            ));
        }
        match data[0] {
            0 => Ok(WorkingArea::China900Mhz),
            1 => Ok(WorkingArea::China800Mhz),
            2 => Ok(WorkingArea::US),
            3 => Ok(WorkingArea::EU),
            4 => Ok(WorkingArea::Korea),
            _ => Err(ConnectorError::InvalidWorkingArea),
        }
    }

    fn _set_transmission_power(p: Option<Packet>, power: f64) -> Result<(), ConnectorError> {
        if let Some(p) = p {
            let data = p.get_data();
            if data.is_empty() {
                return Err(ConnectorError::InvalidResponse(
                    "Empty set-power ACK".into(),
                ));
            }
            if data[0] == 0x00 {
                info!("Power correct set to {}", power);
                return Ok(());
            } else {
                error!("Power not set to {}", power);
                return Err(ConnectorError::FailedSetting(format!(
                    "Transmission power not set to {}",
                    power
                )));
            }
        }
        Err(ConnectorError::NoPacketReceived)
    }

    fn parse_rfid_packets(
        &self,
        response: Option<Vec<Packet>>,
    ) -> Result<Vec<Rfid>, ConnectorError> {
        let mut rfids = Vec::new();
        if let Some(ps) = response {
            if ps.len() == 1 && ps[0].get_data().first() == Some(&0x15) {
                debug!("No tags present");
            } else {
                for p in ps {
                    let data = p.get_data();
                    if data.len() == 17 {
                        rfids.push(Rfid::from_raw(data));
                    }
                }
            }
        }
        Ok(rfids)
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
        let data = p.get_data();
        if data.is_empty() {
            return 0.0;
        }
        match self {
            WorkingArea::China900Mhz => return (data[0] as f64) * 0.25 + 920.125,
            WorkingArea::China800Mhz => return (data[0] as f64) * 0.25 + 840.125,
            WorkingArea::US => return (data[0] as f64) * 0.50 + 902.25,
            WorkingArea::EU => return (data[0] as f64) * 0.2 + 865.1,
            WorkingArea::Korea => return (data[0] as f64) * 0.2 + 917.1,
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
    InvalidResponse(String),
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
            ConnectorError::InvalidResponse(msg) => write!(f, "Invalid response: {}", msg),
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

pub(crate) fn calculate_transmit_power(p: Packet) -> Result<f64, ConnectorError> {
    let data = p.get_data();
    if data.len() >= 2 {
        Ok(((data[0] as u16) * 256 + (data[1] as u16)) as f64 / 100.0)
    } else if data.len() == 1 {
        Ok(data[0] as f64)
    } else {
        Err(ConnectorError::InvalidResponse(
            "Empty power response".into(),
        ))
    }
}
