use async_trait::async_trait;
use crate::connector::{clear_non_ascii, hexdump_line, Connector, ConnectorError, WorkingArea};
use crate::frame::{Command, Frame, R200_FRAME_END, R200_FRAME_HEADER};
use crate::packet::Packet;
use crate::rfid::Rfid;
use log::{debug, info};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[async_trait]
pub trait AsyncIO {
    type Socket: AsyncRead + AsyncWrite + Unpin + Send;
    async fn setup_reader(&mut self) -> Result<(), ConnectorError>;
    async fn get_module_info(&mut self) -> Result<String, ConnectorError>;
    async fn send_packet(&mut self, command: Command) -> Result<(), ConnectorError>;
    async fn single_read_from_serial(&mut self) -> Result<Option<Packet>, ConnectorError>;
    async fn read_from_serial(
        &mut self,
        num_expected_responses: Option<u32>,
    ) -> Result<Option<Vec<Packet>>, ConnectorError>;
    async fn get_working_area(&mut self) -> Result<WorkingArea, ConnectorError>;
    async fn get_working_channel(&mut self) -> Result<f64, ConnectorError>;
    async fn get_transmit_power(&mut self) -> Result<f64, ConnectorError>;
    async fn set_transmission_power(&mut self, power: f64) -> Result<(), ConnectorError>;
    async fn single_polling_instruction(&mut self) -> Result<Vec<Rfid>, ConnectorError>;
    fn parse_rfid_packets(
        &self,
        response: Option<Vec<Packet>>,
    ) -> Result<Vec<Rfid>, ConnectorError>;
    async fn multi_polling_instruction(&mut self) -> Result<Vec<Rfid>, ConnectorError>;
    async fn stop_multiple_polling_instructions(&mut self) -> Result<(), ConnectorError>;
}

