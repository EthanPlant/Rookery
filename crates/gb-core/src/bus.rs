//! Game Boy memory bus.
//!
//! Provides the address space accessed by the SM83 CPU.

use rookery_common::{Buttons, Framebuffer};
use sm83::Bus;
use tracing::{debug, info};

use crate::{
    cartridge::{Cartridge, CartridgeError},
    joypad::Joypad,
    ppu::Ppu,
    timer::Timer,
};

/// Memory bus for the Game Boy and Game Boy Color.
///
/// Owns cartridge data and the memory arrays accessible
/// through the SM83's 16-bit address space.
#[derive(Debug, Clone)]
pub struct GbBus {
    cartridge: Cartridge,
    boot_rom: Option<[u8; 0x100]>,
    boot_mapped: bool,

    wram: [u8; 0x2000],

    io: [u8; 0x80],

    hram: [u8; 0x7F],
    ie: u8,
    // Bytes emitted by the temporary Blargg SB/SC test protocol.
    serial_output: Vec<u8>,

    timer: Timer,
    ppu: Ppu,
    joypad: Joypad,
}

impl GbBus {
    /// Creates a memory bus containing the specified cartridge.
    ///
    /// All writeable memory and placeholder I/O registers are intially cleared to 0xFF.
    pub fn new(cartridge: Cartridge) -> Self {
        let mut io = [0xFF; 0x80];
        io[0x0F] = 0; // Initialize IF to zero pending interrupts.

        Self {
            cartridge,
            boot_rom: None,
            boot_mapped: false,

            wram: [0xFF; 0x2000],

            io,

            hram: [0xFF; 0x7F],
            ie: 0,
            serial_output: Vec::new(),

            timer: Timer::new(),
            ppu: Ppu::new(true),
            joypad: Joypad::post_boot(),
        }
    }

    pub fn initialize_dmg_post_boot(&mut self) {
        for (register, value) in [
            (0x01, 0x00), // SB
            (0x02, 0x7E), // SC
            (0x0F, 0x01), // IF
        ] {
            self.io[register] = value;
        }

        self.timer.initialize_dmg_post_boot();
    }

    /// Creates a bus that begins with the given boot ROM
    /// mapped over cartridge addresses $0000-$00FF.
    pub fn with_boot_rom(cartridge: Cartridge, boot_rom: [u8; 0x100]) -> Self {
        let mut bus = Self::new(cartridge);

        bus.boot_rom = Some(boot_rom);
        bus.boot_mapped = true;
        bus.ppu = Ppu::new(false);
        bus.joypad = Joypad::new();

        bus
    }

    pub fn advance_cycles(&mut self, cycles: u32) {
        if self.timer.tick(cycles) {
            // Request timer interrupt.
            self.io[0x0F] |= 0x04;
        }

        let interrupts = self.ppu.tick(cycles);
        self.io[0x0F] |= interrupts
    }

    /// Returns a reference to the loaded cartridge
    pub fn cartridge(&self) -> &Cartridge {
        &self.cartridge
    }

    pub fn battery_ram(&self) -> Option<&[u8]> {
        self.cartridge.battery_ram()
    }

    pub fn load_battery_ram(&mut self, data: &[u8]) -> Result<(), CartridgeError> {
        self.cartridge.load_battery_ram(data)
    }

    pub fn battery_ram_dirty(&self) -> bool {
        self.cartridge.battery_ram_dirty()
    }

    pub fn acknowledge_battery_save(&mut self) {
        self.cartridge.acknowledge_battery_save();
    }

    pub fn framebuffer(&self) -> &Framebuffer {
        self.ppu.framebuffer()
    }

    pub fn frames_completed(&self) -> u64 {
        self.ppu.frames()
    }

