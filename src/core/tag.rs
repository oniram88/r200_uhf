use std::fmt;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Tag {
    pub rssi: u8,
    pub pc: [u8; 2],
    pub epc: Vec<u8>,
    pub crc: [u8; 2],
}

impl PartialEq for Tag {
    fn eq(&self, other: &Self) -> bool {
        self.epc == other.epc
    }
}

impl Eq for Tag {}

impl Hash for Tag {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.epc.hash(state);
    }
}

impl Tag {
    #[must_use]
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 6 {
            return None;
        }
        let rssi = data[0];
        let pc = [data[1], data[2]];
        let crc = [data[data.len() - 2], data[data.len() - 1]];
        let epc = data[3..data.len() - 2].to_vec();
        Some(Self { rssi, pc, epc, crc })
    }

    #[must_use]
    pub fn uid(&self) -> &[u8] {
        &self.epc
    }

    #[must_use]
    pub fn epc_hex(&self) -> String {
        hex_bytes(&self.epc)
    }

    #[must_use]
    pub fn pc_hex(&self) -> String {
        hex_bytes(&self.pc)
    }

    #[must_use]
    pub fn crc_hex(&self) -> String {
        hex_bytes(&self.crc)
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Tag(EPC={} RSSI={} PC={} CRC={})",
            self.epc_hex(),
            self.rssi,
            self.pc_hex(),
            self.crc_hex(),
        )
    }
}

#[must_use]
pub fn hex_bytes(bytes: &[u8]) -> String {
    crate::util::hex_lower(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_tag() {
        // RSSI(1) + PC(2) + EPC(12) + CRC(2) = 17 bytes
        let data = vec![
            0xAB, 0x30, 0x00, 0xE2, 0x00, 0x10, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            0x99, 0x00, 0x00,
        ];
        let tag = Tag::parse(&data).unwrap();
        assert_eq!(tag.rssi, 0xAB);
        assert_eq!(tag.pc, [0x30, 0x00]);
        assert_eq!(tag.epc.len(), 12);
        assert_eq!(tag.epc[0], 0xE2);
        assert_eq!(tag.crc, [0x00, 0x00]);
    }

    #[test]
    fn parse_minimum_tag() {
        // RSSI(1) + PC(2) + EPC(1) + CRC(2) = 6 bytes
        let data = vec![0x50, 0x30, 0x00, 0xAA, 0x12, 0x34];
        let tag = Tag::parse(&data).unwrap();
        assert_eq!(tag.epc, vec![0xAA]);
    }

    #[test]
    fn parse_rejects_too_short() {
        assert!(Tag::parse(&[0x01, 0x02, 0x03]).is_none());
        assert!(Tag::parse(&[]).is_none());
    }

    #[test]
    fn uid_returns_epc() {
        let data = vec![0x50, 0x30, 0x00, 0xAA, 0xBB, 0x12, 0x34];
        let tag = Tag::parse(&data).unwrap();
        assert_eq!(tag.uid(), &[0xAA, 0xBB]);
    }

    #[test]
    fn epc_hex_format() {
        let tag = Tag {
            rssi: 0x50,
            pc: [0x30, 0x00],
            epc: vec![0xE2, 0x00, 0x10],
            crc: [0x12, 0x34],
        };
        assert_eq!(tag.epc_hex(), "e20010");
    }

    #[test]
    fn display_format() {
        let tag = Tag {
            rssi: 0x50,
            pc: [0x30, 0x00],
            epc: vec![0xAA, 0xBB],
            crc: [0x12, 0x34],
        };
        let s = tag.to_string();
        assert!(s.contains("aabb"));
        assert!(s.contains("80")); // 0x50 = 80 decimal
    }

    #[test]
    fn hash_and_eq_by_epc() {
        use std::collections::HashSet;
        let t1 = Tag {
            rssi: 0x10,
            pc: [0, 0],
            epc: vec![0xAA, 0xBB],
            crc: [0, 0],
        };
        let t2 = Tag {
            rssi: 0x20,
            pc: [0, 0],
            epc: vec![0xAA, 0xBB],
            crc: [0, 0],
        };
        let t3 = Tag {
            rssi: 0x10,
            pc: [0, 0],
            epc: vec![0xCC, 0xDD],
            crc: [0, 0],
        };

        assert_eq!(t1, t2);
        assert_ne!(t1, t3);

        let mut set = HashSet::new();
        set.insert(t1);
        set.insert(t2); // same EPC as t1, should not increase set size
        set.insert(t3);
        assert_eq!(set.len(), 2);
    }
}
