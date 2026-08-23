use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
#[repr(u8)]
pub enum Region {
    China900Mhz = 0x01,
    Us = 0x02,
    Eu = 0x03,
    China800Mhz = 0x04,
    Korea = 0x06,
}

impl Region {
    #[must_use]
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(Self::China900Mhz),
            0x02 => Some(Self::Us),
            0x03 => Some(Self::Eu),
            0x04 => Some(Self::China800Mhz),
            0x06 => Some(Self::Korea),
            _ => None,
        }
    }

    /// Channel center frequency in MHz for the given channel index.
    ///
    /// Formulas per R200 protocol spec §14 (`Freq_CH = CH_Index * step + base`).
    #[must_use]
    pub fn channel_frequency(self, channel: u8) -> f64 {
        let ch = f64::from(channel);
        match self {
            Self::China900Mhz => ch.mul_add(0.25, 920.125),
            Self::China800Mhz => ch.mul_add(0.25, 840.125),
            Self::Us => ch.mul_add(0.5, 902.25),
            Self::Eu => ch.mul_add(0.2, 865.1),
            Self::Korea => ch.mul_add(0.2, 917.1),
        }
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::China900Mhz => write!(f, "China 900 MHz"),
            Self::China800Mhz => write!(f, "China 800 MHz"),
            Self::Us => write!(f, "US 902-928 MHz"),
            Self::Eu => write!(f, "EU 865-868 MHz"),
            Self::Korea => write!(f, "Korea 917-923 MHz"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_byte_valid() {
        assert_eq!(Region::from_byte(0x01), Some(Region::China900Mhz));
        assert_eq!(Region::from_byte(0x02), Some(Region::Us));
        assert_eq!(Region::from_byte(0x03), Some(Region::Eu));
        assert_eq!(Region::from_byte(0x04), Some(Region::China800Mhz));
        assert_eq!(Region::from_byte(0x06), Some(Region::Korea));
    }

    #[test]
    fn from_byte_invalid() {
        assert_eq!(Region::from_byte(0x00), None);
        assert_eq!(Region::from_byte(0x05), None);
        assert_eq!(Region::from_byte(0xFF), None);
    }

    #[test]
    fn channel_frequency_eu() {
        // spec §14: Freq_CH = CH_Index * 0.2 + 865.1
        assert!((Region::Eu.channel_frequency(0) - 865.1).abs() < 0.001);
        assert!((Region::Eu.channel_frequency(5) - 866.1).abs() < 0.001);
    }

    #[test]
    fn channel_frequency_us() {
        // spec §14: Freq_CH = CH_Index * 0.5 + 902.25
        assert!((Region::Us.channel_frequency(0) - 902.25).abs() < 0.001);
        assert!((Region::Us.channel_frequency(1) - 902.75).abs() < 0.001);
    }

    #[test]
    fn channel_frequency_china900() {
        assert!((Region::China900Mhz.channel_frequency(0) - 920.125).abs() < 0.001);
    }

    #[test]
    fn channel_frequency_china800() {
        assert!((Region::China800Mhz.channel_frequency(0) - 840.125).abs() < 0.001);
    }

    #[test]
    fn channel_frequency_korea() {
        assert!((Region::Korea.channel_frequency(0) - 917.1).abs() < 0.001);
    }

    #[test]
    fn display_format() {
        assert_eq!(Region::Eu.to_string(), "EU 865-868 MHz");
        assert_eq!(Region::Us.to_string(), "US 902-928 MHz");
    }
}
