//! SM83 architectural register storage and flag helpers.

use crate::{Register8, Register16};

/// Zero flag (bit 7 of F).
pub const FLAG_Z: u8 = 0x80;
/// Subtract flag (bit 6 of F).
pub const FLAG_N: u8 = 0x40;
/// Half-carry flag (bit 5 of F).
pub const FLAG_H: u8 = 0x20;
/// Carry flag (bit 4 of F).
pub const FLAG_C: u8 = 0x10;

pub(crate) const FLAG_MASK: u8 = 0xF0;

/// The CPU visible registers. `F` is meaningful only in its upper nibble.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Registers {
    /// Accumulator.
    pub a: u8,
    /// Flags.
    pub f: u8,
    /// General-purpose register B.
    pub b: u8,
    /// General-purpose register C.
    pub c: u8,
    /// General-purpose register D.
    pub d: u8,
    /// General-purpose regiser E.
    pub e: u8,
    /// General-purpose register H.
    pub h: u8,
    /// General-purpose register L.
    pub l: u8,
    /// Stack pointer.
    pub sp: u16,
    /// Program counter.
    pub pc: u16,
}

impl Registers {
    /// Read one 8-bit register.
    pub fn get8(&self, register: Register8) -> u8 {
        match register {
            Register8::A => self.a,
            Register8::B => self.b,
            Register8::C => self.c,
            Register8::D => self.d,
            Register8::E => self.e,
            Register8::H => self.h,
            Register8::L => self.l,
        }
    }

    /// Write one 8-bit register.
    pub fn set8(&mut self, register: Register8, value: u8) {
        match register {
            Register8::A => self.a = value,
            Register8::B => self.b = value,
            Register8::C => self.c = value,
            Register8::D => self.d = value,
            Register8::E => self.e = value,
            Register8::H => self.h = value,
            Register8::L => self.l = value,
        }
    }

    /// Read a 16-bit register.
    pub fn get16(&self, register: Register16) -> u16 {
        match register {
            Register16::AF => self.af(),
            Register16::BC => self.bc(),
            Register16::DE => self.de(),
            Register16::HL => self.hl(),
            Register16::SP => self.sp,
        }
    }

    /// Write a 16-bit register.
    pub fn set16(&mut self, register: Register16, value: u16) {
        match register {
            Register16::AF => self.set_af(value),
            Register16::BC => self.set_bc(value),
            Register16::DE => self.set_de(value),
            Register16::HL => self.set_hl(value),
            Register16::SP => self.sp = value,
        }
    }

    /// Return AF, always masking the unused flag bits.
    pub fn af(&self) -> u16 {
        u16::from_be_bytes([self.a, self.f & FLAG_MASK])
    }

    /// Return BC.
    pub fn bc(&self) -> u16 {
        u16::from_be_bytes([self.b, self.c])
    }

    /// Return DE.
    pub fn de(&self) -> u16 {
        u16::from_be_bytes([self.d, self.e])
    }

    /// Return HL.
    pub fn hl(&self) -> u16 {
        u16::from_be_bytes([self.h, self.l])
    }

    /// Write AF, masking the unused flag bits.
    pub fn set_af(&mut self, value: u16) {
        let [a, f] = value.to_be_bytes();
        self.a = a;
        self.f = f & FLAG_MASK;
    }

    /// Write BC.
    pub fn set_bc(&mut self, value: u16) {
        let [b, c] = value.to_be_bytes();
        self.b = b;
        self.c = c;
    }

    /// Write DE.
    pub fn set_de(&mut self, value: u16) {
        let [d, e] = value.to_be_bytes();
        self.d = d;
        self.e = e;
    }

    /// Write HL.
    pub fn set_hl(&mut self, value: u16) {
        let [h, l] = value.to_be_bytes();
        self.h = h;
        self.l = l;
    }
}
