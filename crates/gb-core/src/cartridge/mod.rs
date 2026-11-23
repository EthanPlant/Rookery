use rookery_common::rom::{Rom, RomHeader};
use thiserror::Error;

const ROM_BANK_SIZE: usize = 0x4000;
const RAM_BANK_SIZE: usize = 0x2000;

#[derive(Debug, Error)]
pub enum CartridgError {
    #[error("expected a Game Boy cartridge")]
    WrongPlatform,

    #[error("unsupported cartridge type: ${0:02X}")]
    UnsupportedType(u8),

    #[error("unknown cartridge RAM size code: {0:02X}")]
    UnknownRamSize(u8),

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
}

impl Cartridge {
    pub fn new(rom: Rom) -> Result<Self, CartridgError> {
        let RomHeader::Gb(header) = rom.header() else {
            return Err(CartridgError::WrongPlatform);
        };

        let (mapper, has_ram, battery) = match header.cartridge_type {
            0x00 => (Mapper::RomOnly, false, true),
            0x08 => (Mapper::RomOnly, true, false),
            0x09 => (Mapper::RomOnly, true, true),

            kind => return Err(CartridgError::UnsupportedType(kind)),
        };

        let ram_size = if has_ram {
            header
                .declared_ram_size()
                .ok_or(CartridgError::UnknownRamSize(header.ram_size_code))?
        } else {
            0
        };

        match &mapper {
            Mapper::RomOnly if ram_size > RAM_BANK_SIZE => {
                return Err(CartridgError::UnsupportedRamSize(ram_size));
            }

            _ => {}
        }

        Ok(Self {
            rom,
            mapper,
            ram: vec![0xFF; ram_size],
            battery,
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
            0x0000..=0x7FFF => {}
            0xA000..=0xBFFF => self.write_ram(address, value),

            _ => unreachable!("not a cartridge address"),
        }
    }

    pub fn battery_ram(&self) -> Option<&[u8]> {
        self.battery.then_some(self.ram.as_slice())
    }

    pub fn load_battery_ram(&mut self, data: &[u8]) -> Result<(), CartridgError> {
        if !self.battery {
            return Err(CartridgError::NoBattery);
        }

        if data.len() != self.ram.len() {
            return Err(CartridgError::InvalidSaveSize {
                expected: self.ram.len(),
                actual: data.len(),
            });
        }

        self.ram.copy_from_slice(data);
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
        if let Some(index) = self.ram_index(address) {
            self.ram[index] = value;
        }
    }

    fn ram_index(&self, address: u16) -> Option<usize> {
        if self.ram.is_empty() {
            return None;
        }

        let bank = match &self.mapper {
            Mapper::RomOnly => 0,
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
}
