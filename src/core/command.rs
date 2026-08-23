use std::fmt;

use crate::core::error::CoreError;
use crate::util::PushU16;

pub trait Command {
    type Response;

    const CODE: u8;

    fn encode(&self) -> Vec<u8>;

    /// Decode a response frame's data payload into the command-specific response type.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::InvalidPayload`] if the response data is malformed
    /// or too short for this command.
    fn decode_response(&self, data: &[u8]) -> Result<Self::Response, CoreError>;
}

// ── Module info ─────────────────────────────────────

pub struct GetModuleInfo {
    pub param: ModuleInfoParam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
#[repr(u8)]
pub enum ModuleInfoParam {
    HardwareVersion = 0x00,
    SoftwareVersion = 0x01,
    Manufacturer = 0x02,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ModuleInfoResponse {
    pub param: ModuleInfoParam,
    pub text: String,
}

impl Command for GetModuleInfo {
    type Response = ModuleInfoResponse;
    const CODE: u8 = 0x03;

    fn encode(&self) -> Vec<u8> {
        vec![self.param as u8]
    }

    fn decode_response(&self, data: &[u8]) -> Result<ModuleInfoResponse, CoreError> {
        Ok(ModuleInfoResponse {
            param: self.param,
            text: String::from_utf8_lossy(data).to_string(),
        })
    }
}

// ── Polling ─────────────────────────────────────────

pub struct SinglePollingInstruction;

impl Command for SinglePollingInstruction {
    type Response = Option<crate::core::tag::Tag>;
    const CODE: u8 = 0x22;

    fn encode(&self) -> Vec<u8> {
        vec![]
    }

    fn decode_response(&self, data: &[u8]) -> Result<Option<crate::core::tag::Tag>, CoreError> {
        Ok(crate::core::tag::Tag::parse(data))
    }
}

pub struct MultiplePollingInstruction {
    pub pool_times: u16,
}

impl Command for MultiplePollingInstruction {
    type Response = ();
    const CODE: u8 = 0x27;

    fn encode(&self) -> Vec<u8> {
        // Spec §3: PL=0x0003, Reserved(1)=0x22, CNT(2). The reserved byte is
        // mandatory — without it the device misparses CNT and ignores the
        // requested poll count (verified on device: CNT=1 yields one inventory
        // round only when the 0x22 byte is present).
        let mut buf = Vec::with_capacity(3);
        buf.push(0x22);
        buf.push_u16(self.pool_times);
        buf
    }

    fn decode_response(&self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
}

pub struct StopMultiplePolling;

impl Command for StopMultiplePolling {
    type Response = ();
    const CODE: u8 = 0x28;

    fn encode(&self) -> Vec<u8> {
        vec![]
    }

    fn decode_response(&self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
}

// ── Select ──────────────────────────────────────────

/// `Select` command parameters (spec §5, command 0x0C).
///
/// The payload is `SelParam(1)`, `Ptr(4)` (a bit offset), `MaskLen(1)` (a bit
/// length), `Truncate(1)`, then `Mask(N)`. `SelParam` packs Target (3 bits),
/// Action (3 bits) and `MemBank` (2 bits) into a single byte.
pub struct SetSelect {
    /// Target (3 bits), Action (3 bits) and memory bank (2 bits), pre-packed.
    pub sel_param: u8,
    /// Mask start offset within the bank, in **bits**.
    pub ptr: u32,
    /// Truncate flag: `0x00` disables, `0x80` enables truncation.
    pub truncate: u8,
    /// The mask bytes to match against.
    pub mask: Vec<u8>,
}

impl SetSelect {
    /// Build a `Select` that matches a full EPC.
    ///
    /// Uses Target/Action `0`, the EPC bank, and a bit pointer of `0x20` (32),
    /// which is where the EPC bytes begin in the EPC memory bank (after the
    /// 16-bit CRC and 16-bit PC words).
    #[must_use]
    pub fn by_epc(epc: &[u8]) -> Self {
        // SelParam: Target(000) Action(000) MemBank(01 = EPC) => 0x01.
        Self {
            sel_param: 0x01,
            ptr: 0x20,
            truncate: 0x00,
            mask: epc.to_vec(),
        }
    }
}

impl Command for SetSelect {
    type Response = ();
    const CODE: u8 = 0x0C;

    fn encode(&self) -> Vec<u8> {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "mask bit length fits in u8 (max 255 bits per spec)"
        )]
        let mask_len_bits = (self.mask.len() * 8) as u8;
        let mut buf = Vec::with_capacity(7 + self.mask.len());
        buf.push(self.sel_param);
        buf.extend_from_slice(&self.ptr.to_be_bytes());
        buf.push(mask_len_bits);
        buf.push(self.truncate);
        buf.extend_from_slice(&self.mask);
        buf
    }

