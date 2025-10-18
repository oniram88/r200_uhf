use crate::frame::SerializableCommand;
use crate::frame::{Command, FrameError};
use std::fmt::Display;

pub struct Packet {
    raw_data: Vec<u8>,
}

impl Packet {
    pub(crate) fn new(raw_data: Vec<u8>) -> Packet {
        Packet { raw_data }
    }
    fn frame_type(&self) -> u8 {
        self.raw_data[1]
    }
    fn command_code(&self) -> u8 {
        self.raw_data[2]
    }
    fn data_len(&self) -> u16 {
        ((self.raw_data[3] as u16) << 8) | (self.raw_data[4] as u16)
    }

    pub(crate) fn get_data(&self) -> Vec<u8> {
        let data = &self.raw_data[5..(5 + self.data_len() as usize)];
        data.to_vec()
    }

    pub(crate) fn debug(&self) -> String {
        format!(
            "Tipo: {:02X}, Comando: {:02X}, Lunghezza: {} - Dato: {:?}",
            self.frame_type(),
            self.command_code(),
            self.data_len(),
            self.get_data()
        )
    }

    fn command(&self) -> Result<Command, FrameError> {
        Command::from_tuple((vec![self.command_code()], vec![self.raw_data[5]]))
    }
}

impl Display for Packet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let out = {
            if let Ok(text) = std::str::from_utf8(&*self.get_data()) {
                text.to_string()
            } else {
                "Invalid UTF-8".to_string()
            }
        };
        write!(f, "{out}")
    }
}
