use crate::core::error::FrameError;
use crate::util::PushU16;

pub const FRAME_HEADER: u8 = 0xAA;
pub const FRAME_END: u8 = 0xDD;
pub const MIN_FRAME_LEN: usize = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[repr(u8)]
pub enum FrameType {
    Command = 0x00,
    Response = 0x01,
    Notification = 0x02,
}

impl FrameType {
    #[must_use]
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x00 => Some(Self::Command),
            0x01 => Some(Self::Response),
            0x02 => Some(Self::Notification),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub frame_type: FrameType,
    pub command_code: u8,
    pub data: Vec<u8>,
}

impl Frame {
    #[must_use]
    pub fn encode_command(command_code: u8, data: &[u8]) -> Vec<u8> {
        let len = data.len();
        let mut buf = Vec::with_capacity(MIN_FRAME_LEN + len);
        buf.push(FRAME_HEADER);
        buf.push(FrameType::Command as u8);
        buf.push(command_code);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "protocol frame length fits in u16"
        )]
        buf.push_u16(len as u16);
        buf.extend_from_slice(data);
        buf.push(checksum(&buf[1..buf.len()]));
        buf.push(FRAME_END);
        buf
    }

    /// Decode a frame from the front of a byte buffer.
    ///
    /// Returns the decoded frame and the number of bytes consumed.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError`] if the buffer is too short, has a bad header/checksum,
    /// or declares more data than available.
    pub fn decode(buf: &[u8]) -> Result<(Self, usize), FrameError> {
        if buf.len() < MIN_FRAME_LEN {
            return Err(FrameError::TooShort(buf.len()));
        }
        if buf[0] != FRAME_HEADER {
            return Err(FrameError::BadHeader(buf[0]));
        }

        let frame_type =
            FrameType::from_byte(buf[1]).ok_or(FrameError::UnknownFrameType(buf[1]))?;
        let command_code = buf[2];
        let data_len = ((buf[3] as usize) << 8) | (buf[4] as usize);
        let total_len = MIN_FRAME_LEN + data_len;

        if buf.len() < total_len {
            return Err(FrameError::Truncated {
                declared: data_len,
                available: buf.len() - 5,
            });
        }

        let end = buf[total_len - 1];
        if end != FRAME_END {
            return Err(FrameError::MissingEndMarker(end));
        }

        let checksum_range = &buf[1..total_len - 2];
        let expected_cs = checksum(checksum_range);
        let actual_cs = buf[total_len - 2];
        if expected_cs != actual_cs {
            return Err(FrameError::ChecksumMismatch {
                expected: expected_cs,
                actual: actual_cs,
            });
        }

        let data = buf[5..5 + data_len].to_vec();
        Ok((
            Self {
                frame_type,
                command_code,
                data,
            },
            total_len,
        ))
    }
}

#[must_use]
pub fn checksum(bytes: &[u8]) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "checksum is modulo 256 by design"
    )]
    {
        bytes.iter().fold(0u16, |acc, &b| acc + u16::from(b)) as u8
    }
}

/// Command code the device uses for error response frames.
pub const ERROR_FRAME_CODE: u8 = 0xFF;