    fn decode_response(&self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
}

pub struct SetSendSelect(pub bool);

impl Command for SetSendSelect {
    type Response = ();
    const CODE: u8 = 0x12;

    fn encode(&self) -> Vec<u8> {
        vec![if self.0 { 0x02 } else { 0x01 }]
    }

    fn decode_response(&self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
}

// ── Memory access ───────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
#[repr(u8)]
pub enum MemBank {
    Reserved = 0x00,
    Epc = 0x01,
    Tid = 0x02,
    User = 0x03,
}

impl MemBank {
    #[must_use]
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x00 => Some(Self::Reserved),
            0x01 => Some(Self::Epc),
            0x02 => Some(Self::Tid),
            0x03 => Some(Self::User),
            _ => None,
        }
    }
}

impl fmt::Display for MemBank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reserved => write!(f, "Reserved"),
            Self::Epc => write!(f, "EPC"),
            Self::Tid => write!(f, "TID"),
            Self::User => write!(f, "User"),
        }
    }
}

pub struct ReadLabel {
    pub access_password: [u8; 4],
    pub bank: MemBank,
    pub address: u16,
    pub length: u16,
}

impl Command for ReadLabel {
    type Response = Vec<u8>;
    const CODE: u8 = 0x39;

    fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(9);
        buf.extend_from_slice(&self.access_password);
        buf.push(self.bank as u8);
        buf.push_u16(self.address);
        buf.push_u16(self.length);
        buf
    }

    fn decode_response(&self, data: &[u8]) -> Result<Vec<u8>, CoreError> {
        // Response layout (spec §6.4): UL(1) + PC(2) + EPC(UL-2) + Data(N),
        // where UL is the combined PC+EPC length in bytes. The read-back data
        // follows the echoed PC+EPC, so it starts at offset 1 + UL.
        let ul = *data
            .first()
            .ok_or_else(|| CoreError::InvalidPayload("read response missing UL byte".into()))?
            as usize;
        data.get(1 + ul..).map(<[u8]>::to_vec).ok_or_else(|| {
            CoreError::InvalidPayload(format!(
                "read response too short: UL={ul} but only {} bytes",
                data.len()
            ))
        })
    }
}

pub struct WriteLabel {
    pub access_password: [u8; 4],
    pub bank: MemBank,
    pub address: u16,
    pub data: Vec<u8>,
}

impl Command for WriteLabel {
    type Response = ();
    const CODE: u8 = 0x49;

    fn encode(&self) -> Vec<u8> {
        let word_count = self.data.len() / 2;
        let mut buf = Vec::with_capacity(9 + self.data.len());
        buf.extend_from_slice(&self.access_password);
        buf.push(self.bank as u8);
        buf.push_u16(self.address);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "RFID tag data fits in u16 words"
        )]
        buf.push_u16(word_count as u16);
        buf.extend_from_slice(&self.data);
        buf
    }

    fn decode_response(&self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
}

// ── Tag control ─────────────────────────────────────

pub struct KillTag {
    pub password: [u8; 4],
}

impl Command for KillTag {
    type Response = ();
    const CODE: u8 = 0x65;

    fn encode(&self) -> Vec<u8> {
        self.password.to_vec()
    }

    fn decode_response(&self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
}

pub struct LockTag {
    pub password: [u8; 4],
    pub lock_data: [u8; 3],
}

impl Command for LockTag {
    type Response = ();
    const CODE: u8 = 0x82;

    fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(7);
        buf.extend_from_slice(&self.password);
        buf.extend_from_slice(&self.lock_data);
        buf
    }

    fn decode_response(&self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
}

// ── Radio configuration ─────────────────────────────

pub struct GetWorkingArea;

impl Command for GetWorkingArea {
    type Response = crate::core::region::Region;
    const CODE: u8 = 0x08;

    fn encode(&self) -> Vec<u8> {
        vec![]
    }

    fn decode_response(&self, data: &[u8]) -> Result<crate::core::region::Region, CoreError> {
        let byte = data.first().copied().unwrap_or(0);
        crate::core::region::Region::from_byte(byte)
            .ok_or_else(|| CoreError::InvalidPayload(format!("unknown region byte 0x{byte:02X}")))
    }
}

pub struct SetWorkingArea(pub crate::core::region::Region);

impl Command for SetWorkingArea {
    type Response = ();
    const CODE: u8 = 0x07;

