use std::fmt::{Display, Formatter};

/// Known R200 constants
pub const R200_FRAME_HEADER: u8 = 0xAA;
pub const R200_FRAME_END: u8 = 0xDD;

/// Frame type:
const FRAME_TYPE_SEND_COMMAND: u8 = 0x00; // from PC to R200
const INSTRUCTION_READER_WRITER_MODULE_INFO: u8 = 0x03; // Get reader/writer module information

#[derive(Debug)]
pub enum FrameError {
    InvalidCommand(String),
}

impl Display for FrameError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::InvalidCommand(msg) => write!(f, "Invalid command: {}", msg),
        }
    }
}

impl std::error::Error for FrameError {}

pub(crate) enum Command {
    GetWorkingChannel,
    GetWorkingArea,
    AcquireTransmitPower,
    SetTrasmissionPower(f64),
    HardwareVersion,
    SoftwareVersion,
    Manufacturer,
    SinglePollingInstruction,
}

impl Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::HardwareVersion => write!(f, "Hardware Version"),
            Command::SoftwareVersion => write!(f, "Software Version"),
            Command::Manufacturer => write!(f, "Manufacturer"),
            Command::GetWorkingChannel => write!(f, "Get Working Channel"),
            Command::GetWorkingArea => write!(f, "Get Working Area"),
            Command::AcquireTransmitPower => write!(f, "Acquire transmit power"),
            Command::SetTrasmissionPower(power) => write!(f, "Set transmission power to {}", power),
            Command::SinglePollingInstruction => write!(f, "Single Polling Instruction"),
        }
    }
}

/// Trait for serializable commands
pub(crate) trait SerializableCommand {
    /// Returns a tuple of bytes (command, parameters)
    /// Parameters may be empty if not present
    fn to_bytes(&self) -> (Vec<u8>, Vec<u8>);
    fn from_tuple(tuple: (Vec<u8>, Vec<u8>)) -> Result<Self, FrameError>
    where
        Self: Sized;
}

const READ_WRITE_INFO_HARDWARE_VERSION: u8 = 0x00;
const READ_WRITE_INFO_SOFTWARE_VERSION: u8 = 0x01;
const READ_WRITE_INFO_MANUFACTURER: u8 = 0x02;

impl SerializableCommand for Command {
    fn to_bytes(&self) -> (Vec<u8>, Vec<u8>) {
        match self {
            Command::HardwareVersion => (
                vec![INSTRUCTION_READER_WRITER_MODULE_INFO],
                vec![READ_WRITE_INFO_HARDWARE_VERSION],
            ), //Command::HardwareVersion
            Command::SoftwareVersion => (
                vec![INSTRUCTION_READER_WRITER_MODULE_INFO],
                vec![READ_WRITE_INFO_SOFTWARE_VERSION],
            ), //Command::SoftwareVersion
            Command::Manufacturer => (
                vec![INSTRUCTION_READER_WRITER_MODULE_INFO],
                vec![READ_WRITE_INFO_MANUFACTURER],
            ), //Command::Manufacturer
            Command::GetWorkingChannel => (vec![0xAA], vec![]),
            Command::GetWorkingArea => (vec![0x08], vec![]),
            Command::AcquireTransmitPower => (vec![0xB7], vec![]),
            Command::SetTrasmissionPower(p) => {
                let power = (p * 100.0) as u16;
                let mut v = Vec::new();
                v.push((power >> 8) as u8);
                v.push((power & 0xFF) as u8);
                (vec![0xB6], v)
            }
            Command::SinglePollingInstruction => (vec![0x22], vec![]),
        }
    }

    fn from_tuple(tuple: (Vec<u8>, Vec<u8>)) -> Result<Self, FrameError> {
        match (tuple.0[0], tuple.1[0]) {
            (INSTRUCTION_READER_WRITER_MODULE_INFO, READ_WRITE_INFO_HARDWARE_VERSION) => {
                Ok(Command::HardwareVersion)
            }
            (INSTRUCTION_READER_WRITER_MODULE_INFO, READ_WRITE_INFO_SOFTWARE_VERSION) => {
                Ok(Command::SoftwareVersion)
            }
            (INSTRUCTION_READER_WRITER_MODULE_INFO, READ_WRITE_INFO_MANUFACTURER) => {
                Ok(Command::Manufacturer)
            }
            (INSTRUCTION_READER_WRITER_MODULE_INFO, _) => Err(FrameError::InvalidCommand(format!(
                "Invalid command code: {}",
                tuple.1[0]
            ))),
            (0xAA, _) => Ok(Command::GetWorkingChannel),
            (0x08, _) => Ok(Command::GetWorkingArea),
            (0xB7, _) => Ok(Command::AcquireTransmitPower),
            _ => Err(FrameError::InvalidCommand(format!(
                "Invalid command code: {}",
                tuple.0[0]
            ))),
        }
    }
}

pub(crate) struct Frame {
    payload: Vec<u8>,
}

impl Frame {
    pub(crate) fn new(payload: &Command) -> Self {
        let mut v = Vec::new();
        // command
        v.extend(payload.to_bytes().0);
        let payload_size = payload.to_bytes().1.len() as u16;
        v.push((payload_size >> 8) as u8);
        v.push((payload_size & 0xFF) as u8);
        v.extend(payload.to_bytes().1);

        Frame { payload: v }
    }

    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.push(R200_FRAME_HEADER);
        v.push(FRAME_TYPE_SEND_COMMAND);

        v.extend(&self.payload);

        v.push(self.checksum(&v[2..]));
        v.push(R200_FRAME_END);
        v
    }

    fn checksum(&self, bytes: &[u8]) -> u8 {
        let sum: u16 = bytes.iter().map(|&b| b as u16).sum();
        (sum & 0xFF) as u8
    }
}
