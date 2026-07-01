use crate::connector::{
    Connector, ConnectorError, WorkingArea, calculate_transmit_power, clear_non_ascii, hexdump_line,
};
use crate::frame::{Command, Frame, R200_FRAME_END, R200_FRAME_HEADER};
use crate::packet::Packet;
use crate::rfid::Rfid;
use log::{debug, error};
use std::io::{self, Read, Write};

pub trait SyncIO {
    type Socket: Read + Write;
    /// Setup the reader with default settings (inspired by e710_uhf)
    fn setup_reader(&mut self) -> Result<(), ConnectorError>;
    fn get_module_info(&mut self) -> Result<String, ConnectorError>;
    /// Builds and sends the command
    fn send_packet(&mut self, command: Command) -> Result<(), ConnectorError>;
    fn single_read_from_serial(&mut self) -> Result<Option<Packet>, ConnectorError>;
    fn read_from_serial(
        &mut self,
        num_expected_responses: Option<u32>,
    ) -> Result<Option<Vec<Packet>>, ConnectorError>;
    /// Get the current regulatory working area configured on the device.
    ///
    /// Returns
    /// - Ok(WorkingArea) with the region inferred from the device response.
    /// - Err(ConnectorError::InvalidWorkingArea) if the response contains an unknown code.
    /// - Err(ConnectorError::NoPacketReceived) if nothing is received.
    /// - Other ConnectorError variants on I/O failure or timeout.
    fn get_working_area(&mut self) -> Result<WorkingArea, ConnectorError>;
    /// Get the current working RF channel as a frequency in MHz.
    ///
    /// The raw channel index returned by the device is converted to MHz based on
    /// the configured WorkingArea. Different regions use different spacing and base frequencies.
    ///
    /// Returns
    /// - Ok(f64) with the center frequency in MHz.
    /// - Err(ConnectorError::NoPacketReceived) if no response is obtained.
    /// - Other ConnectorError variants on I/O failure, timeout, or unknown working area.
    fn get_working_channel(&mut self) -> Result<f64, ConnectorError>;
    /// Read the current transmit power reported by the device.
    ///
    /// The device returns two bytes that represent the power value scaled by 100.
    /// This method combines them and returns the value as f64.
    ///
    /// Returns
    /// - Ok(f64) with the transmit power (device-specific units, typically dBm).
    /// - Err(ConnectorError::NoPacketReceived) if no response is obtained.
    /// - Other ConnectorError variants on I/O failure or timeout.
    fn get_transmit_power(&mut self) -> Result<f64, ConnectorError>;
    /// Set the transmitter output power.
    ///
    /// Parameters
    /// - power: Desired transmit power in device-specific units (typically dBm).
    ///
    /// Returns
    /// - Ok(()) when the device acknowledges the setting.
    /// - Err(ConnectorError::NoPacketReceived) if no response is obtained.
    /// - Other ConnectorError variants on I/O failure or timeout.
    fn set_transmission_power(&mut self, power: f64) -> Result<(), ConnectorError>;
    /// Perform a single inventory (poll) and return the list of detected tags.
    ///
    /// Sends a SinglePollingInstruction to the reader and parses all returned packets
    /// into a collection of Rfid records containing RSSI, PC, EPC (UID) and CRC.
    ///
    /// Returns
    /// - Ok(Vec<Rfid>) possibly empty if no tags are present.
    /// - Err(ConnectorError::Timeout or other) on communication errors.
    fn single_polling_instruction(&mut self) -> Result<Vec<Rfid>, ConnectorError>;
    fn multi_polling_instruction(&mut self) -> Result<Vec<Rfid>, ConnectorError>; // Start Multi: AA 00 27 00 03 22 FF FF 4A DD
    fn enable_multiple_polling_instructions(
        &mut self,
        pool_times: u16,
    ) -> Result<(), ConnectorError>; // Stop Multi: AA 00 28 00 00 28 DD
    fn stop_multiple_polling_instructions(&mut self) -> Result<(), ConnectorError>;
}

