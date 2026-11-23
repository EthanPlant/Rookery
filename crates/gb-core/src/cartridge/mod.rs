mod mbc1;

use rookery_common::rom::{Rom, RomHeader};
use thiserror::Error;

use crate::cartridge::mbc1::Mbc1;

const ROM_BANK_SIZE: usize = 0x4000;
const RAM_BANK_SIZE: usize = 0x2000;

#[derive(Debug, Error)]
pub enum CartridgeError {
    #[error("expected a Game Boy cartridge")]
    WrongPlatform,

    #[error("unsupported cartridge type: ${0:02X}")]
    UnsupportedType(u8),

    #[error("unknown cartridge RAM size code: {0:02X}")]
    UnknownRamSize(u8),

    #[error("unsupported cartridge ROM size: {0} bytes")]
    UnsupportedRomSize(usize),

    #[error("unsupported cartridge RAM size: {0} bytes")]
    UnsupportedRamSize(usize),

    #[error("cartridge does not have battery-backed RAM")]
    NoBattery,

    #[error("invalid save size: expected: {expected}, got {actual}")]
    InvalidSaveSize { expected: usize, actual: usize },
}

#[derive(Debug, Clone)]
pub struct Cartridge {
    rom: Rom,
    mapper: Mapper,
    ram: Vec<u8>,
    battery: bool,
    rom_bank_mask: usize,
    ram_dirty: bool,
}

impl Cartridge {
    pub fn new(rom: Rom) -> Result<Self, CartridgeError> {
        let RomHeader::Gb(header) = rom.header() else {
            return Err(CartridgeError::WrongPlatform);
        };

        let (mapper, has_ram, battery) = match header.cartridge_type {
            0x00 => (Mapper::RomOnly, false, false),
            0x08 => (Mapper::RomOnly, true, false),
            0x09 => (Mapper::RomOnly, true, true),

            0x01 => (Mapper::Mbc1(Mbc1::default()), false, false),
            0x02 => (Mapper::Mbc1(Mbc1::default()), true, false),
            0x03 => (Mapper::Mbc1(Mbc1::default()), true, true),

            kind => return Err(CartridgeError::UnsupportedType(kind)),
        };

        let ram_size = if has_ram {
            header
                .declared_ram_size()
                .ok_or(CartridgeError::UnknownRamSize(header.ram_size_code))?
        } else {
            0
        };

        match &mapper {
            Mapper::RomOnly if ram_size > RAM_BANK_SIZE => {
                return Err(CartridgeError::UnsupportedRamSize(ram_size));
            }

            Mapper::Mbc1(_) if ram_size > 4 * RAM_BANK_SIZE => {
                return Err(CartridgeError::UnsupportedRamSize(ram_size));
            }
            Mapper::Mbc1(_) if rom.len() > 128 * ROM_BANK_SIZE => {
                return Err(CartridgeError::UnsupportedRomSize(rom.len()));
            }

            _ => {}
        }

        let rom_bank_mask = bank_mask(rom.len(), ROM_BANK_SIZE);

        Ok(Self {
            rom,
            mapper,
            ram: vec![0xFF; ram_size],
            battery,
            rom_bank_mask,
            ram_dirty: false,
        })
    }

    pub fn rom(&self) -> &Rom {
        &self.rom
    }

    pub fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x7FFF => self.read_rom(address),
            0xA000..=0xBFFF => self.read_ram(address),

            _ => unreachable!("not a cartridge address"),
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x7FFF => {
                if let Mapper::Mbc1(mbc1) = &mut self.mapper {
                    mbc1.write_register(address, value);
                }
            }
            0xA000..=0xBFFF => self.write_ram(address, value),