    pub fn take_frame_ready(&mut self) -> bool {
        self.ppu.take_frame_ready()
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

    /// Reads VRAM without CPU bus restrictions
    pub fn debug_vram(&self) -> &[u8; 0x2000] {
        self.ppu.debug_vram()
    }

    /// Reads the background palette without a bus read.
    pub fn debug_bgp(&self) -> u8 {
        self.ppu.debug_bgp()
    }

    pub fn debug_lcdc(&self) -> u8 {
        self.ppu.debug_lcdc()
    }

    pub fn debug_scx(&self) -> u8 {
        self.ppu.debug_scx()
    }

    pub fn debug_scy(&self) -> u8 {
        self.ppu.debug_scy()
    }

    pub fn buttons(&self) -> Buttons {
        self.joypad.buttons()
    }

    pub fn set_buttons(&mut self, buttons: Buttons) {
        if self.joypad.set_buttons(buttons) {
            self.io[0x0F] |= 0x10;
        }
    }

    fn start_oam_dma(&mut self, source_page: u8) {
        self.io[0x46] = source_page;

        let source_base = u16::from(source_page) << 8;

        for offset in 0..0xA0 {
            let source = source_base.wrapping_add(offset);

            let value = self.read8(source);
            self.ppu.write_oam(0xFE00 + offset, value);
        }
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
            0x0000..=0x7FFF => self.cartridge.read(address),
            // VRAM
            0x8000..=0x9FFF => self.ppu.read_vram(address),
            // Cartridge RAM
            0xA000..=0xBFFF => self.cartridge.read(address),
            // Work RAM
            0xC000..=0xDFFF => self.wram[usize::from(address - 0xC000)],
            // Echo RAM
            0xE000..=0xFDFF => self.wram[usize::from(address - 0xE000)],
            // Sprite attribute memory
            0xFE00..=0xFE9F => self.ppu.read_oam(address),
            // Unusuable memory
            0xFEA0..=0xFEFF => {
                debug!("Attempted to read from illegal memory address ${address:04X}");
                0xFF
            }
            0xFF00 => self.joypad.read(),
            0xFF04..=0xFF07 => self.timer.read(address),
            0xFF0F => self.io[0x0F] | 0xE0, // Interrupt flags: unused upper bits read as 1.
            0xFF40..=0xFF45 | 0xFF47..=0xFF4B => self.ppu.read(address),
            // I/O registers
            0xFF01..=0xFF7F => self.io[usize::from(address - 0xFF00)],
            // High RAM
            0xFF80..=0xFFFE => self.hram[usize::from(address - 0xFF80)],
            // Interrupt enable register
            0xFFFF => self.ie,
        }
    }