/// A streaming frame decoder that buffers raw bytes and yields complete frames.
///
/// Both the sync and async transports feed bytes in via [`FrameDecoder::extend`]
/// and repeatedly call [`FrameDecoder::next_frame`]. This keeps the byte-level
/// framing logic in one place, independent of the I/O mechanism (sans-io).
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    /// Create a decoder with a pre-allocated buffer.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: Vec::with_capacity(capacity),
        }
    }

    /// Append freshly-read bytes to the internal buffer.
    pub fn extend(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Number of bytes currently buffered.
    #[must_use]
    pub const fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// Try to decode the next complete frame from the buffer.
    ///
    /// Leading noise before a [`FRAME_HEADER`] is discarded. Returns `Ok(None)`
    /// when more bytes are needed to complete a frame.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError`] for a malformed frame (bad checksum, missing end
    /// marker, unknown frame type, ...). A truncated frame is not an error; it
    /// yields `Ok(None)` so the caller can read more bytes.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        let Some(pos) = self.buf.iter().position(|&b| b == FRAME_HEADER) else {
            return Ok(None);
        };
        if pos > 0 {
            self.buf.drain(..pos);
        }
        match Frame::decode(&self.buf) {
            Ok((frame, consumed)) => {
                self.buf.drain(..consumed);
                Ok(Some(frame))
            }
            Err(FrameError::TooShort(_) | FrameError::Truncated { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Map an error response frame (command code `0xFF`) to its
/// [`CommandError`](crate::core::error::CommandError).
///
/// Returns `None` for non-error frames.
#[must_use]
pub fn error_frame_to_command_error(frame: &Frame) -> Option<crate::core::error::CommandError> {
    if frame.command_code == ERROR_FRAME_CODE {
        let code = frame.data.first().copied().unwrap_or(ERROR_FRAME_CODE);
        Some(crate::core::error::CommandError(code))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_command_frame() {
        let wire = Frame::encode_command(0x03, &[0x00]);
        assert_eq!(wire[0], FRAME_HEADER);
        assert_eq!(wire[1], 0x00); // Command type
        assert_eq!(wire[2], 0x03); // GetModuleInfo
        assert_eq!(wire[3], 0x00); // len high
        assert_eq!(wire[4], 0x01); // len low
        assert_eq!(wire[5], 0x00); // data
        assert_eq!(wire[7], FRAME_END);
    }

    #[test]
    fn encode_decode_round_trip() {
        let data = vec![0x01, 0x02, 0x03, 0x04];
        let wire = Frame::encode_command(0x22, &data);
        let (decoded, consumed) = Frame::decode(&wire).unwrap();
        assert_eq!(consumed, wire.len());
        assert_eq!(decoded.command_code, 0x22);
        assert_eq!(decoded.data, data);
    }

    #[test]
    fn decode_rejects_short_frame() {
        let err = Frame::decode(&[0xAA, 0x00]).unwrap_err();
        assert!(matches!(err, FrameError::TooShort(2)));
    }

    #[test]
    fn decode_rejects_bad_header() {
        let wire = Frame::encode_command(0x03, &[]);
        let mut bad = wire.clone();
        bad[0] = 0xFF;
        assert!(matches!(
            Frame::decode(&bad),
            Err(FrameError::BadHeader(0xFF))
        ));
    }

    #[test]
    fn decode_rejects_bad_checksum() {
        let mut wire = Frame::encode_command(0x03, &[0x01]);
        let cs_idx = wire.len() - 2;
        wire[cs_idx] ^= 0xFF;
        assert!(matches!(
            Frame::decode(&wire),
            Err(FrameError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn decode_rejects_missing_end_marker() {
        let mut wire = Frame::encode_command(0x03, &[]);
        let end_idx = wire.len() - 1;
        wire[end_idx] = 0xFF;
        assert!(matches!(
            Frame::decode(&wire),
            Err(FrameError::MissingEndMarker(0xFF))
        ));
    }

    #[test]
    fn decode_rejects_truncated_payload() {
        let mut wire = Frame::encode_command(0x03, &[0x01, 0x02]);
        wire.truncate(wire.len() - 1);
        assert!(matches!(
            Frame::decode(&wire),
            Err(FrameError::Truncated { .. })
        ));
    }

    #[test]
    fn checksum_basic() {
        assert_eq!(checksum(&[0x00]), 0x00);
        assert_eq!(checksum(&[0x01, 0x02, 0x03]), 0x06);
    }

    #[test]
    fn frame_type_from_byte() {
        assert_eq!(FrameType::from_byte(0x00), Some(FrameType::Command));
        assert_eq!(FrameType::from_byte(0x01), Some(FrameType::Response));
        assert_eq!(FrameType::from_byte(0x02), Some(FrameType::Notification));
        assert_eq!(FrameType::from_byte(0xFF), None);
    }

    #[test]
    fn encode_large_payload() {
        let data = vec![0xAB; 256];
        let wire = Frame::encode_command(0x27, &data);
        assert_eq!(wire.len(), MIN_FRAME_LEN + 256);
        let (decoded, _) = Frame::decode(&wire).unwrap();
        assert_eq!(decoded.data.len(), 256);
    }

    #[test]
    fn decode_multiple_frames_in_buffer() {
        let f1 = Frame::encode_command(0x03, &[0x00]);
        let f2 = Frame::encode_command(0x22, &[0x01, 0x02]);
        let mut buf = f1.clone();
        buf.extend_from_slice(&f2);

        let (frame1, consumed1) = Frame::decode(&buf).unwrap();
        assert_eq!(frame1.command_code, 0x03);
        assert_eq!(consumed1, f1.len());

        let (frame2, consumed2) = Frame::decode(&buf[consumed1..]).unwrap();
        assert_eq!(frame2.command_code, 0x22);
        assert_eq!(consumed2, f2.len());
    }

    #[test]
    fn decoder_yields_frames_across_chunks() {
        let wire = Frame::encode_command(0x22, &[0xAA, 0xBB]);
        let (head, tail) = wire.split_at(4);

        let mut dec = FrameDecoder::default();
        dec.extend(head);
        // Incomplete: needs more bytes, not an error.
        assert!(dec.next_frame().unwrap().is_none());
        dec.extend(tail);
        let frame = dec.next_frame().unwrap().unwrap();
        assert_eq!(frame.command_code, 0x22);
        assert_eq!(frame.data, vec![0xAA, 0xBB]);
        // Buffer drained.
        assert_eq!(dec.buffered(), 0);
        assert!(dec.next_frame().unwrap().is_none());
    }

    #[test]
    fn decoder_discards_leading_noise() {
        let wire = Frame::encode_command(0x08, &[0x03]);
        let mut dec = FrameDecoder::default();
        dec.extend(&[0x00, 0xFF, 0x42]);
        dec.extend(&wire);
        let frame = dec.next_frame().unwrap().unwrap();
        assert_eq!(frame.command_code, 0x08);
        assert_eq!(frame.data, vec![0x03]);
    }

    #[test]
    fn decoder_propagates_checksum_error() {
        let mut wire = Frame::encode_command(0x03, &[0x01]);
        let cs_idx = wire.len() - 2;
        wire[cs_idx] ^= 0xFF;
        let mut dec = FrameDecoder::default();
        dec.extend(&wire);
        assert!(matches!(
            dec.next_frame(),
            Err(FrameError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn error_frame_maps_to_command_error() {
        let ok = Frame {
            frame_type: FrameType::Response,
            command_code: 0x22,
            data: vec![],
        };
        assert!(error_frame_to_command_error(&ok).is_none());

        let err = Frame {
            frame_type: FrameType::Response,
            command_code: ERROR_FRAME_CODE,
            data: vec![0x09],
        };
        assert_eq!(
            error_frame_to_command_error(&err),
            Some(crate::core::error::CommandError(0x09))
        );
    }
}
