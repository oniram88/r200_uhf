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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uid_returns_lowercase_hex_concatenation() {
        let r = Rfid { rssi: 50, pc: 0x1234, epc: vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02], crc: 0xABCD };
        // expect lowercase, two chars per byte, no separators
        assert_eq!(r.uid(), "deadbeef0102");
    }

    #[test]
    fn uid_handles_empty_epc() {
        let r = Rfid { rssi: 0, pc: 0, epc: vec![], crc: 0 };
        assert_eq!(r.uid(), "");
    }

    #[test]
    fn display_formats_all_fields() {
        let r = Rfid { rssi: 77, pc: 0x3344, epc: vec![0x01, 0x02, 0xA0], crc: 0x5566 };
        let s = format!("{}", r);
        // Display uses debug formatting for EPC vector
        assert_eq!(s, "RSSI: 77, PC: 13124, EPC(UID): [1, 2, 160], CRC: 21862");
    }
}