    fn write8(&mut self, address: u16, value: u8) {
        match address {
            // Cartridge ROM
            0x0000..=0x7FFF => self.cartridge.write(address, value),
            0x8000..=0x9FFF => self.ppu.write_vram(address, value),
            0xA000..=0xBFFF => self.cartridge.write(address, value),
            0xC000..=0xDFFF => self.wram[usize::from(address - 0xC000)] = value,
            0xE000..=0xFDFF => self.wram[usize::from(address - 0xE000)] = value,
            0xFE00..=0xFE9F => self.ppu.write_oam(address, value),
            0xFEA0..=0xFEFF => {
                debug!(
                    "Attempted to write to illegal address ${address:04X} with value {value:02X}"
                )
            }
            0xFF00 => {
                if self.joypad.write(value) {
                    self.io[0x0F] |= 0x10
                }
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
            0xFF40..=0xFF45 | 0xFF47..=0xFF4B => {
                let interrupts = self.ppu.write(address, value);
                self.io[0x0F] |= interrupts;
            }
            0xFF46 => self.start_oam_dma(value),
            0xFF50 => {
                // Boot ROM mapping is disabled by a nonzero write
                if value != 0 {
                    info!("Boot ROM unmapped!");
                    self.boot_mapped = false;
                }
                self.io[0x50] = value;
            }
            0xFF01..=0xFF7F => self.io[usize::from(address - 0xFF00)] = value,
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
    use rookery_common::{Button, Platform, rom::Rom};
    use sm83::Bus;

    fn test_bus() -> GbBus {
        let mut bytes = vec![0; 0x8000];

        bytes[0x0100] = 0x3E;
        bytes[0x0101] = 0x42;

        let rom = Rom::from_bytes(bytes, Some(Platform::Gb)).unwrap();

        GbBus::new(Cartridge::new(rom).unwrap())
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

    #[test]
    fn upper_rom_bank_is_fixed() {
        let mut bytes = vec![0; 0x8000];

        bytes[0x3FFF] = 0x12;
        bytes[0x4000] = 0x34;
        bytes[0x7FFF] = 0x56;

        let rom = Rom::from_bytes(bytes, Some(Platform::Gb)).unwrap();

        let mut bus = GbBus::new(Cartridge::new(rom).unwrap());

        assert_eq!(bus.read8(0x3FFF), 0x12);
        assert_eq!(bus.read8(0x4000), 0x34);
        assert_eq!(bus.read8(0x7FFF), 0x56);

        bus.write8(0x2000, 0x05);
        bus.write8(0x4000, 0xAA);

        // ROM-only cartridges do not switch banks.
        assert_eq!(bus.read8(0x4000), 0x34);
        assert_eq!(bus.read8(0x7FFF), 0x56);
    }

    #[test]
    fn echo_ram_has_correct_upper_boundary() {
        let mut bus = test_bus();

        bus.write8(0xDDFF, 0x42);

        assert_eq!(bus.read8(0xFDFF), 0x42);

        // DE00 is not mirrored into FE00.
        bus.write8(0xDE00, 0x55);
        bus.write8(0xFE00, 0xAA);

        assert_eq!(bus.read8(0xDE00), 0x55);
        assert_eq!(bus.read8(0xFE00), 0xAA);
    }

    #[test]
    fn unusable_memory_ignores_writes() {
        let mut bus = test_bus();

        for address in 0xFEA0..=0xFEFF {
            bus.write8(address, 0x42);
            assert_eq!(bus.read8(address), 0xFF);
        }
    }

    #[test]
    fn vram_and_oam_are_independent() {
        let mut bus = test_bus();

        bus.write8(0x8000, 0x12);
        bus.write8(0x9FFF, 0x34);
        bus.write8(0xFE00, 0x56);
        bus.write8(0xFE9F, 0x78);

        assert_eq!(bus.read8(0x8000), 0x12);
        assert_eq!(bus.read8(0x9FFF), 0x34);
        assert_eq!(bus.read8(0xFE00), 0x56);
        assert_eq!(bus.read8(0xFE9F), 0x78);
    }

    #[test]
    fn oam_dma_copies_160_bytes_from_wram() {
        let mut bus = test_bus();

        for offset in 0..0xA0u16 {
            bus.write8(0xC100 + offset, (offset as u8).wrapping_mul(3));
        }

        // Transfer $C100–$C19F to OAM.
        bus.write8(0xFF46, 0xC1);

        for offset in 0..0xA0u16 {
            assert_eq!(bus.read8(0xFE00 + offset), (offset as u8).wrapping_mul(3),);
        }

        assert_eq!(bus.read8(0xFF46), 0xC1);
    }

    #[test]
    fn joypad_is_visible_through_memory_bus() {
        let mut bus = test_bus();

        // Post-boot P1 value.
        assert_eq!(bus.read8(0xFF00), 0xCF);

        // Select directions.
        bus.write8(0xFF00, 0x20);

        let mut buttons = Buttons::new();
        buttons.set(Button::Right, true);

        bus.set_buttons(buttons);

        assert_eq!(bus.read8(0xFF00), 0xEE);

        // Select action buttons instead.
        bus.write8(0xFF00, 0x10);

        assert_eq!(bus.read8(0xFF00), 0xDF);
    }

    #[test]
    fn joypad_interrupt_reaches_cpu_controller() {
        let mut bus = test_bus();

        bus.write8(0xFF00, 0x20);
        bus.write8(0xFF0F, 0);

        let mut buttons = Buttons::new();
        buttons.set(Button::Right, true);

        bus.set_buttons(buttons);

        // IF receives the request.
        assert_eq!(bus.read8(0xFF0F) & 0x10, 0x10);

        // IE is still zero, so nothing is pending
        // from the CPU's perspective.
        assert_eq!(bus.pending_interrupts(), 0);

        // Enable the joypad interrupt.
        bus.write8(0xFFFF, 0x10);

        assert_eq!(bus.pending_interrupts(), 0x10);

        // Simulate CPU acknowledgement.
        bus.acknowledge_interrupt(0x10);

        assert_eq!(bus.pending_interrupts(), 0);
        assert_eq!(bus.read8(0xFF0F) & 0x10, 0);

        // Still held: no new edge.
        bus.set_buttons(buttons);

        assert_eq!(bus.read8(0xFF0F) & 0x10, 0);

        // Release, then press again.
        buttons.set(Button::Right, false);
        bus.set_buttons(buttons);

        buttons.set(Button::Right, true);
        bus.set_buttons(buttons);

        assert_eq!(bus.read8(0xFF0F) & 0x10, 0x10);
    }

    #[test]
    fn selecting_a_held_button_requests_interrupt() {
        let mut bus = test_bus();

        // Deselect both groups.
        bus.write8(0xFF00, 0x30);

        let mut buttons = Buttons::new();
        buttons.set(Button::Start, true);
        bus.set_buttons(buttons);

        // The unselected button must not trigger IRQ.
        assert_eq!(bus.read8(0xFF0F) & 0x10, 0);

        // Selecting the action group should trigger it.
        bus.write8(0xFF00, 0x10);

        assert_eq!(bus.read8(0xFF00), 0xD7);
        assert_eq!(bus.read8(0xFF0F) & 0x10, 0x10);
    }
}
