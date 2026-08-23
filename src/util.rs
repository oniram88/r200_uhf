pub trait PushU16 {
    fn push_u16(&mut self, val: u16);
}

impl PushU16 for Vec<u8> {
    fn push_u16(&mut self, val: u16) {
        self.push((val >> 8) as u8);
        self.push((val & 0xFF) as u8);
    }
}

/// Encode bytes as a lowercase hex string (no separators).
#[must_use]
pub fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Encode bytes as an uppercase hex string (no separators).
#[cfg(feature = "cli")]
#[must_use]
pub fn hex_upper(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02X}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_u16_big_endian() {
        let mut v = Vec::new();
        v.push_u16(0x1234);
        assert_eq!(v, [0x12, 0x34]);
    }

    #[test]
    fn hex_lower_encodes() {
        assert_eq!(hex_lower(&[0xAB, 0x0F]), "ab0f");
        assert_eq!(hex_lower(&[]), "");
    }

    #[cfg(feature = "cli")]
    #[test]
    fn hex_upper_encodes() {
        assert_eq!(hex_upper(&[0xAB, 0x0F]), "AB0F");
    }
}
