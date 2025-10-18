use crate::frame::{Command, Frame, R200_FRAME_END, R200_FRAME_HEADER};
use crate::packet::Packet;
use crate::rfid::Rfid;
use log::{debug, error, info};
use serialport::SerialPort;
use std::fmt;
use std::io;

#[derive(Debug)]
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
    SerialRead(String),
}

impl fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectorError::Io(e) => write!(f, "IO error: {}", e),
            ConnectorError::Timeout => write!(f, "Timeout"),
            ConnectorError::InvalidWorkingArea => write!(f, "Invalid working area"),
            ConnectorError::NoPacketReceived => write!(f, "No packet received"),
            ConnectorError::SerialRead(msg) => write!(f, "Serial read error: {}", msg),
        }
    }
}

impl std::error::Error for ConnectorError {}

impl From<io::Error> for ConnectorError {
    fn from(err: io::Error) -> Self {
        ConnectorError::Io(err)
    }
}

pub struct Connector {
    port: Box<dyn SerialPort>,
}

impl Connector {
    /// Create a new Connector from an already opened SerialPort.
    ///
    /// Parameters
    /// - p0: A boxed serialport::SerialPort already configured (baud rate, timeout, etc.)
    ///
    /// Returns
    /// A Connector instance bound to the given serial port.
    pub fn new(p0: Box<dyn SerialPort>) -> Self {
        Connector { port: p0 }
    }

    pub fn get_module_info(&mut self) -> Result<String, ConnectorError> {
        self.send_packet(Command::HardwareVersion)?;
        let hardware = self.single_read_from_serial();
        self.send_packet(Command::SoftwareVersion)?;
        let software = self.single_read_from_serial();
        self.send_packet(Command::Manufacturer)?;
        let manufacture = self.single_read_from_serial();

        let out = format!(
            "Hardware: {} - Software: {} - Manufacturer: {}",
            hardware?.unwrap().to_string(),
            software?.unwrap().to_string(),
            manufacture?.unwrap().to_string()
        );

        Ok(out)
    }

    /// Builds and sends the command
    fn send_packet(&mut self, command: Command) -> Result<(), ConnectorError> {
        let frame = Frame::new(&command).to_bytes();

        let mut out = String::new();
        for b in &frame {
            out.push_str(format!("{:02X} ", b).as_str());
        }
        debug!("[TX] {out} - [{command}]");

        self.port.write_all(&frame)?;
        self.port.flush()?;
        Ok(())
    }

    fn single_read_from_serial(&mut self) -> Result<Option<Packet>, ConnectorError> {
        let out = self.read_from_serial(Some(1))?;
        Ok(out.unwrap_or(vec![]).pop())
    }