#[async_trait]
impl<S> AsyncIO for Connector<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    type Socket = S;

    async fn setup_reader(&mut self) -> Result<(), ConnectorError> {
        self.stop_multiple_polling_instructions().await.ok();
        Ok(())
    }

    async fn get_module_info(&mut self) -> Result<String, ConnectorError> {
        self.send_packet(Command::HardwareVersion).await?;
        let hardware = self.single_read_from_serial().await?;
        self.send_packet(Command::SoftwareVersion).await?;
        let software = self.single_read_from_serial().await?;
        self.send_packet(Command::Manufacturer).await?;
        let manufacture = self.single_read_from_serial().await?;

        let hw_str = hardware.map(|p| p.to_string()).unwrap_or_default();
        let sw_str = software.map(|p| p.to_string()).unwrap_or_default();
        let mf_str = manufacture.map(|p| p.to_string()).unwrap_or_default();

        let out = format!(
            "Hardware: {} - Software: {} - Manufacturer: {}",
            clear_non_ascii(&hw_str),
            clear_non_ascii(&sw_str),
            clear_non_ascii(&mf_str)
        );

        Ok(out)
    }

    async fn send_packet(&mut self, command: Command) -> Result<(), ConnectorError> {
        let frame = Frame::new(&command).to_bytes();

        let mut out = String::new();
        for b in &frame {
            out.push_str(format!("{:02X} ", b).as_str());
        }
        debug!("[TX] {out} - [{command}]");

        self.port.write_all(&frame).await?;
        self.port.flush().await?;
        Ok(())
    }

    async fn single_read_from_serial(&mut self) -> Result<Option<Packet>, ConnectorError> {
        let out = self.read_from_serial(Some(1)).await?;
        Ok(out.unwrap_or(vec![]).pop())
    }

    async fn read_from_serial(
        &mut self,
        num_expected_responses: Option<u32>,
    ) -> Result<Option<Vec<Packet>>, ConnectorError> {
        let mut read_buf: [u8; 1024] = [0u8; 1024];
        let mut rolling: Vec<u8> = Vec::with_capacity(4096);
        let mut output: Vec<Packet> = Vec::new();

        loop {
            let read_future = self.port.read(&mut read_buf);
            
            // In a real async scenario with timeout, we might use tokio::time::timeout
            let raw_data_size = match tokio::time::timeout(std::time::Duration::from_millis(500), read_future).await {
                Ok(res) => res,
                Err(_) => {
                    if output.is_empty() {
                        return Err(ConnectorError::Timeout);
                    }
                    break;
                }
            };

            match raw_data_size {
                Ok(n) if n > 0 => {
                    rolling.extend_from_slice(&read_buf[..n]);
                    hexdump_line("[RAW] ", &rolling);

                    while let Some(header_pos) = rolling.iter().position(|&x| x == R200_FRAME_HEADER) {
                        if let Some(end_pos) = rolling.iter().position(|&x| x == R200_FRAME_END) {
                            if end_pos > header_pos {
                                let chunk = &rolling[header_pos..=end_pos];
                                if chunk.len() > 4 {
                                    let p = Packet::new(Vec::from(chunk));
                                    if p.is_valid() {
                                        debug!("{}", p.debug());
                                        output.push(p);
                                        if output.len() >= num_expected_responses.unwrap_or(100000) as usize {
                                            return Ok(Some(output));
                                        }
                                    }
                                }
                                rolling.drain(..=end_pos);
                            } else {
                                // End before header, discard everything before header
                                rolling.drain(..header_pos);
                                break;
                            }
                        } else {
                            // Header but no end yet
                            break;
                        }
                    }

                    if rolling.len() > 8192 {
                        rolling.drain(..rolling.len() - 4096);
                    }
                }
                Ok(_) => return Ok(None),
                Err(e) => return Err(ConnectorError::SerialRead(e.to_string())),
            }
        }
        Ok(Some(output))
    }

    async fn get_working_area(&mut self) -> Result<WorkingArea, ConnectorError> {
        self.send_packet(Command::GetWorkingArea).await?;
        if let Some(p) = self.single_read_from_serial().await? {
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

    async fn get_working_channel(&mut self) -> Result<f64, ConnectorError> {
        self.send_packet(Command::GetWorkingChannel).await?;
        if let Some(p) = self.single_read_from_serial().await? {
            match self.get_working_area().await? {
                WorkingArea::China900Mhz => return Ok((p.get_data()[0] as f64) * 0.25 + 920.125),
                WorkingArea::China800Mhz => return Ok((p.get_data()[0] as f64) * 0.25 + 840.125),
                WorkingArea::US => return Ok((p.get_data()[0] as f64) * 0.50 + 902.25),
                WorkingArea::EU => return Ok((p.get_data()[0] as f64) * 0.2 + 865.1),
                WorkingArea::Korea => return Ok((p.get_data()[0] as f64) * 0.2 + 917.1),
            }
        }
        Err(ConnectorError::NoPacketReceived)
    }

    async fn get_transmit_power(&mut self) -> Result<f64, ConnectorError> {
        self.send_packet(Command::AcquireTransmitPower).await?;
        if let Some(p) = self.single_read_from_serial().await? {
            let data = p.get_data();
            return Ok(((data[0] as u16) * 256 + (data[1] as u16)) as f64 / 100.0);
        }
        Err(ConnectorError::NoPacketReceived)
    }

    async fn set_transmission_power(&mut self, power: f64) -> Result<(), ConnectorError> {
        self.send_packet(Command::SetTransmissionPower(power)).await?;
        if let Some(p) = self.single_read_from_serial().await? {
            if p.get_data()[0] == 0x00 {
                info!("Power correctly set to {}", power);
                return Ok(());
            } else {
                return Err(ConnectorError::FailedSetting(format!(
                    "Failed to set power to {}",
                    power
                )));
            }
        }
        Err(ConnectorError::NoPacketReceived)
    }

    async fn single_polling_instruction(&mut self) -> Result<Vec<Rfid>, ConnectorError> {
        self.send_packet(Command::SinglePollingInstruction).await?;
        let response = self.read_from_serial(None).await?;
        self.parse_rfid_packets(response)
    }

    fn parse_rfid_packets(
        &self,
        response: Option<Vec<Packet>>,
    ) -> Result<Vec<Rfid>, ConnectorError> {
        let mut rfids = Vec::new();
        if let Some(ps) = response {
            if ps.len() == 1 && ps[0].get_data()[0] == 0x15 {
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

    async fn multi_polling_instruction(&mut self) -> Result<Vec<Rfid>, ConnectorError> {
        self.send_packet(Command::MultiplePollingInstruction(100)).await?;
        let response = self.read_from_serial(Some(100)).await?;
        self.parse_rfid_packets(response)
    }

    async fn stop_multiple_polling_instructions(&mut self) -> Result<(), ConnectorError> {
        self.send_packet(Command::StopMultiplePollingInstruction).await?;
        if let Some(p) = self.single_read_from_serial().await? {
            if matches!(p.command(), Ok(Command::StopMultiplePollingInstruction)) {
                return Ok(());
            }
        }
        Err(ConnectorError::ErrorStopMultiPolling(
            "Failed to stop multi polling".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncRead, AsyncWrite};
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::sync::{Arc, Mutex};
    use std::io;

    struct MockAsyncPort {
        read_data: Vec<u8>,
        written_data: Arc<Mutex<Vec<u8>>>,
    }

    impl AsyncRead for MockAsyncPort {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            if self.read_data.is_empty() {
                // Return EOF if empty to avoid infinite loop or timeout in tests
                return Poll::Ready(Ok(()));
            }
            let n = std::cmp::min(buf.remaining(), self.read_data.len());
            let data: Vec<u8> = self.read_data.drain(..n).collect();
            buf.put_slice(&data);
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for MockAsyncPort {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.written_data.lock().unwrap().extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn test_async_get_module_info() {
        // Mock response for Hardware, Software, Manufacturer
        // For simplicity, just one valid packet
        let mut resp = Vec::new();
        // Hardware Version (Command 0x03)
        let mut f1 = Frame::new(&Command::HardwareVersion).to_bytes();
        // Replace TX frame with RX frame for test (mocking device response)
        f1[1] = 0x01; // Device to PC
        resp.extend_from_slice(&f1);
        
        // Software Version
        let mut f2 = Frame::new(&Command::SoftwareVersion).to_bytes();
        f2[1] = 0x01;
        resp.extend_from_slice(&f2);
        
        // Manufacturer
        let mut f3 = Frame::new(&Command::Manufacturer).to_bytes();
        f3[1] = 0x01;
        resp.extend_from_slice(&f3);

        let port = MockAsyncPort {
            read_data: resp,
            written_data: Arc::new(Mutex::new(Vec::new())),
        };
        let mut connector = Connector::new(port);
        let info = connector.get_module_info().await.unwrap();
        assert!(info.contains("Hardware"));
    }
}
