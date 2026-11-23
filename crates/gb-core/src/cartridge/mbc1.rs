#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum BankingMode {
    #[default]
    Simple,
    Advanced,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Mbc1 {
    ram_enabled: bool,
    rom_bank_low: u8,
    bank_high: u8,
    mode: BankingMode,
}

impl Mbc1 {
    pub fn write_register(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1FFF => self.ram_enabled = value & 0xF == 0x0A,
            0x2000..=0x3FFF => self.rom_bank_low = value & 0x1F,
            0x4000..=0x5FFF => self.bank_high = value & 0x03,
            0x6000..=0x7FFF => {
                self.mode = if value & 1 == 0 {
                    BankingMode::Simple
                } else {
                    BankingMode::Advanced
                }
            }
            _ => unreachable!("not a valid cartridge address"),
        }
    }

    pub fn lower_rom_bank(&self, mask: usize) -> usize {
        match self.mode {
            BankingMode::Simple => 0,
            BankingMode::Advanced => (usize::from(self.bank_high) << 5) & mask,
        }
    }

    pub fn upper_rom_bank(&self, mask: usize) -> usize {
        let low = if self.rom_bank_low == 0 {
            1
        } else {
            self.rom_bank_low
        };

        let bank = (usize::from(self.bank_high) << 5) | usize::from(low);

        bank & mask
    }

    pub fn ram_bank(&self, mask: usize) -> usize {
        match self.mode {
            BankingMode::Simple => 0,
            BankingMode::Advanced => usize::from(self.bank_high) & mask,
        }
    }

    pub fn ram_enabled(&self) -> bool {
        self.ram_enabled
    }
}
