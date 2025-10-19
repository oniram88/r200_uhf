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

#[cfg(test)]
mod tests {
    use super::*;
    use serialport::{ClearBuffer, DataBits, FlowControl, Parity, SerialPort, StopBits};
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    // Helper: build a device->PC frame with given command code and data bytes
    // cmd: command code for the request
    // param: optional parameter byte (e.g. channel code)
    // data: response data
    //
    fn make_frame(cmd: u8,param:Option<u8>, data: &[u8]) -> ResponseType {
        let mut v = Vec::new();
        v.push(R200_FRAME_HEADER);
        v.push(0x01); // frame type: from device to PC (arbitrary for tests)
        v.push(cmd);
        let len = data.len() as u16;
        v.push((len >> 8) as u8);
        v.push((len & 0xFF) as u8);
        v.extend_from_slice(data);
        // checksum: sum of bytes starting at index 2 (cmd, len, data)
        let sum: u16 = v[2..].iter().map(|&b| b as u16).sum();
        v.push((sum & 0xFF) as u8);
        v.push(R200_FRAME_END);

        ResponseType::Ok(        MockChat{
            request: (cmd,param),
            responses: Ok(v)
        })

    }

    fn make_error_frame(i: io::Error )-> ResponseType{
        ResponseType::Error(i)
    }

    enum ResponseType{
        Ok(MockChat),
        Error(io::Error),
        Raw(Vec<u8>),
    }

    #[derive(Default)]
    struct MockState {
        writes: Vec<Vec<u8>>, // captured writes
        // queue of reads to return on successive read() calls
        chats: Vec<ResponseType>,
        timeout: Duration,
    }

    struct MockSerialPort {
        state: Arc<Mutex<MockState>>,
    }

    struct MockChat {
        request: (u8, Option<u8>),
        responses: io::Result<Vec<u8>>,
    }

    impl MockSerialPort {
        fn new(chats: Vec<ResponseType>) -> Self {
            Self {
                state: Arc::new(Mutex::new(MockState {
                    writes: vec![],
                    chats,
                    timeout: Duration::from_millis(50),
                })),
            }
        }
        fn take_writes(&self) -> Vec<Vec<u8>> {
            let mut st = self.state.lock().unwrap();
            let out = st.writes.clone();
            st.writes.clear();
            out
        }
    }

    impl Read for MockSerialPort {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let mut st = self.state.lock().unwrap();

            let writes = st.writes.clone();

            if st.chats.is_empty() {
                // simulate timeout when no more data
                return Err(io::Error::new(io::ErrorKind::TimedOut, "timeout"));
            }
            let next = st.chats.remove(0);