    fn encode(&self) -> Vec<u8> {
        vec![self.0 as u8]
    }

    fn decode_response(&self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
}

pub struct GetWorkingChannel;

impl Command for GetWorkingChannel {
    type Response = u8;
    const CODE: u8 = 0xAA;

    fn encode(&self) -> Vec<u8> {
        vec![]
    }

    fn decode_response(&self, data: &[u8]) -> Result<u8, CoreError> {
        data.first()
            .copied()
            .ok_or_else(|| CoreError::InvalidPayload("channel response is empty".into()))
    }
}

pub struct GetTransmitPower;

impl Command for GetTransmitPower {
    type Response = f64;
    const CODE: u8 = 0xB7;

    fn encode(&self) -> Vec<u8> {
        vec![]
    }

    fn decode_response(&self, data: &[u8]) -> Result<f64, CoreError> {
        if data.len() < 2 {
            return Err(CoreError::InvalidPayload(format!(
                "power response too short: {} bytes, need 2",
                data.len()
            )));
        }
        let raw = (u16::from(data[0]) << 8) | u16::from(data[1]);
        Ok(f64::from(raw) * 0.01)
    }
}

pub struct SetTransmitPower(pub f64);

impl Command for SetTransmitPower {
    type Response = ();
    const CODE: u8 = 0xB6;

    fn encode(&self) -> Vec<u8> {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "power 0.0..30.0 dBm * 100 fits in u16; validated by CLI"
        )]
        let raw = (self.0 * 100.0) as u16;
        let mut buf = Vec::with_capacity(2);
        buf.push_u16(raw);
        buf
    }

    fn decode_response(&self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
}

// ── Tests ───────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_module_info_encode() {
        let cmd = GetModuleInfo {
            param: ModuleInfoParam::HardwareVersion,
        };
        assert_eq!(cmd.encode(), vec![0x00]);

        let cmd = GetModuleInfo {
            param: ModuleInfoParam::SoftwareVersion,
        };
        assert_eq!(cmd.encode(), vec![0x01]);