    fn read_from_serial(
        &mut self,
        num_expected_responses: Option<u32>,
    ) -> Result<Option<Vec<Packet>>, ConnectorError> {
        let mut read_buf: [u8; 1024] = [0u8; 1024];
        let mut rolling: Vec<u8> = Vec::with_capacity(4096);

        let mut output: Vec<Packet> = Vec::new();

        loop {
            let raw_data_size = self.port.read(&mut read_buf);
            debug!("raw_data_size: {:?}", raw_data_size);
            debug!("rolling: {:?}", rolling);
            match raw_data_size {
                Ok(n) if n > 0 => {
                    rolling.extend_from_slice(&read_buf[..n]);

                    debug!("rolling: {:?}", rolling);

                    // print raw for debug
                    hexdump_line("[RAW] ", &rolling);

                    if !rolling.contains(&R200_FRAME_HEADER) {
                        rolling.clear();
                        continue;
                    }
                    if !rolling.contains(&R200_FRAME_END) {
                        continue;
                    }

                    let first_frame_index = rolling
                        .iter()
                        .position(|&x| x == R200_FRAME_HEADER)
                        .unwrap();
                    let last_frame_index =
                        rolling.iter().position(|&x| x == R200_FRAME_END).unwrap();

                    let chunk = &rolling[first_frame_index..last_frame_index + 1];

                    if chunk.len() > 4
                        && chunk[0] == R200_FRAME_HEADER
                        && chunk.last() == Some(&R200_FRAME_END)
                    {
                        // Extract type, command, and data
                        let p = Packet::new(Vec::from(chunk));

                        if !p.get_data().is_empty() {
                            debug!("{}", p.debug());
                            output.push(p);
                            if output.len() >= num_expected_responses.unwrap_or(100000) as usize {
                                return Ok(Some(output));
                            }
                        }
                    }

                    rolling.drain(..last_frame_index + 1);

                    if rolling.len() > 8192 {
                        rolling.drain(..rolling.len() - 4096);
                    }
                }
                Ok(_) => {
                    // n == 0, nothing
                    return Ok(None);
                }
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    // timeout: continue and read again
                    if output.is_empty() {
                        return Err(ConnectorError::Timeout);
                    }
                    break;
                }
                Err(ref e) => {
                    error!("Serial read error: {}", e);
                    return Err(ConnectorError::SerialRead(e.to_string()));
                }
            }
        }
        Ok(Some(output))
    }

    /// Get the current regulatory working area configured on the device.
    ///
    /// Returns
    /// - Ok(WorkingArea) with the region inferred from the device response.
    /// - Err(ConnectorError::InvalidWorkingArea) if the response contains an unknown code.
    /// - Err(ConnectorError::NoPacketReceived) if nothing is received.
    /// - Other ConnectorError variants on I/O failure or timeout.
    pub fn get_working_area(&mut self) -> Result<WorkingArea, ConnectorError> {
        self.send_packet(Command::GetWorkingArea)?;
        let p = self.single_read_from_serial()?;
        if let Some(p) = p {
            return match p.get_data()[0] {
                0 => Ok(WorkingArea::China900Mhz),
                1 => Ok(WorkingArea::China800Mhz),
                2 => Ok(WorkingArea::US),
                3 => Ok(WorkingArea::EU),
                4 => Ok(WorkingArea::Korea),
                _ => Err(ConnectorError::InvalidWorkingArea),
            };
        }
        Err(ConnectorError::NoPacketReceived)
    }

    /// Get the current working RF channel as a frequency in MHz.
    ///
    /// The raw channel index returned by the device is converted to MHz based on
    /// the configured WorkingArea. Different regions use different spacing and base frequencies.
    ///
    /// Returns
    /// - Ok(f64) with the center frequency in MHz.
    /// - Err(ConnectorError::NoPacketReceived) if no response is obtained.
    /// - Other ConnectorError variants on I/O failure, timeout, or unknown working area.
    pub fn get_working_channel(&mut self) -> Result<f64, ConnectorError> {
        self.send_packet(Command::GetWorkingChannel)?;
        let p = self.single_read_from_serial()?;
        if let Some(p) = p {
            match self.get_working_area()? {
                WorkingArea::China900Mhz => {
                    return Ok((p.get_data()[0] as f64) * 0.25 + 920.125);
                }
                WorkingArea::China800Mhz => {
                    return Ok((p.get_data()[0] as f64) * 0.25 + 840.125);
                }
                WorkingArea::US => {
                    return Ok((p.get_data()[0] as f64) * 0.50 + 902.25);
                }
                WorkingArea::EU => {
                    return Ok((p.get_data()[0] as f64) * 0.2 + 865.1);
                }
                WorkingArea::Korea => {
                    return Ok((p.get_data()[0] as f64) * 0.2 + 917.1);
                }
            }
        }
        Err(ConnectorError::NoPacketReceived)
    }

    /// Read the current transmit power reported by the device.
    ///
    /// The device returns two bytes that represent the power value scaled by 100.
    /// This method combines them and returns the value as f64.
    ///
    /// Returns
    /// - Ok(f64) with the transmit power (device-specific units, typically dBm).
    /// - Err(ConnectorError::NoPacketReceived) if no response is obtained.
    /// - Other ConnectorError variants on I/O failure or timeout.
    pub fn get_transmit_power(&mut self) -> Result<f64, ConnectorError> {
        self.send_packet(Command::AcquireTransmitPower)?;
        let p = self.single_read_from_serial()?;
        if let Some(p) = p {
            let data = p.get_data();
            return Ok(((data[0] as u16) * 16 * 16 + (data[1] as u16)) as f64 / 100.0);
        }
        Err(ConnectorError::NoPacketReceived)
    }

    /// Set the transmitter output power.
    ///
    /// Parameters
    /// - power: Desired transmit power in device-specific units (typically dBm).
    ///
    /// Returns
    /// - Ok(()) when the device acknowledges the setting.
    /// - Err(ConnectorError::NoPacketReceived) if no response is obtained.
    /// - Other ConnectorError variants on I/O failure or timeout.
    pub fn set_trasmission_power(&mut self, power: f64) -> Result<(), ConnectorError> {
        self.send_packet(Command::SetTrasmissionPower(power))?;
        let p = self.single_read_from_serial()?;
        if let Some(p) = p {
            let data = p.get_data();
            if data[0] == 0x00 {
                info!("Power correct set to {}", power);
                return Ok(());
            }
        }
        Err(ConnectorError::NoPacketReceived)
    }

    /// Perform a single inventory (poll) and return the list of detected tags.
    ///
    /// Sends a SinglePollingInstruction to the reader and parses all returned packets
    /// into a collection of Rfid records containing RSSI, PC, EPC (UID) and CRC.
    ///
    /// Returns
    /// - Ok(Vec<Rfid>) possibly empty if no tags are present.
    /// - Err(ConnectorError::Timeout or other) on communication errors.
    pub fn single_polling_instruction(&mut self) -> Result<Vec<Rfid>, ConnectorError> {
        let mut rfids: Vec<Rfid> = Vec::new();
        self.send_packet(Command::SinglePollingInstruction)?;
        if let Ok(ps) = self.read_from_serial(None) {
            let ps = ps.unwrap_or(vec![]);

            if ps.len() == 1 && ps[0].get_data()[0] == 0x15 {
                debug!("Nessun tag presente in memoria");
            } else {
                info!("RFID ricevuti: {}", ps.len());
                for p in ps.iter() {
                    let data = p.get_data();
                    debug!("Lettura RFID Data: {:?}", data);

                    let rssi = data[0];
                    let pc = (data[1] as u16) * 16 * 16 + data[2] as u16;
                    let epc: Vec<u8>;
                    epc = data[3..12].to_owned();
                    let crc = data[15] as u16 * 16 * 16 + data[16] as u16;

                    rfids.push(Rfid { rssi, pc, epc, crc })
                }
            }
        }

        Ok(rfids)
    }
}

fn hexdump_line(prefix: &str, data: &[u8]) {
    let mut out = format!("{}", prefix);
    for b in data {
        out.push_str(format!("{:02X} ", b).as_str());
    }
    debug!("{}", out);
}