            // TODO non sto ancora controllando il parametro corretto
            match next {
                ResponseType::Ok(n) => {
                    if let Some(last_write) = writes.last() {
                        let request_command = last_write[2];
                        if n.request.0 == request_command {
                            match n.responses {
                                Ok(bytes) => {
                                    let n = bytes.len().min(buf.len());
                                    buf[..n].copy_from_slice(&bytes[..n]);
                                    Ok(n)
                                }
                                Err(e) => Err(e),
                            }
                        } else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "Sequenza di comandi non prevista",
                            ));
                        }
                    }else{
                        // nel caso non abbiamo ricevuto nessuno comando di scrittura vuol dire
                        // che stiamo semplicemente leggendo una sequenza di frame
                        let bytes = n.responses.unwrap();
                        let n = bytes.len().min(buf.len());
                        buf[..n].copy_from_slice(&bytes[..n]);
                        Ok(n)
                    }
                }
                ResponseType::Error(e) => {
                    return Err(e)
                }
                ResponseType::Raw(bytes) => {
                    let n = bytes.len().min(buf.len());
                    buf[..n].copy_from_slice(&bytes[..n]);
                    Ok(n)
                }
            }

        }
    }

    impl Write for MockSerialPort {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut st = self.state.lock().unwrap();
            st.writes.push(buf.to_vec());
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl SerialPort for MockSerialPort {
        fn name(&self) -> Option<String> {
            Some("mock".into())
        }
        fn baud_rate(&self) -> serialport::Result<u32> {
            Ok(115200)
        }
        fn data_bits(&self) -> serialport::Result<DataBits> {
            Ok(DataBits::Eight)
        }
        fn flow_control(&self) -> serialport::Result<FlowControl> {
            Ok(FlowControl::None)
        }
        fn parity(&self) -> serialport::Result<Parity> {
            Ok(Parity::None)
        }
        fn stop_bits(&self) -> serialport::Result<StopBits> {
            Ok(StopBits::One)
        }
        fn timeout(&self) -> Duration {
            self.state.lock().unwrap().timeout
        }
        fn set_baud_rate(&mut self, _baud_rate: u32) -> serialport::Result<()> {
            Ok(())
        }
        fn set_data_bits(&mut self, _data_bits: DataBits) -> serialport::Result<()> {
            Ok(())
        }
        fn set_flow_control(&mut self, _flow_control: FlowControl) -> serialport::Result<()> {
            Ok(())
        }
        fn set_parity(&mut self, _parity: Parity) -> serialport::Result<()> {
            Ok(())
        }
        fn set_stop_bits(&mut self, _stop_bits: StopBits) -> serialport::Result<()> {
            Ok(())
        }
        fn set_timeout(&mut self, timeout: Duration) -> serialport::Result<()> {
            self.state.lock().unwrap().timeout = timeout;
            Ok(())
        }
        fn write_request_to_send(&mut self, _level: bool) -> serialport::Result<()> {
            Ok(())
        }
        fn write_data_terminal_ready(&mut self, _level: bool) -> serialport::Result<()> {
            Ok(())
        }
        fn read_clear_to_send(&mut self) -> serialport::Result<bool> {
            Ok(true)
        }
        fn read_data_set_ready(&mut self) -> serialport::Result<bool> {
            Ok(true)
        }
        fn read_ring_indicator(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn read_carrier_detect(&mut self) -> serialport::Result<bool> {
            Ok(true)
        }
        fn bytes_to_read(&self) -> serialport::Result<u32> {
            Ok(0)
        }
        fn bytes_to_write(&self) -> serialport::Result<u32> {
            Ok(0)
        }
        fn clear(&self, _buffer_to_clear: ClearBuffer) -> serialport::Result<()> {
            Ok(())
        }
        fn try_clone(&self) -> serialport::Result<Box<dyn SerialPort>> {
            Ok(Box::new(MockSerialPort {
                state: self.state.clone(),
            }))
        }
        fn set_break(&self) -> serialport::Result<()> {
            Ok(())
        }
        fn clear_break(&self) -> serialport::Result<()> {
            Ok(())
        }
    }

    // ----- Tests -----

    #[test]
    fn test_get_module_info() {
        let hw = make_frame(0x03,Some(0x01), b"HW1.0");
        let sw = make_frame(0x03,Some(0x02), b"SW2.0");
        let mf = make_frame(0x03,Some(0x03), b"ACME");
        let mock = MockSerialPort::new(vec![hw, sw, mf]);
        let mut connector = Connector::new(Box::new(mock));

        let info = connector.get_module_info().unwrap();
        assert!(info.contains("Hardware: HW1.0"));
        assert!(info.contains("Software: SW2.0"));
        assert!(info.contains("Manufacturer: ACME"));
    }

    #[test]
    fn test_get_working_area_mapping() {
        for (code, expected) in [
            (0, WorkingArea::China900Mhz),
            (1, WorkingArea::China800Mhz),
            (2, WorkingArea::US),
            (3, WorkingArea::EU),
            (4, WorkingArea::Korea),
        ] {
            let frame = make_frame(0x08,None, &[code]);
            let mock = MockSerialPort::new(vec![frame]);
            let mut connector = Connector::new(Box::new(mock));
            let area = connector.get_working_area().unwrap();
            // Compare by variant name via debug
            assert_eq!(format!("{:?}", area), format!("{:?}", expected));
        }
    }

    #[test]
    fn test_get_working_channel_uses_area() {
        // Channel index 4 -> depends on area. We'll test EU mapping: 0.2 MHz step + 865.1
        // First response: channel index, Second: area code 3 (EU)
        let chan = make_frame(0xAA,None, &[4]);
        let area = make_frame(0x08,None, &[3]);
        let mock = MockSerialPort::new(vec![chan, area]);
        let mut connector = Connector::new(Box::new(mock));
        let freq = connector.get_working_channel().unwrap();
        assert!((freq - (4.0 * 0.2 + 865.1)).abs() < 1e-6);
    }

    #[test]
    fn test_get_transmit_power() {
        // 27.50 -> 2750 -> 0x0A BE (for example 0x0A, 0xBE => 2750)
        let frame = make_frame(0xB7,Some(0x01), &[0x0A, 0xBE]);
        let mock = MockSerialPort::new(vec![frame]);
        let mut connector = Connector::new(Box::new(mock));
        let p = connector.get_transmit_power().unwrap();
        assert!((p - 27.50).abs() < 1e-6);
    }

    #[test]
    fn test_set_transmission_power_ack() {
        // ACK byte 0x00
        let frame = make_frame(0xB6,Some(0x01), &[0x00]);
        let mock = MockSerialPort::new(vec![frame]);
        let mut connector = Connector::new(Box::new(mock));
        connector.set_trasmission_power(30.0).unwrap();
    }

    #[test]
    fn test_single_polling_instruction_parses_tags() {
        // Build two tag frames then a timeout to end collection
        let tag1 = {
            let data = vec![
                55, // RSSI
                0x30, 0x12, // PC = 0x3012
                0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04, 0x05, // EPC 9 bytes (3..11)
                0x00, 0x00, 0x00, // padding to reach index 15
                0xAB, 0xCD, // CRC bytes at 15,16
            ];
            make_frame(0x22,None, &data)
        };
        let tag2 = {
            let data = vec![
                60, 0x20, 0x34, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0x00, 0x00,
                0x00, 0x12, 0x34,
            ];
            make_frame(0x22,None, &data)
        };
        let timeout = make_error_frame(io::Error::new(io::ErrorKind::TimedOut, "done"));
        let mock = MockSerialPort::new(vec![tag1, tag2, timeout]);
        let mut connector = Connector::new(Box::new(mock));
        let tags = connector.single_polling_instruction().unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].rssi, 55);
        assert_eq!(tags[0].pc, 0x3012);
        assert_eq!(tags[0].uid(), "deadbeef0102030405");
        assert_eq!(tags[0].crc, 0xABCD);
    }

    #[test]
    fn test_read_from_serial_noise_and_multiple_frames() {
        // Noise bytes, then two frames in one read, then timeout to finish
        let noise = vec![0x00, 0xFF, 0x13, 0x37];
        let f1 = make_frame(0x08,None, &[2]);
        let f2 = make_frame(0xAA,None, &[7]);
        let mock = MockSerialPort::new(vec![
            ResponseType::Raw(noise),
            f1,
            f2,
            make_error_frame(io::Error::new(io::ErrorKind::TimedOut, "t")),
        ]);
        let mut connector = Connector::new(Box::new(mock));
        let out = connector.read_from_serial(None).unwrap().unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].get_data(), vec![2]);
        assert_eq!(out[1].get_data(), vec![7]);
    }
}