impl<S> SyncIO for Connector<S>
where
    S: Read + Write,
{
    type Socket = S;

    /// Setup the reader with default settings (inspired by e710_uhf)
    fn setup_reader(&mut self) -> Result<(), ConnectorError> {
        self.stop_multiple_polling_instructions().ok();
        Ok(())
    }

    fn get_module_info(&mut self) -> Result<String, ConnectorError> {
        self.send_packet(Command::HardwareVersion)?;
        let hardware = self.single_read_from_serial();
        self.send_packet(Command::SoftwareVersion)?;
        let software = self.single_read_from_serial();
        self.send_packet(Command::Manufacturer)?;
        let manufacture = self.single_read_from_serial();

        let out = format!(
            "Hardware: {} - Software: {} - Manufacturer: {}",
            clear_non_ascii(hardware?.unwrap().to_string().as_str()),
            clear_non_ascii(software?.unwrap().to_string().as_str()),
            clear_non_ascii(manufacture?.unwrap().to_string().as_str())
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

                        if p.is_valid() {
                            debug!("{}", p.debug());
                            output.push(p);
                            if output.len() >= num_expected_responses.unwrap_or(100000) as usize {
                                return Ok(Some(output));
                            }
                        } else {
                            error!("Invalid packet: {:?}", chunk);
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
    fn get_working_area(&mut self) -> Result<WorkingArea, ConnectorError> {
        self.send_packet(Command::GetWorkingArea)?;
        let p = self.single_read_from_serial()?;
        if let Some(p) = p {
            return Connector::<S>::parse_to_working_area(p);
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
    fn get_working_channel(&mut self) -> Result<f64, ConnectorError> {
        self.send_packet(Command::GetWorkingChannel)?;
        let p = self.single_read_from_serial()?;
        if let Some(p) = p {
            return Ok(self.get_working_area()?.packet_to_64(p));
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
    fn get_transmit_power(&mut self) -> Result<f64, ConnectorError> {
        self.send_packet(Command::AcquireTransmitPower)?;
        let p = self.single_read_from_serial()?;
        if let Some(p) = p {
            return Ok(calculate_transmit_power(p));
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
    fn set_transmission_power(&mut self, power: f64) -> Result<(), ConnectorError> {
        self.send_packet(Command::SetTransmissionPower(power))?;
        Connector::<S>::_set_transmission_power(self.single_read_from_serial()?, power)
    }

    /// Perform a single inventory (poll) and return the list of detected tags.
    ///
    /// Sends a SinglePollingInstruction to the reader and parses all returned packets
    /// into a collection of Rfid records containing RSSI, PC, EPC (UID) and CRC.
    ///
    /// Returns
    /// - Ok(Vec<Rfid>) possibly empty if no tags are present.
    /// - Err(ConnectorError::Timeout or other) on communication errors.
    fn single_polling_instruction(&mut self) -> Result<Vec<Rfid>, ConnectorError> {
        self.send_packet(Command::SinglePollingInstruction)?;
        let response = self.read_from_serial(None)?;
        self.parse_rfid_packets(response)
    }

    fn multi_polling_instruction(&mut self) -> Result<Vec<Rfid>, ConnectorError> {
        self.send_packet(Command::MultiplePollingInstruction(100))?;
        let response = self.read_from_serial(Some(100))?;
        self.parse_rfid_packets(response)
    }

    // Start Multi: AA 00 27 00 03 22 FF FF 4A DD
    fn enable_multiple_polling_instructions(
        &mut self,
        pool_times: u16,
    ) -> Result<(), ConnectorError> {
        self.send_packet(Command::MultiplePollingInstruction(pool_times))
    }

    // Stop Multi: AA 00 28 00 00 28 DD
    fn stop_multiple_polling_instructions(&mut self) -> Result<(), ConnectorError> {
        self.send_packet(Command::StopMultiplePollingInstruction)?;
        if let Some(p) = self.single_read_from_serial()? {
            if matches!(p.command(), Ok(Command::StopMultiplePollingInstruction)) {
                return Ok(());
            } else {
                return Err(ConnectorError::ErrorStopMultiPolling(
                    "Wrong response from device".into(),
                ));
            }
        }
        Err(ConnectorError::ErrorStopMultiPolling(
            "Generic comunication error".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    // Helper: build a device->PC frame with given command code and data bytes
    // cmd: command code for the request
    // param: optional parameter byte (e.g. channel code)
    // data: response data
    //
    fn make_frame(cmd: u8, param: Option<Vec<u8>>, data: &[u8]) -> ResponseType {
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

        ResponseType::Ok(MockChat {
            request: (cmd, param),
            responses: Ok(v),
        })
    }

    fn make_error_frame(i: io::Error) -> ResponseType {
        ResponseType::Error(i)
    }

    enum ResponseType {
        Ok(MockChat),
        Error(io::Error),
        Raw(Vec<u8>),
    }

    #[derive(Default)]
    struct MockState {
        writes: Vec<Vec<u8>>, // captured writes
        // queue of reads to return on successive read() calls
        chats: Vec<ResponseType>,
    }

    struct MockSerialPort {
        state: Arc<Mutex<MockState>>,
    }

    struct MockChat {
        request: (u8, Option<Vec<u8>>),
        responses: io::Result<Vec<u8>>,
    }

    impl MockSerialPort {
        fn new(chats: Vec<ResponseType>) -> Self {
            Self {
                state: Arc::new(Mutex::new(MockState {
                    writes: vec![],
                    chats,
                })),
            }
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

            match next {
                ResponseType::Ok(n) => {
                    if let Some(last_write) = writes.last() {
                        let request_command = last_write[2];

                        // check del parametro
                        let parameter_is_valid: bool;

                        if let Some(p) = n.request.1 {
                            // controllo che sia impostato il valore 1 di lunghezza parametri (posizione 4) e
                            // che il parametro sia impostato corettamente (posizione 5)
                            let params = &last_write[5..5 + p.len()];
                            if last_write[4] == (p.len() as u8) && p == params {
                                parameter_is_valid = true;
                            } else {
                                parameter_is_valid = false;
                            }
                        } else {
                            parameter_is_valid = true
                        }

                        if n.request.0 == request_command && parameter_is_valid {
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
                    } else {
                        // nel caso non abbiamo ricevuto nessuno comando di scrittura vuol dire
                        // che stiamo semplicemente leggendo una sequenza di frame
                        let bytes = n.responses.unwrap();
                        let n = bytes.len().min(buf.len());
                        buf[..n].copy_from_slice(&bytes[..n]);
                        Ok(n)
                    }
                }
                ResponseType::Error(e) => return Err(e),
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

    // ----- Tests -----

    #[test]
    fn test_get_module_info() {
        let hw = make_frame(0x03, Some(vec![0x00]), b"HW1.0");
        let sw = make_frame(0x03, Some(vec![0x01]), b"SW2.0");
        let mf = make_frame(0x03, Some(vec![0x02]), b"ACME");
        let mock = MockSerialPort::new(vec![hw, sw, mf]);
        let mut connector = Connector::new(mock);

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
            let frame = make_frame(0x08, None, &[code]);
            let mock = MockSerialPort::new(vec![frame]);
            let mut connector = Connector::new(mock);
            let area = connector.get_working_area().unwrap();
            // Compare by variant name via debug
            assert_eq!(format!("{:?}", area), format!("{:?}", expected));
        }
    }

    #[test]
    fn test_get_working_channel_uses_area() {
        // Channel index 4 -> depends on area. We'll test EU mapping: 0.2 MHz step + 865.1
        // First response: channel index, Second: area code 3 (EU)
        let chan = make_frame(0xAA, None, &[4]);
        let area = make_frame(0x08, None, &[3]);
        let mock = MockSerialPort::new(vec![chan, area]);
        let mut connector = Connector::new(mock);
        let freq = connector.get_working_channel().unwrap();
        assert!((freq - (4.0 * 0.2 + 865.1)).abs() < 1e-6);
    }

    #[test]
    fn test_get_transmit_power() {
        // 27.50 -> 2750 -> 0x0A BE (for example 0x0A, 0xBE => 2750)
        let frame = make_frame(0xB7, None, &[0x0A, 0xBE]);
        let mock = MockSerialPort::new(vec![frame]);
        let mut connector = Connector::new(mock);
        let p = connector.get_transmit_power().unwrap();
        assert!((p - 27.50).abs() < 1e-6);
    }

    #[test]
    fn test_set_transmission_power_ack() {
        // ACK byte 0x00
        let frame = make_frame(0xB6, Some(vec![0x07, 0xD0]), &[0x00]);
        let mock = MockSerialPort::new(vec![frame]);
        let mut connector = Connector::new(mock);
        connector.set_transmission_power(20.0).unwrap();
    }

    #[test]
    fn test_single_polling_instruction_parses_tags() {
        // Build two tag frames then a timeout to end collection
        let tag1 = {
            let data = vec![
                55, // RSSI
                0x30, 0x12, // PC = 0x3012
                0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
                0x08, // padding to reach index 15
                0xAB, 0xCD, // CRC bytes at 15,16
            ];
            make_frame(0x22, None, &data)
        };
        let tag2 = {
            let data = vec![
                60, 0x20, 0x34, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB,
                0xCC, 0x12, 0x34,
            ];
            make_frame(0x22, None, &data)
        };
        let timeout = make_error_frame(io::Error::new(io::ErrorKind::TimedOut, "done"));
        let mock = MockSerialPort::new(vec![tag1, tag2, timeout]);
        let mut connector = Connector::new(mock);
        let tags = connector.single_polling_instruction().unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].uid(), "DEADBEEF0102030405060708");
    }

    #[test]
    fn test_read_from_serial_noise_and_multiple_frames() {
        // Noise bytes, then two frames in one read, then timeout to finish
        let noise = vec![0x00, 0xFF, 0x13, 0x37];
        let f1 = make_frame(0x08, None, &[2]);
        let f2 = make_frame(0xAA, None, &[7]);
        let mock = MockSerialPort::new(vec![
            ResponseType::Raw(noise),
            f1,
            f2,
            make_error_frame(io::Error::new(io::ErrorKind::TimedOut, "t")),
        ]);
        let mut connector = Connector::new(mock);
        let out = connector.read_from_serial(None).unwrap().unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].get_data(), vec![2]);
        assert_eq!(out[1].get_data(), vec![7]);
    }

    // ---- clear_non_ascii tests ----

    #[test]
    fn test_clear_non_ascii_ascii_only() {
        let s = "Hello, World! 123";
        let out = clear_non_ascii(s);
        assert_eq!(out, s);
    }

    #[test]
    fn test_clear_non_ascii_removes_non_ascii() {
        // Mixed ASCII + non-ASCII (Euro sign, CJK, and 'ç')
        let s = "a€b測cçd";
        let out = clear_non_ascii(s);
        assert_eq!(out, "abcd");
    }

    #[test]
    fn test_clear_non_ascii_keeps_ascii_control_chars() {
        let s = "A\nB\tC\r\x07"; // includes newline, tab, carriage return, BEL
        let out = clear_non_ascii(s);
        assert_eq!(out, s);
    }

    #[test]
    fn test_clear_non_ascii_empty_input() {
        let s = "";
        let out = clear_non_ascii(s);
        assert_eq!(out, "");
    }
}
