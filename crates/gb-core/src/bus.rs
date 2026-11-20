//! Game Boy memory bus.
//!
//! Provides the address space accessed by the SM83 CPU.

use rookery_common::rom::Rom;
use tracing::{info, warn};

use crate::timer::Timer;

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
    // Bytes emitted by the temporary Blargg SB/SC test protocol.
    serial_output: Vec<u8>,

    timer: Timer,

    // TEMP:
    ly: u8,
    line_cycles: u32,
}

impl GbBus {
    /// Creates a memory bus containing the specified cartridge.
    ///
    /// All writeable memory and placeholder I/O registers are intially cleared to 0xFF.
    pub fn new(cartridge: Rom) -> Self {
        let mut io = [0xFF; 0x80];
        io[0x0F] = 0; // Initialize IF to zero pending interrupts.

        Self {
            cartridge,
            boot_rom: None,
            boot_mapped: false,

            vram: [0xFF; 0x2000],
            wram: [0xFF; 0x2000],
            oam: [0xFF; 0xA0],

            io,

            hram: [0xFF; 0x7F],
            ie: 0,
            serial_output: Vec::new(),

            timer: Timer::new(),

            ly: 0,
            line_cycles: 0,
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

    pub fn advance_cycles(&mut self, cycles: u32) {
        if self.timer.tick(cycles) {
            // Request timer interrupt.
            self.io[0x0F] |= 0x04;
        }

        // TEMP: Scanline simulation until the PPU exists.
        if self.io[0x40] & 0x80 == 0 {
            self.ly = 0;
            self.line_cycles = 0;
            return;
        }

        self.line_cycles += cycles;

        while self.line_cycles >= 456 {
            self.line_cycles -= 456;
            self.ly = (self.ly + 1) % 154;
        }
    }

    /// Returns a reference to the loaded cartridge
    pub fn cartridge(&self) -> &Rom {
        &self.cartridge
    }

    /// Returns whether the boot ROM is currently mapped.
    pub fn boot_rom_mapped(&self) -> bool {
        self.boot_mapped
    }

    /// Bytes emitted through Blargg's temporary SB/SC serial convention.
    /// This is not yet a full Game Boy serial peripheral implementation.
    pub fn serial_output(&self) -> &[u8] {
        &self.serial_output
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
            0xFF04..=0xFF07 => self.timer.read(address),
            0xFF0F => self.io[0x0F] | 0xE0, // Interrupt flags: unused upper bits read as 1.
            0xFF44 => self.ly,
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
            // Temporary workaround for serial output for Blargg tests
            0xFF02 => {
                self.io[0x02] = value;

                if value == 0x81 {
                    self.serial_output.push(self.io[0x01]);
                }
            }
            0xFF04..=0xFF07 => self.timer.write(address, value),
            0xFF0F => self.io[0x0F] = value & 0x1F,
            0xFF44 => {
                warn!("Attempted to write to read-only LY register")
            }
            0xFF50 => {
                // Boot ROM mapping is disabled by a nonzero write
                if value != 0 {
                    info!("Boot ROM unmapped!");
                    self.boot_mapped = false;
                }
                self.io[0x50] = value;
            }
            0xFF00..=0xFF7F => self.io[usize::from(address - 0xFF00)] = value,
            0xFF80..=0xFFFE => self.hram[usize::from(address - 0xFF80)] = value,
            0xFFFF => self.ie = value,
        }
    }

    fn pending_interrupts(&self) -> u8 {
        self.ie & self.io[0x0F] & 0x1F
    }

    fn acknowledge_interrupt(&mut self, mask: u8) {
        self.io[0x0F] &= !mask
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
    fn captures_blargg_serial_protocol() {
        let mut bus = test_bus();
        bus.write8(0xFF01, b'P');
        bus.write8(0xFF02, 0x81);
        bus.write8(0xFF01, b'a');
        bus.write8(0xFF02, 0x81);
        assert_eq!(bus.serial_output(), b"Pa");
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

    #[test]
    fn timer_overflow_requests_interrupt() {
        let mut bus = test_bus();

        // Timer enabled, 16 T-cycles per increment.
        bus.write8(0xFF07, 0x05);

        // Overflow on the first increment.
        bus.write8(0xFF05, 0xFF);

        // Reload TIMA with 0x42.
        bus.write8(0xFF06, 0x42);

        // Clear interrupt requests.
        bus.write8(0xFF0F, 0);

        // After 16 cycles, TIMA overflows to zero.
        bus.advance_cycles(16);

        assert_eq!(bus.read8(0xFF05), 0);
        assert_eq!(bus.read8(0xFF0F) & 0x04, 0);

        // Three more cycles: still pending.
        bus.advance_cycles(3);

        assert_eq!(bus.read8(0xFF0F) & 0x04, 0);

        // Fourth cycle: reload and interrupt.
        bus.advance_cycles(1);

        assert_eq!(bus.read8(0xFF05), 0x42);
        assert_eq!(bus.read8(0xFF0F) & 0x04, 0x04);
    }

    #[test]
    fn timer_interrupt_flag_can_be_cleared() {
        let mut bus = test_bus();

        bus.write8(0xFF07, 0x05);
        bus.write8(0xFF05, 0xFF);
        bus.write8(0xFF0F, 0);

        bus.advance_cycles(20);

        assert_ne!(bus.read8(0xFF0F) & 0x04, 0);

        bus.write8(0xFF0F, 0);

        assert_eq!(bus.read8(0xFF0F) & 0x04, 0);
    }

    #[test]
    fn div_continues_while_lcd_is_disabled() {
        let mut bus = test_bus();

        bus.write8(0xFF40, 0);
        bus.advance_cycles(512);

        assert_eq!(bus.read8(0xFF04), 2);
        assert_eq!(bus.read8(0xFF44), 0);
    }
}