            _ => unreachable!("not a cartridge address"),
        }
    }

    pub fn battery_ram(&self) -> Option<&[u8]> {
        self.battery.then_some(self.ram.as_slice())
    }

    pub fn load_battery_ram(&mut self, data: &[u8]) -> Result<(), CartridgeError> {
        if !self.battery {
            return Err(CartridgeError::NoBattery);
        }

        if data.len() != self.ram.len() {
            return Err(CartridgeError::InvalidSaveSize {
                expected: self.ram.len(),
                actual: data.len(),
            });
        }

        self.ram.copy_from_slice(data);
        self.ram_dirty = false;
        Ok(())
    }

    fn read_rom(&self, address: u16) -> u8 {
        let bank = match &self.mapper {
            Mapper::RomOnly => {
                if address < 0x4000 {
                    0
                } else {
                    1
                }
            }
            Mapper::Mbc1(mbc1) => {
                if address < 0x4000 {
                    mbc1.lower_rom_bank(self.rom_bank_mask)
                } else {
                    mbc1.upper_rom_bank(self.rom_bank_mask)
                }
            }
        };

        let offset = usize::from(address) & 0x3FFF;
        let index = bank * ROM_BANK_SIZE + offset;

        self.rom.bytes().get(index).copied().unwrap_or(0xFF)
    }

    fn read_ram(&self, address: u16) -> u8 {
        self.ram_index(address)
            .and_then(|index| self.ram.get(index))
            .copied()
            .unwrap_or(0xFF)
    }

    fn write_ram(&mut self, address: u16, value: u8) {
        if let Some(index) = self.ram_index(address)
            && self.ram[index] != value
        {
            self.ram[index] = value;

            if self.battery {
                self.ram_dirty = true;
            }
        }
    }

    pub fn battery_ram_dirty(&self) -> bool {
        self.battery && self.ram_dirty
    }

    pub fn acknowledge_battery_save(&mut self) {
        self.ram_dirty = false;
    }

    fn ram_index(&self, address: u16) -> Option<usize> {
        if self.ram.is_empty() {
            return None;
        }

        let bank = match &self.mapper {
            Mapper::RomOnly => 0,
            Mapper::Mbc1(mbc1) => {
                if !mbc1.ram_enabled() {
                    return None;
                }

                mbc1.ram_bank(bank_mask(self.ram.len(), RAM_BANK_SIZE))
            }
        };

        let window_size = self.ram.len().min(RAM_BANK_SIZE);
        let offset = usize::from(address - 0xA000) % window_size;
        let index = bank * RAM_BANK_SIZE + offset;

        (index < self.ram.len()).then_some(index)
    }
}

#[derive(Debug, Clone)]
enum Mapper {
    RomOnly,
    Mbc1(Mbc1),
}

