use std::fmt::Display;
use std::hash::Hash;

#[derive(Clone)]
pub struct Rfid {
    pub rssi: u8,
    pub pc: String,
    pub epc: String, // also known as the tag UID
    pub crc: String,
    pub(crate) raw: Vec<u8>,
}

impl Rfid {
    pub(crate) fn from_raw(raw: Vec<u8>) -> Rfid {
        let rssi = raw[0];

        Self {
            pc: bytes_to_hex_upper(&raw[1..3].to_vec()),
            epc: bytes_to_hex_upper(&raw[3..15]),
            crc: bytes_to_hex_upper(&raw[15..17].to_vec()),
            rssi,
            raw,
        }
    }
}

impl Hash for Rfid {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.epc.hash(state);
    }
}

impl PartialEq<Self> for Rfid {
    fn eq(&self, other: &Self) -> bool {
        self.epc == other.epc
    }
}
impl Eq for Rfid {}

impl Display for Rfid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RSSI: {}, PC: {}, EPC(UID): {:?}, CRC: {}, RAW: {}",
            self.rssi,
            self.pc,
            self.epc,
            self.crc,
            bytes_to_hex_upper(&self.raw)
        )
    }
}

impl Rfid {
    pub fn uid(&self) -> String {
        self.epc.clone()
    }
}

fn bytes_to_hex_upper(bytes: &[u8]) -> String {
    // usa formatting manuale per performance / controllo
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02X}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parsing_rfid() {
        let intake = "BC3000E28069150000501D63E2784FB0B7";

        let bytes: Vec<u8> = (0..intake.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&intake[i..i + 2], 16).unwrap())
            .collect();

        let packet = Rfid::from_raw(bytes);

        assert_eq!(packet.rssi, 0xBC);
        assert_eq!(packet.pc, "3000");
        assert_eq!(packet.epc, "E28069150000501D63E2784F");
        assert_eq!(packet.crc, "B0B7");
    }
}
