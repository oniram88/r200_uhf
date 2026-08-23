use crate::Tag;

pub fn display_tag(tag: &Tag) {
    println!(
        "  EPC: {}  RSSI: {}  PC: {}  CRC: {}",
        tag.epc_hex(),
        tag.rssi,
        tag.pc_hex(),
        tag.crc_hex(),
    );
}

pub fn hex(bytes: &[u8]) -> String {
    crate::util::hex_lower(bytes)
}