        let cmd = GetModuleInfo {
            param: ModuleInfoParam::Manufacturer,
        };
        assert_eq!(cmd.encode(), vec![0x02]);
    }

    #[test]
    fn get_module_info_decode() {
        let cmd = GetModuleInfo {
            param: ModuleInfoParam::SoftwareVersion,
        };
        let resp = cmd.decode_response(b"V2.3.5").unwrap();
        assert_eq!(resp.text, "V2.3.5");
    }

    #[test]
    fn single_polling_encode() {
        assert!(SinglePollingInstruction.encode().is_empty());
    }

    #[test]
    fn single_polling_decode_empty() {
        let resp = SinglePollingInstruction.decode_response(&[]).unwrap();
        assert!(resp.is_none());
    }

    #[test]
    fn single_polling_decode_tag() {
        let data = vec![
            0xAB, 0x30, 0x00, 0xE2, 0x00, 0x10, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            0x99, 0x00, 0x00,
        ];
        let tag = SinglePollingInstruction
            .decode_response(&data)
            .unwrap()
            .unwrap();
        assert_eq!(tag.rssi, 0xAB);
    }

    #[test]
    fn multi_polling_encode() {
        // Spec §3: reserved byte 0x22 followed by big-endian CNT.
        let cmd = MultiplePollingInstruction { pool_times: 1000 };
        assert_eq!(cmd.encode(), vec![0x22, 0x03, 0xE8]);
    }

    #[test]
    fn stop_multi_polling_encode() {
        assert!(StopMultiplePolling.encode().is_empty());
    }

    #[test]
    fn set_select_encode() {
        // Spec §5 example: SelParam=0x01, Ptr=0x00000020, MaskLen=0x18 (3
        // bytes = 24 bits), Truncate=0x00, then the mask bytes.
        let cmd = SetSelect::by_epc(&[0xE2, 0x00, 0x10]);
        assert_eq!(
            cmd.encode(),
            vec![0x01, 0x00, 0x00, 0x00, 0x20, 0x18, 0x00, 0xE2, 0x00, 0x10]
        );
    }

    #[test]
    fn set_select_mask_len_is_bits() {
        // 12-byte EPC => 96 bits => 0x60.
        let cmd = SetSelect::by_epc(&[0xAB; 12]);
        assert_eq!(cmd.encode()[5], 0x60);
    }

    #[test]
    fn set_send_select_encode() {
        assert_eq!(SetSendSelect(true).encode(), vec![0x02]);
        assert_eq!(SetSendSelect(false).encode(), vec![0x01]);
    }

    #[test]
    fn read_label_encode() {
        let cmd = ReadLabel {
            access_password: [0x00; 4],
            bank: MemBank::Epc,
            address: 0x0000,
            length: 4,
        };
        assert_eq!(
            cmd.encode(),
            vec![0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04]
        );
    }

    #[test]
    fn read_label_decode_strips_pc_epc() {
        // Spec §6.4 response: UL(1) + PC(2) + EPC(UL-2) + Data(N).
        // UL = 0x0E = 14 = PC(2) + EPC(12); data follows at offset 1 + UL.
        let data = vec![
            0x0E, // UL
            0x30, 0x00, // PC
            0xE2, 0x80, 0x68, 0x94, 0x00, 0x00, 0x40, 0x1A, 0x48, 0xB8, 0xA1, 0x01, // EPC
            0xAA, 0xBB, 0xCC, 0xDD, // read-back data
        ];
        let resp = ReadLabel {
            access_password: [0; 4],
            bank: MemBank::Epc,
            address: 0,
            length: 0,
        }
        .decode_response(&data)
        .unwrap();
        assert_eq!(resp, vec![0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn read_label_decode_rejects_short() {
        let cmd = ReadLabel {
            access_password: [0; 4],
            bank: MemBank::Epc,
            address: 0,
            length: 0,
        };
        // Empty payload: no UL byte.
        assert!(cmd.decode_response(&[]).is_err());
        // UL declares more PC+EPC than present.
        assert!(cmd.decode_response(&[0x0E, 0x30, 0x00]).is_err());
    }

    #[test]
    fn write_label_encode() {
        let cmd = WriteLabel {
            access_password: [0x00; 4],
            bank: MemBank::Epc,
            address: 0x0000,
            data: vec![0xAA, 0xBB, 0xCC, 0xDD],
        };
        let enc = cmd.encode();
        assert_eq!(enc[4], 0x01); // bank
        assert_eq!(enc[7], 0x00); // word count high
        assert_eq!(enc[8], 0x02); // word count low (4 bytes = 2 words)
        assert_eq!(&enc[9..], &[0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn kill_tag_encode() {
        let cmd = KillTag {
            password: [0x11, 0x22, 0x33, 0x44],
        };
        assert_eq!(cmd.encode(), vec![0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    fn lock_tag_encode() {
        let cmd = LockTag {
            password: [0x11, 0x22, 0x33, 0x44],
            lock_data: [0x02, 0x00, 0x80],
        };
        assert_eq!(cmd.encode(), vec![0x11, 0x22, 0x33, 0x44, 0x02, 0x00, 0x80]);
    }

    #[test]
    fn get_working_area_encode() {
        assert!(GetWorkingArea.encode().is_empty());
    }

    #[test]
    fn get_working_area_decode() {
        let resp = GetWorkingArea.decode_response(&[0x03]).unwrap();
        assert_eq!(resp, crate::core::region::Region::Eu);
    }

    #[test]
    fn set_working_area_encode() {
        let cmd = SetWorkingArea(crate::core::region::Region::Us);
        assert_eq!(cmd.encode(), vec![0x02]);
    }

    #[test]
    fn get_working_channel_encode() {
        assert!(GetWorkingChannel.encode().is_empty());
    }

    #[test]
    fn get_working_channel_decode() {
        let resp = GetWorkingChannel.decode_response(&[0x05]).unwrap();
        assert_eq!(resp, 5);
    }

    #[test]
    fn get_transmit_power_encode() {
        assert!(GetTransmitPower.encode().is_empty());
    }

    #[test]
    fn get_transmit_power_decode() {
        let resp = GetTransmitPower.decode_response(&[0x09, 0x9C]).unwrap();
        assert!((resp - 24.6).abs() < 0.01);
    }

    #[test]
    fn set_transmit_power_encode() {
        let cmd = SetTransmitPower(23.6);
        let enc = cmd.encode();
        assert_eq!(enc.len(), 2);
        let raw = ((enc[0] as u16) << 8) | (enc[1] as u16);
        assert_eq!(raw, 2360);
    }

    #[test]
    fn mem_bank_display() {
        assert_eq!(MemBank::Epc.to_string(), "EPC");
        assert_eq!(MemBank::Tid.to_string(), "TID");
        assert_eq!(MemBank::User.to_string(), "User");
        assert_eq!(MemBank::Reserved.to_string(), "Reserved");
    }

    #[test]
    fn mem_bank_from_byte() {
        assert_eq!(MemBank::from_byte(0x01), Some(MemBank::Epc));
        assert_eq!(MemBank::from_byte(0xFF), None);
    }
}