fn bank_mask(size: usize, bank_size: usize) -> usize {
    if size == 0 {
        return 0;
    }

    size.div_ceil(bank_size).next_power_of_two() - 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use rookery_common::Platform;

    // Each ROM bank contains its own bank number as the
    // byte value. This makes incorrect mapping obvious.
    fn make_cartridge(cartridge_type: u8, rom_banks: usize, ram_size_code: u8) -> Cartridge {
        let mut bytes = vec![0; rom_banks * ROM_BANK_SIZE];

        for (bank, contents) in bytes
            .as_chunks_mut::<ROM_BANK_SIZE>()
            .0
            .iter_mut()
            .enumerate()
        {
            contents.fill(bank as u8);
        }

        bytes[0x147] = cartridge_type;
        bytes[0x149] = ram_size_code;

        let rom = Rom::from_bytes(bytes, Some(Platform::Gb)).unwrap();

        Cartridge::new(rom).unwrap()
    }

    #[test]
    fn rom_only_does_not_switch_banks() {
        let mut cart = make_cartridge(0x00, 2, 0);

        assert_eq!(cart.read(0x0000), 0);
        assert_eq!(cart.read(0x4000), 1);

        cart.write(0x2000, 0x05);
        cart.write(0x4000, 0x03);
        cart.write(0x6000, 0x01);

        assert_eq!(cart.read(0x0000), 0);
        assert_eq!(cart.read(0x4000), 1);
    }

    #[test]
    fn mbc1_starts_on_rom_bank_one() {
        let cart = make_cartridge(0x01, 8, 0);

        assert_eq!(cart.read(0x0000), 0);
        assert_eq!(cart.read(0x4000), 1);
    }

    #[test]
    fn mbc1_switches_rom_banks() {
        let mut cart = make_cartridge(0x01, 16, 0);

        for bank in 1..16u8 {
            cart.write(0x2000, bank);
            assert_eq!(cart.read(0x4000), bank);
        }
    }

    #[test]
    fn mbc1_zero_bank_maps_to_one() {
        let mut cart = make_cartridge(0x01, 32, 0);

        cart.write(0x2000, 0);
        assert_eq!(cart.read(0x4000), 1);

        cart.write(0x2000, 0x20);
        assert_eq!(cart.read(0x4000), 1);
    }

    #[test]
    fn mbc1_masks_rom_register_to_five_bits() {
        let mut cart = make_cartridge(0x01, 32, 0);

        cart.write(0x2000, 0xE5);

        assert_eq!(cart.read(0x4000), 5);
    }

    #[test]
    fn mbc1_uses_upper_rom_bank_bits() {
        let mut cart = make_cartridge(0x01, 128, 0);

        cart.write(0x2000, 5);

        cart.write(0x4000, 1);
        assert_eq!(cart.read(0x4000), 0x25);

        cart.write(0x4000, 2);
        assert_eq!(cart.read(0x4000), 0x45);

        cart.write(0x4000, 3);
        assert_eq!(cart.read(0x4000), 0x65);
    }

    #[test]
    fn mbc1_remaps_20_40_and_60() {
        let mut cart = make_cartridge(0x01, 128, 0);

        cart.write(0x2000, 0);

        for upper in 1..=3 {
            cart.write(0x4000, upper);

            let expected = (upper << 5) | 1;
            assert_eq!(cart.read(0x4000), expected);
        }
    }

    #[test]
    fn mbc1_advanced_mode_switches_lower_rom_window() {
        let mut cart = make_cartridge(0x01, 128, 0);

        cart.write(0x2000, 3);
        cart.write(0x4000, 2);

        // Simple mode: lower window always uses bank zero.
        assert_eq!(cart.read(0x0000), 0);
        assert_eq!(cart.read(0x4000), 0x43);

        cart.write(0x6000, 1);

        // Advanced mode: lower window uses high bank bits.
        assert_eq!(cart.read(0x0000), 0x40);

        // Upper window still uses both registers.
        assert_eq!(cart.read(0x4000), 0x43);

        cart.write(0x6000, 0);

        assert_eq!(cart.read(0x0000), 0);
        assert_eq!(cart.read(0x4000), 0x43);
    }

    #[test]
    fn mbc1_can_map_bank_zero_on_small_roms() {
        let mut cart = make_cartridge(0x01, 16, 0);

        // Full five-bit value is nonzero, so the
        // zero-to-one translation does not occur.
        // The fifth physical bank bit is disconnected.
        cart.write(0x2000, 0x10);

        assert_eq!(cart.read(0x4000), 0);

        cart.write(0x2000, 0);

        assert_eq!(cart.read(0x4000), 1);
    }

    #[test]
    fn mbc1_ram_must_be_enabled() {
        let mut cart = make_cartridge(0x02, 32, 0x02);

        assert_eq!(cart.read(0xA000), 0xFF);

        cart.write(0xA000, 0x42);
        assert_eq!(cart.read(0xA000), 0xFF);

        // Only the low nibble is examined.
        cart.write(0x0000, 0x1A);
        cart.write(0xA000, 0x42);

        assert_eq!(cart.read(0xA000), 0x42);

        cart.write(0x0000, 0x00);
        assert_eq!(cart.read(0xA000), 0xFF);

        cart.write(0x0000, 0x0A);
        assert_eq!(cart.read(0xA000), 0x42);
    }

    #[test]
    fn mbc1_switches_all_four_ram_banks() {
        let mut cart = make_cartridge(0x03, 32, 0x03);

        cart.write(0x0000, 0x0A);
        cart.write(0x6000, 1);

        for bank in 0..4u8 {
            cart.write(0x4000, bank);
            cart.write(0xA000, 0x10 + bank);
        }

        for bank in 0..4u8 {
            cart.write(0x4000, bank);

            assert_eq!(cart.read(0xA000), 0x10 + bank);
        }

        // Simple mode always maps RAM bank zero.
        cart.write(0x6000, 0);

        for bank in 0..4u8 {
            cart.write(0x4000, bank);
            assert_eq!(cart.read(0xA000), 0x10);
        }
    }

    #[test]
    fn battery_ram_can_be_exported_and_restored() {
        let mut cart = make_cartridge(0x03, 32, 0x02);

        cart.write(0x0000, 0x0A);
        cart.write(0xA000, 0x42);
        cart.write(0xA001, 0x99);

        let save = cart.battery_ram().unwrap().to_vec();

        assert_eq!(save.len(), 0x2000);
        assert_eq!(&save[..2], &[0x42, 0x99]);

        let mut restored = make_cartridge(0x03, 32, 0x02);

        restored.load_battery_ram(&save).unwrap();
        restored.write(0x0000, 0x0A);

        assert_eq!(restored.read(0xA000), 0x42);
        assert_eq!(restored.read(0xA001), 0x99);
    }

    #[test]
    fn incorrect_save_size_is_rejected() {
        let mut cart = make_cartridge(0x03, 32, 0x02);

        assert!(matches!(
            cart.load_battery_ram(&[0; 16]),
            Err(CartridgeError::InvalidSaveSize { .. })
        ));
    }

    #[test]
    fn unsupported_cartridge_type_is_rejected() {
        let mut bytes = vec![0; 0x8000];
        bytes[0x147] = 0x11; // MBC3

        let rom = Rom::from_bytes(bytes, Some(Platform::Gb)).unwrap();

        assert!(matches!(
            Cartridge::new(rom),
            Err(CartridgeError::UnsupportedType(0x11))
        ));
    }

    #[test]
    fn battery_ram_is_only_dirty_after_a_change() {
        let mut cart = make_cartridge(0x03, 32, 0x02);

        assert!(!cart.battery_ram_dirty());

        cart.write(0x0000, 0x0A);

        assert_eq!(cart.read(0xA000), 0xFF);
        assert!(!cart.battery_ram_dirty());

        cart.write(0xA000, 0xFF);
        assert!(!cart.battery_ram_dirty());

        cart.write(0xA000, 0x42);
        assert!(cart.battery_ram_dirty());

        cart.acknowledge_battery_save();
        assert!(!cart.battery_ram_dirty());

        cart.write(0xA000, 0x42);
        assert!(!cart.battery_ram_dirty());

        cart.write(0xA000, 0x99);
        assert!(cart.battery_ram_dirty());
    }
}
