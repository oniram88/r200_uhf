use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

use crate::core::command::Command;
use crate::core::error::CoreError;
use crate::core::frame::{Frame, FrameDecoder, error_frame_to_command_error};

const DEFAULT_TIMEOUT: Duration = Duration::from_millis(500);

pub struct AsyncReader<W> {
    port: W,
    timeout: Duration,
    decoder: FrameDecoder,
}

impl<W: AsyncReadExt + AsyncWriteExt + Unpin + Send> AsyncReader<W> {
    pub fn new(port: W) -> Self {
        Self {
            port,
            timeout: DEFAULT_TIMEOUT,
            decoder: FrameDecoder::with_capacity(4096),
        }
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Send a command and wait for its typed response.
    ///
    /// Unsolicited notification frames whose command code does not match the
    /// request are skipped, so a stray tag report cannot be mistaken for this
    /// command's response. Device error frames (`0xFF`) are always surfaced.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::Io`] on I/O failure, [`CoreError::Frame`] on
    /// malformed responses, or [`CoreError::Command`] if the device reports an error.
    pub async fn send<C: Command + Sync>(&mut self, cmd: &C) -> Result<C::Response, CoreError> {
        let wire = Frame::encode_command(C::CODE, &cmd.encode());
        self.port.write_all(&wire).await?;
        self.port.flush().await?;
        log::trace!("send {wire:02X?}");
        loop {
            let frame = self.recv_frame().await?;
            if frame.command_code == C::CODE {
                return cmd.decode_response(&frame.data);
            }
            log::trace!(
                "skipping unsolicited frame {:02X} while awaiting {:02X}",
                frame.command_code,
                C::CODE
            );
        }
    }

    async fn recv_frame(&mut self) -> Result<Frame, CoreError> {
        let frame = timeout(self.timeout, self.read_frame())
            .await
            .map_err(|_| CoreError::Frame(crate::core::error::FrameError::TooShort(0)))??;

        log::trace!("recv {:02X}: {:02X?}", frame.command_code, frame.data);

        if let Some(err) = error_frame_to_command_error(&frame) {
            return Err(CoreError::Command(err));
        }

        Ok(frame)
    }

    async fn read_frame(&mut self) -> Result<Frame, CoreError> {
        let mut tmp = [0u8; 1024];

        loop {
            if let Some(frame) = self.decoder.next_frame()? {
                return Ok(frame);
            }

            let n = self.port.read(&mut tmp).await?;
            if n == 0 {
                return Err(CoreError::Frame(crate::core::error::FrameError::TooShort(
                    self.decoder.buffered(),
                )));
            }
            self.decoder.extend(&tmp[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Region;
    use crate::core::command::*;
    use crate::core::error::CommandError;
    use crate::core::frame::{FRAME_HEADER, FrameType};
    use crate::util::PushU16;

    fn response_frame(command_code: u8, data: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(FRAME_HEADER);
        buf.push(FrameType::Response as u8);
        buf.push(command_code);
        buf.push_u16(data.len() as u16);
        buf.extend_from_slice(data);
        let cs: u8 = buf[1..].iter().fold(0u16, |a, &b| a + b as u16) as u8;
        buf.push(cs);
        buf.push(0xDD);
        buf
    }

    #[tokio::test]
    async fn get_module_info_async() {
        let resp = response_frame(0x03, b"V2.3.5");
        let (mut client, mut server) = tokio::io::duplex(4096);

        tokio::spawn(async move {
            let mut cmd_buf = [0u8; 1024];
            let _ = server.read(&mut cmd_buf).await.unwrap();
            server.write_all(&resp).await.unwrap();
        });

        let mut reader = AsyncReader::new(&mut client);
        let info = reader
            .send(&GetModuleInfo {
                param: ModuleInfoParam::SoftwareVersion,
            })
            .await
            .unwrap();
        assert_eq!(info.text, "V2.3.5");
    }

    #[tokio::test]
    async fn single_polling_no_tag_async() {
        let resp = response_frame(0x22, &[]);
        let (mut client, mut server) = tokio::io::duplex(4096);

        tokio::spawn(async move {
            let mut cmd_buf = [0u8; 1024];
            let _ = server.read(&mut cmd_buf).await.unwrap();
            server.write_all(&resp).await.unwrap();
        });

        let mut reader = AsyncReader::new(&mut client);
        let tag = reader.send(&SinglePollingInstruction).await.unwrap();
        assert!(tag.is_none());
    }

    #[tokio::test]
    async fn command_error_async() {
        let resp = response_frame(0xFF, &[0x09]);
        let (mut client, mut server) = tokio::io::duplex(4096);

        tokio::spawn(async move {
            let mut cmd_buf = [0u8; 1024];
            let _ = server.read(&mut cmd_buf).await.unwrap();
            server.write_all(&resp).await.unwrap();
        });

        let mut reader = AsyncReader::new(&mut client);
        let err = reader.send(&SinglePollingInstruction).await.unwrap_err();
        assert!(matches!(err, CoreError::Command(CommandError(0x09))));
    }

    #[tokio::test]
    async fn multiple_commands_async() {
        let r1 = response_frame(0x08, &[0x03]);
        let r2 = response_frame(0xB7, &[0x09, 0x9C]);
        let (mut client, mut server) = tokio::io::duplex(4096);

        tokio::spawn(async move {
            let mut cmd_buf = [0u8; 1024];
            let _ = server.read(&mut cmd_buf).await.unwrap();
            server.write_all(&r1).await.unwrap();
            let _ = server.read(&mut cmd_buf).await.unwrap();
            server.write_all(&r2).await.unwrap();
        });

        let mut reader = AsyncReader::new(&mut client);
        let region = reader.send(&GetWorkingArea).await.unwrap();
        assert_eq!(region, Region::Eu);

        let power = reader.send(&GetTransmitPower).await.unwrap();
        assert!((power - 24.6).abs() < 0.01);
    }
}
