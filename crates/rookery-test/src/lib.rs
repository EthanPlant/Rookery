/// Headless PPU screenshot regression runner.
pub mod acid2;
/// Machine-level test-ROM runners.
pub mod blargg;

use rookery_common::{
    Platform,
    rom::{Rom, RomHeader},
};

#[derive(Debug)]
pub struct TestResult {
    pub name: &'static str,
    pub passed: bool,
}

pub fn run_header_tests() -> Vec<TestResult> {
    let mut results = Vec::new();

    // Synthetic Game Boy cartridge.
    let mut gb = vec![0u8; 0x8000];
    gb[0x134..0x138].copy_from_slice(b"TEST");
    gb[0x148] = 0x00;

    let checksum = gb[0x134..=0x14C]
        .iter()
        .fold(0u8, |acc, &byte| acc.wrapping_sub(byte).wrapping_sub(1));
    gb[0x14D] = checksum;

    let gb_passed = match Rom::from_bytes(gb, Some(Platform::Gb)) {
        Ok(rom) => match rom.header() {
            RomHeader::Gb(header) => {
                header.title == "TEST"
                    && header.header_checksum_valid
                    && header.declared_rom_size() == Some(32768)
            }
            _ => false,
        },
        Err(_) => false,
    };
    results.push(TestResult {
        name: "GB cartridge header",
        passed: gb_passed,
    });

    // Synthetic GBA cartridge
    let mut gba = vec![0u8; 0xC0];
    gba[0xA0..0xA4].copy_from_slice(b"TEST");
    gba[0xAC..0xB0].copy_from_slice(b"TST1");
    gba[0xB2] = 0x96;

    gba[0xBD] = gba[0xA0..=0xBC]
        .iter()
        .fold(0xE7, |acc, &byte| acc.wrapping_sub(byte));

    let gba_passed = match Rom::from_bytes(gba, Some(Platform::Agb)) {
        Ok(rom) => match rom.header() {
            RomHeader::Agb(header) => {
                header.title == "TEST" && header.game_code == "TST1" && header.header_checksum_valid
            }
            _ => false,
        },
        Err(_) => false,
    };
    results.push(TestResult {
        name: "GBA cartridge header",
        passed: gba_passed,
    });

    // Invalid rom
    let invalid = Rom::from_bytes(vec![0u8; 32], None);
    results.push(TestResult {
        name: "Reject truncated ROM",
        passed: invalid.is_err(),
    });

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_suite_passes() {
        for result in run_header_tests() {
            assert!(result.passed, "test failed: {}", result.name);
        }
    }
}
