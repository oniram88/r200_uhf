use std::fmt::Display;

pub struct Rfid {
    pub rssi: u8,
    pub pc: u16,
    pub epc: Vec<u8>, // also known as the tag UID
    pub crc: u16,
}

impl Display for Rfid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RSSI: {}, PC: {}, EPC(UID): {:?}, CRC: {}",
            self.rssi, self.pc, self.epc, self.crc
        )
    }
}

impl Rfid {
    pub fn uid(&self) -> String {
        self.epc.iter().map(|b| format!("{:02x}", b)).collect()
    }
}
