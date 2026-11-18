//! Game Boy memory bus.
//!
//! Provides the address space accessed by the SM83 CPU.

use rookery_common::rom::Rom;
use tracing::warn;

/// Memory bus for the Game Boy and Game Boy Color.
///
/// Owns cartridge data and the memory arrays accessible
/// through the SM83's 16-bit address space.
#[derive(Debug, Clone)]
pub struct GbBus {
    cartridge: Rom,
    boot_rom: Option<[u8; 0x100]>,
    boot_mapped: bool,

    vram: [u8; 0x2000],
    wram: [u8; 0x2000],
    oam: [u8; 0xA0],

    io: [u8; 0x80],

    hram: [u8; 0x7F],
    ie: u8,
}

impl GbBus {
    /// Creates a memory bus containing the specified cartridge.
    ///
    /// All writeable memory and placeholder I/O registers are intially cleared to 0xFF.
    pub fn new(cartridge: Rom) -> Self {
        Self {
            cartridge,
            boot_rom: None,
            boot_mapped: false,

            vram: [0xFF; 0x2000],
            wram: [0xFF; 0x2000],
            oam: [0xFF; 0xA0],

            io: [0xFF; 0x80],

            hram: [0xFF; 0x7F],
            ie: 0,
        }
    }

    /// Creates a bus that begins with the given boot ROM
    /// mapped over cartridge addresses $0000-$00FF.
    pub fn with_boot_rom(cartridge: Rom, boot_rom: [u8; 0x100]) -> Self {
        let mut bus = Self::new(cartridge);

        bus.boot_rom = Some(boot_rom);
        bus.boot_mapped = true;

        bus
    }

    /// Returns a reference to the loaded cartridge
    pub fn cartridge(&self) -> &Rom {
        &self.cartridge
    }

    /// Returns whether the boot ROM is currently mapped.
    pub fn boot_rom_mapped(&self) -> bool {
        self.boot_mapped
    }
}

impl sm83::Bus for GbBus {
    fn read8(&mut self, address: u16) -> u8 {
        if self.boot_mapped && address <= 0x00FF {
            return self.boot_rom.as_ref().expect("mapped boot ROM must exist")
                [usize::from(address)];
        }

        match address {
            // Cartridge ROM
            0x0000..=0x7FFF => self
                .cartridge
                .bytes()
                .get(usize::from(address))
                .copied()
                .unwrap_or(0xFF),
            // VRAM
            0x8000..=0x9FFF => self.vram[usize::from(address - 0x8000)],
            // Cartridge RAM (not implemented)
            0xA000..=0xBFFF => 0xFF,
            // Work RAM
            0xC000..=0xDFFF => self.wram[usize::from(address - 0xC000)],
            // Echo RAM
            0xE000..=0xFDFF => self.wram[usize::from(address - 0xE000)],
            // Sprite attribute memory
            0xFE00..=0xFE9F => self.oam[usize::from(address - 0xFE00)],
            // Unusuable memory
            0xFEA0..=0xFEFF => {
                warn!("Attempted to read from illegal memory address ${address:04X}");
                0xFF
            }
            // I/O registers
            0xFF00..=0xFF7F => self.io[usize::from(address - 0xFF00)],
            // High RAM
            0xFF80..=0xFFFE => self.hram[usize::from(address - 0xFF80)],
            // Interrupt enable register
            0xFFFF => self.ie,
        }
    }

    fn write8(&mut self, address: u16, value: u8) {
        match address {
            // Cartridge ROM
            0x0000..=0x7FFF => warn!(
                "Attempted to write to cartridge ROM at ${address:04X} with value {value:02X}"
            ),
            0x8000..=0x9FFF => self.vram[usize::from(address - 0x8000)] = value,
            0xA000..=0xBFFF => {}
            0xC000..=0xDFFF => self.wram[usize::from(address - 0xC000)] = value,
            0xE000..=0xFDFF => self.wram[usize::from(address - 0xE000)] = value,
            0xFE00..=0xFE9F => self.oam[usize::from(address - 0xFE00)] = value,
            0xFEA0..=0xFEFF => {
                warn!("Attempted to write to illegal address ${address:04X} with value {value:02X}")
            }
            0xFF50 => {
                // Boot ROM mappins is disabled by a nonzero write
                if value != 0 {
                    self.boot_mapped = false;
                }
                self.io[0x50] = value;
            }
            0xFF00..=0xFF7F => self.io[usize::from(address - 0xFF00)] = value,
            0xFF80..=0xFFFE => self.hram[usize::from(address - 0xFF80)] = value,
            0xFFFF => self.ie = value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rookery_common::Platform;
    use sm83::Bus;

    fn test_bus() -> GbBus {
        let mut bytes = vec![0; 0x8000];

        bytes[0x0100] = 0x3E;
        bytes[0x0101] = 0x42;

        let rom = Rom::from_bytes(bytes, Some(Platform::Gb)).unwrap();

        GbBus::new(rom)
    }

    #[test]
    fn reads_cartridge_rom() {
        let mut bus = test_bus();

        assert_eq!(bus.read8(0x0100), 0x3E);
        assert_eq!(bus.read8(0x0101), 0x42);
    }

    #[test]
    fn cartridge_rom_is_read_only() {
        let mut bus = test_bus();

        bus.write8(0x0100, 0xFF);

        assert_eq!(bus.read8(0x0100), 0x3E);
    }

    #[test]
    fn work_ram_is_writable() {
        let mut bus = test_bus();

        bus.write8(0xC123, 0xAB);

        assert_eq!(bus.read8(0xC123), 0xAB);
    }

    #[test]
    fn echo_ram_mirrors_work_ram() {
        let mut bus = test_bus();

        bus.write8(0xC123, 0x55);

        assert_eq!(bus.read8(0xE123), 0x55);

        bus.write8(0xE123, 0xAA);

        assert_eq!(bus.read8(0xC123), 0xAA);
    }

    #[test]
    fn hram_and_ie_are_addressable() {
        let mut bus = test_bus();

        bus.write8(0xFF80, 0x12);
        bus.write8(0xFFFF, 0x1F);

        assert_eq!(bus.read8(0xFF80), 0x12);
        assert_eq!(bus.read8(0xFFFF), 0x1F);
    }
}
