//! Sharp SM83 CPU interpreter.
//!
//! Implements the CPU used by the Nintendo Game Boy (DMG)
//! and Game Boy Color (CGB).
//!
//! The CPU operates independently of the surrounding hardware
//! through the [`Bus`] trait. Each instruction is executed
//! atomically, returning its total number of CPU T-cycles.
//!
//! This implementation is currently incomplete. Unsupported
//! instructions return [`CpuError::Unimplemented`] rather than
//! silently executing incorrect behavior.
#![warn(missing_docs)]

use thiserror::Error;

use crate::Instruction::DecrementHL;

/// Zero Flag (Z)
///
/// Set when an operation produces a zero result, or when required by the
/// instruction's specific flag behaviour.
pub const FLAG_Z: u8 = 0x80;

/// Subtraction Flag (N)
///
/// Set when the preceding arithmetic operation was a subtraction,
/// including comparison and decrement operations.
pub const FLAG_N: u8 = 0x40;

/// Half-Carry Flag (H)
///
/// Indicates a carry or borrow between bits 3 and 4 for 8-bit
/// arithmetic, or bits 11 and 12 for 16-bit operations.
pub const FLAG_H: u8 = 0x20;

/// Carry flag (C)
///
/// Indicates a carry or borrow from the most significant bit,
/// or stores the bit shifted out by certain instructions.
pub const FLAG_C: u8 = 0x10;

const FLAG_MASK: u8 = 0xF0;

/// Provides memory access for the SM83 CPU.
///
/// The CPU uses this interface for instruction fetching,
/// operand rads, and memory writes.
///
/// Implementations are responsible for mapping addresses
/// to cartridge ROM, RAM, hardware registers, and other
/// memory mapped devices.
pub trait Bus {
    /// Reads a byte from the specified address.
    ///
    /// Reads require mutable access because reads and writes
    /// may have hardware side effects.
    fn read8(&mut self, address: u16) -> u8;

    /// Writes a byte to the specified address.
    ///
    /// The bus implementation determines whether the address is writable
    /// and what side effects occur.
    fn write8(&mut self, address: u16, value: u8);
}

/// Error encountered while executing an SM83 instruction.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CpuError {
    /// The opcode is valid but not yet implemented
    #[error("unimplemented SM83 opcode {opcode:X?} at ${pc:04X}")]
    Unimplemented {
        /// Address of the unsupported instrruction.
        pc: u16,
        /// Unsupported opcode.
        opcode: Opcode,
    },

    /// The opcode is not defined by the SM83 instruction set.
    #[error("illegal SM83 opcode {opcode:02X} at ${pc:04X}")]
    Illegal {
        /// Address of the illegal opcode
        pc: u16,
        /// Unsupported opcode
        opcode: u8,
    },
}

/// Interpreter for the Sharp SM83 CPU.
///
/// Owns the CPU's registers and execution-control state,
/// but does not own system memory or peripherals.
///
/// Instructions execute against an external [`Bus`]
/// provided to [`Sm83::step`] or
/// [`Sm83::step_with_trace`].
#[derive(Debug, Default)]
pub struct Sm83 {
    registers: Registers,

    ime: bool,
    halted: bool,
    stopped: bool,

    ime_enable_delay: u8,
    halt_bug: bool,
}

impl Sm83 {
    /// Creates a CPU with zero-initialized registers
    /// and execution-control state.
    ///
    /// This does not represent the hardware's post-boot
    /// register values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a CPU with the provided register state.
    ///
    /// The lower four bits of F are cleared to enforce the SM83
    /// flag-register layout.
    ///
    /// All execution-control state is initialized to its default
    /// values.
    ///
    /// # Examples
    ///
    /// ```
    /// use sm83::{Registers, Sm83};
    ///
    /// let mut registers = Registers {
    ///     a: 0x01,
    ///     f: 0x00,
    ///     b: 0xFF,
    ///     c: 0x13,
    ///     d: 0x00,
    ///     e: 0xC1,
    ///     h: 0x84,
    ///     l: 0x03,
    ///     pc: 0x100,
    ///     sp: 0xFFFE,
    /// };
    ///
    /// let cpu = Sm83::with_registers(registers);
    ///
    /// assert_eq!(cpu.registers().a, 0x01);
    /// ```
    pub fn with_registers(mut registers: Registers) -> Self {
        registers.f &= FLAG_MASK;

        Self {
            registers,
            ..Self::default()
        }
    }

    /// Fetches, decodes, and executes on SM83 instruction,
    /// returning the number of T-cycles consumed.
    ///
    /// # Errors
    ///
    /// Returns [`CpuError`] if the instruction is illegal or unimplemented.
    ///
    /// On error, PC is restored to the instruction's
    /// starting address. Memory-read side effects during
    /// decoding are not rolled back.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> Result<u32, CpuError> {
        Ok(self.step_with_trace(bus)?.cycles)
    }

    /// Executes one instruction and returns execution metadata.
    ///
    /// Equivalent to [`Sm83::step`], but additionally returns the starting
    /// program counter, opcode, and decoded instruction for debugging or
    /// tracing.
    ///
    /// # Errors
    ///
    /// Returns [`CpuError`] if the instruction is illegal or unimplemented.
    ///
    /// On error, PC is restored to the instruction's
    /// starting address. Memory-read side effects during
    /// decoding are not rolled back.
    pub fn step_with_trace<B: Bus>(&mut self, bus: &mut B) -> Result<StepResult, CpuError> {
        let start_pc = self.registers.pc;

        let decoded = match self.decode(bus) {
            Ok(decoded) => decoded,
            Err(e) => {
                self.registers.pc = start_pc;
                return Err(e);
            }
        };

        let cycles = self.execute(bus, decoded.instruction);

        Ok(StepResult {
            pc: decoded.pc,
            opcode: decoded.opcode,
            instruction: decoded.instruction,
            cycles,
        })
    }

    /// Returns an immutable reference to the CPU registers.
    pub fn registers(&self) -> &Registers {
        &self.registers
    }

    /// Returns a snapshot of the CPU's execution-control state.
    pub fn status(&self) -> CpuStatus {
        CpuStatus {
            ime: self.ime,
            halted: self.halted,
            stopped: self.stopped,
            ime_enable_delay: self.ime_enable_delay,
        }
    }

    fn decode<B: Bus>(&mut self, bus: &mut B) -> Result<DecodedInstruction, CpuError> {
        let pc = self.registers.pc;
        let first_byte = self.fetch_ocpode(bus);

        let opcode = if first_byte == 0xCB {
            Opcode::Cb(self.fetch8(bus))
        } else {
            Opcode::Base(first_byte)
        };

        let instruction = match opcode {
            Opcode::Base(0x00) => Instruction::Nop,

            Opcode::Base(0xC3) => Instruction::Jump {
                addr: self.fetch16(bus),
            },

            Opcode::Base(0xE9) => Instruction::JumpHl,

            Opcode::Base(0x18) => Instruction::JumpRelative {
                offset: self.fetch8(bus) as i8,
            },

            Opcode::Base(code @ (0x20 | 0x28 | 0x30)) => {
                let condition = match code {
                    0x20 => Condition::NotZero,
                    0x28 => Condition::Zero,
                    0x30 => Condition::NotCarry,
                    _ => unreachable!(),
                };

                let offset = self.fetch8(bus) as i8;

                Instruction::JumpRelativeConditional { condition, offset }
            }

            Opcode::Base(code @ (0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x3E)) => {
                let register = match code {
                    0x06 => Register8::B,
                    0x0E => Register8::C,
                    0x16 => Register8::D,
                    0x1E => Register8::E,
                    0x26 => Register8::H,
                    0x2E => Register8::L,
                    0x3E => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::LoadImmediate8 {
                    register,
                    value: self.fetch8(bus),
                }
            }

            Opcode::Base(code @ 0x29) => {
                let register = match code {
                    0x29 => Register16::HL,
                    _ => unreachable!(),
                };

                Instruction::AddHlRegister { register }
            }

            Opcode::Base(code @ (0x40 | 0x41 | 0x42 | 0x43 | 0x44 | 0x45 | 0x47)) => {
                let source = match code {
                    0x40 => Register8::B,
                    0x41 => Register8::C,
                    0x42 => Register8::D,
                    0x43 => Register8::E,
                    0x44 => Register8::H,
                    0x45 => Register8::L,
                    0x47 => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::LoadSourceDest {
                    source,
                    dest: Register8::B,
                }
            }

            Opcode::Base(code @ (0x48 | 0x49 | 0x4A | 0x4B | 0x4C | 0x4D | 0x4F)) => {
                let source = match code {
                    0x48 => Register8::B,
                    0x49 => Register8::C,
                    0x4A => Register8::D,
                    0x4B => Register8::E,
                    0x4C => Register8::H,
                    0x4D => Register8::L,
                    0x4F => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::LoadSourceDest {
                    source,
                    dest: Register8::C,
                }
            }

            Opcode::Base(code @ (0x50 | 0x51 | 0x52 | 0x53 | 0x54 | 0x55 | 0x57)) => {
                let source = match code {
                    0x50 => Register8::B,
                    0x51 => Register8::C,
                    0x52 => Register8::D,
                    0x53 => Register8::E,
                    0x54 => Register8::H,
                    0x55 => Register8::L,
                    0x57 => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::LoadSourceDest {
                    source,
                    dest: Register8::D,
                }
            }

            Opcode::Base(code @ (0x58 | 0x59 | 0x5A | 0x5B | 0x5C | 0x5D | 0x5F)) => {
                let source = match code {
                    0x58 => Register8::B,
                    0x59 => Register8::C,
                    0x5A => Register8::D,
                    0x5B => Register8::E,
                    0x5C => Register8::H,
                    0x5D => Register8::L,
                    0x5F => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::LoadSourceDest {
                    source,
                    dest: Register8::E,
                }
            }

            Opcode::Base(code @ (0x60 | 0x61 | 0x62 | 0x63 | 0x64 | 0x65 | 0x67)) => {
                let source = match code {
                    0x60 => Register8::B,
                    0x61 => Register8::C,
                    0x62 => Register8::D,
                    0x63 => Register8::E,
                    0x64 => Register8::H,
                    0x65 => Register8::L,
                    0x67 => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::LoadSourceDest {
                    source,
                    dest: Register8::H,
                }
            }

            Opcode::Base(code @ (0x68 | 0x69 | 0x6A | 0x6B | 0x6C | 0x6D | 0x6F)) => {
                let source = match code {
                    0x68 => Register8::B,
                    0x69 => Register8::C,
                    0x6A => Register8::D,
                    0x6B => Register8::E,
                    0x6C => Register8::H,
                    0x6D => Register8::L,
                    0x6F => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::LoadSourceDest {
                    source,
                    dest: Register8::L,
                }
            }

            Opcode::Base(code @ (0x78 | 0x79 | 0x7A | 0x7B | 0x7C | 0x7D | 0x7F)) => {
                let source = match code {
                    0x78 => Register8::B,
                    0x79 => Register8::C,
                    0x7A => Register8::D,
                    0x7B => Register8::E,
                    0x7C => Register8::H,
                    0x7D => Register8::L,
                    0x7F => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::LoadSourceDest {
                    source,
                    dest: Register8::A,
                }
            }

            Opcode::Base(code @ (0x01 | 0x11 | 0x21 | 0x31)) => {
                let register = match code {
                    0x01 => Register16::BC,
                    0x11 => Register16::DE,
                    0x21 => Register16::HL,
                    0x31 => Register16::SP,
                    _ => unreachable!(),
                };

                Instruction::LoadImmediate16 {
                    register,
                    value: self.fetch16(bus),
                }
            }

            Opcode::Base(code @ 0x1A) => {
                let register = match code {
                    0x1A => Register16::DE,
                    _ => unreachable!(),
                };

                Instruction::LoadRegPair { register }
            }

            Opcode::Base(code @ 0x12) => {
                let register = match code {
                    0x12 => Register16::DE,
                    _ => unreachable!(),
                };

                Instruction::StoreRegPair { register }
            }

            Opcode::Base(0x22) => Instruction::LoadIncHl,
            Opcode::Base(0x32) => Instruction::LoadDecHl,
            Opcode::Base(0x2A) => Instruction::LoadAIncHl,

            Opcode::Base(code @ (0xA9 | 0xAD | 0xAF)) => {
                let register = match code {
                    0xA9 => Register8::C,
                    0xAD => Register8::L,
                    0xAF => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::Xor { register }
            }
            Opcode::Base(0xEE) => Instruction::XorImmediate {
                value: self.fetch8(bus),
            },
            Opcode::Base(0xAE) => Instruction::XorHl,

            Opcode::Base(code @ (0xB0 | 0xB1 | 0xB7)) => {
                let register = match code {
                    0xB0 => Register8::B,
                    0xB1 => Register8::C,
                    0xB7 => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::Or { register }
            }
            Opcode::Base(0xB6) => Instruction::OrHl,

            Opcode::Base(0xE6) => Instruction::AndImmediate {
                value: self.fetch8(bus),
            },

            Opcode::Base(0xE0) => Instruction::LoadHigh {
                offset: self.fetch8(bus),
            },
            Opcode::Base(0xE2) => Instruction::LoadHighRamC,

            Opcode::Base(0xF0) => Instruction::ReadHigh {
                offset: self.fetch8(bus),
            },

            Opcode::Base(code @ (0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x3C)) => {
                let register = match code {
                    0x04 => Register8::B,
                    0x0C => Register8::C,
                    0x14 => Register8::D,
                    0x1C => Register8::E,
                    0x24 => Register8::H,
                    0x2C => Register8::L,
                    0x3C => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::Increment { register }
            }

            Opcode::Base(code @ (0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x3D)) => {
                let register = match code {
                    0x05 => Register8::B,
                    0x0D => Register8::C,
                    0x15 => Register8::D,
                    0x1D => Register8::E,
                    0x25 => Register8::H,
                    0x2D => Register8::L,
                    0x3D => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::Decrement { register }
            }

            Opcode::Base(code @ (0x03 | 0x13 | 0x23)) => {
                let register = match code {
                    0x03 => Register16::BC,
                    0x13 => Register16::DE,
                    0x23 => Register16::HL,
                    _ => unreachable!(),
                };

                Instruction::Increment16 { register }
            }

            Opcode::Base(code @ 0x1B) => {
                let register = match code {
                    0x1B => Register16::DE,
                    _ => unreachable!(),
                };

                Instruction::Decrement16 { register }
            }

            Opcode::Base(0x35) => DecrementHL,

            Opcode::Base(code @ (0x70 | 0x71 | 0x72 | 0x73 | 0x74 | 0x75 | 0x77)) => {
                let register = match code {
                    0x70 => Register8::B,
                    0x71 => Register8::C,
                    0x72 => Register8::D,
                    0x73 => Register8::E,
                    0x74 => Register8::H,
                    0x75 => Register8::L,
                    0x77 => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::StoreHl { register }
            }

            Opcode::Base(code @ (0x46 | 0x4E | 0x56 | 0x5E | 0x66 | 0x6E | 0x7E)) => {
                let register = match code {
                    0x46 => Register8::B,
                    0x4E => Register8::C,
                    0x56 => Register8::D,
                    0x5E => Register8::E,
                    0x66 => Register8::H,
                    0x6E => Register8::L,
                    0x7E => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::LoadHl { register }
            }

            Opcode::Base(0xCD) => Instruction::Call {
                addr: self.fetch16(bus),
            },

            Opcode::Base(code @ 0xC4) => {
                let condition = match code {
                    0xC4 => Condition::NotZero,
                    _ => unreachable!(),
                };

                Instruction::CallConditional {
                    condition,
                    addr: self.fetch16(bus),
                }
            }

            Opcode::Base(0xC9) => Instruction::Return,
            Opcode::Base(code @ (0xC8 | 0xD0 | 0xD8)) => {
                let condition = match code {
                    0xC8 => Condition::Zero,
                    0xD0 => Condition::NotCarry,
                    0xD8 => Condition::Carry,
                    _ => unreachable!(),
                };

                Instruction::ReturnConditional { condition }
            }

            Opcode::Base(code @ (0xC5 | 0xD5 | 0xE5 | 0xF5)) => {
                let register = match code {
                    0xC5 => Register16::BC,
                    0xD5 => Register16::DE,
                    0xE5 => Register16::HL,
                    0xF5 => Register16::AF,
                    _ => unreachable!(),
                };

                Instruction::Push { register }
            }

            Opcode::Base(code @ (0xC1 | 0xD1 | 0xE1 | 0xF1)) => {
                let register = match code {
                    0xE1 => Register16::HL,
                    0xD1 => Register16::DE,
                    0xC1 => Register16::BC,
                    0xF1 => Register16::AF,
                    _ => unreachable!(),
                };

                Instruction::Pop { register }
            }

            Opcode::Base(0x17) => Instruction::RotateALeft,
            Opcode::Base(0x1F) => Instruction::RotateARight,

            Opcode::Base(0xFE) => Instruction::Compare {
                value: self.fetch8(bus),
            },
            Opcode::Base(0xBE) => Instruction::CompareHl,

            Opcode::Base(0xEA) => Instruction::StoreA {
                addr: self.fetch16(bus),
            },

            Opcode::Base(0xFA) => Instruction::LoadA {
                addr: self.fetch16(bus),
            },

            Opcode::Base(code @ 0x90) => {
                let register = match code {
                    0x90 => Register8::B,
                    _ => unreachable!(),
                };

                Instruction::Subtract { register }
            }
            Opcode::Base(0xD6) => Instruction::SubImmediate {
                value: self.fetch8(bus),
            },

            Opcode::Base(0xC6) => Instruction::AddImmediate {
                value: self.fetch8(bus),
            },
            Opcode::Base(0x86) => Instruction::AddHl,

            Opcode::Base(0xCE) => Instruction::AddCarryImmediate {
                value: self.fetch8(bus),
            },

            Opcode::Base(0xF3) => Instruction::DisableInterrupts,

            Opcode::Cb(code @ 0x38) => {
                let register = match code {
                    0x38 => Register8::B,
                    _ => unreachable!(),
                };

                Instruction::ShiftRight { register }
            }

            Opcode::Cb(code @ 0x11) => {
                let register = match code {
                    0x11 => Register8::C,
                    _ => unreachable!(),
                };

                Instruction::RotateLeft { register }
            }

            Opcode::Cb(code @ (0x19 | 0x1A | 0x1B)) => {
                let register = match code {
                    0x19 => Register8::C,
                    0x1A => Register8::D,
                    0x1B => Register8::E,
                    _ => unreachable!(),
                };

                Instruction::RotateRight { register }
            }

            Opcode::Cb(code @ 0x40..=0x7F) if code & 0x07 != 6 => {
                let bit = (code >> 3) & 0x07;

                let register = match code & 0x07 {
                    0 => Register8::B,
                    1 => Register8::C,
                    2 => Register8::D,
                    3 => Register8::E,
                    4 => Register8::H,
                    5 => Register8::L,
                    7 => Register8::A,
                    _ => unreachable!(),
                };

                Instruction::TestBit { bit, register }
            }

            Opcode::Base(
                code @ (0xD3 | 0xDB | 0xDD | 0xE3 | 0xE4 | 0xEB | 0xEC | 0xED | 0xF4 | 0xFC | 0xFD),
            ) => return Err(CpuError::Illegal { pc, opcode: code }),

            _ => return Err(CpuError::Unimplemented { pc, opcode }),
        };

        Ok(DecodedInstruction {
            pc,
            opcode,
            instruction,
        })
    }

    fn execute<B: Bus>(&mut self, bus: &mut B, instruction: Instruction) -> u32 {
        match instruction {
            Instruction::Nop => 4,

            Instruction::LoadImmediate8 { register, value } => {
                self.registers.set8(register, value);
                8
            }

            Instruction::LoadSourceDest { source, dest } => {
                self.registers.set8(dest, self.registers.get8(source));
                4
            }

            Instruction::LoadImmediate16 { register, value } => {
                self.registers.set16(register, value);
                12
            }

            Instruction::Jump { addr } => {
                self.registers.pc = addr;
                16
            }

            Instruction::JumpHl => {
                self.registers.pc = self.registers.hl();
                4
            }

            Instruction::JumpRelative { offset } => {
                self.registers.pc = self.registers.pc.wrapping_add_signed(i16::from(offset));
                12
            }

            Instruction::JumpRelativeConditional { condition, offset } => {
                if condition.evaluate(self.registers.f) {
                    self.registers.pc = self.registers.pc.wrapping_add_signed(i16::from(offset));
                    12
                } else {
                    8
                }
            }

            Instruction::AddHlRegister { register } => {
                let result = self
                    .registers
                    .hl()
                    .wrapping_add(self.registers.get16(register));
                self.registers.f = 0;
                if (self.registers.hl() & 0xFFF) + (self.registers.get16(register) & 0x0FFF)
                    > 0x0FFF
                {
                    self.registers.f |= FLAG_H;
                }
                if (self.registers.hl() as u32) + (self.registers.get16(register) as u32) > 0xFFFF {
                    self.registers.f |= FLAG_C;
                }
                self.registers.set_hl(result);
                8
            }

            Instruction::LoadIncHl => {
                self.write8(bus, self.registers.hl(), self.registers.a);
                self.registers.set_hl(self.registers.hl().wrapping_add(1));
                8
            }

            Instruction::LoadDecHl => {
                self.write8(bus, self.registers.hl(), self.registers.a);
                self.registers.set_hl(self.registers.hl().wrapping_sub(1));
                8
            }

            Instruction::LoadAIncHl => {
                self.registers.a = self.read8(bus, self.registers.hl());
                self.registers.set_hl(self.registers.hl().wrapping_add(1));
                8
            }

            Instruction::LoadRegPair { register } => {
                self.registers.a = self.read8(bus, self.registers.get16(register));
                8
            }

            Instruction::StoreRegPair { register } => {
                self.write8(bus, self.registers.get16(register), self.registers.a);
                8
            }

            Instruction::Or { register } => {
                let value = self.registers.get8(register);
                self.registers.a |= value;
                self.registers.f = if self.registers.a == 0 { FLAG_Z } else { 0 };
                4
            }

            Instruction::OrHl => {
                self.registers.a |= self.read8(bus, self.registers.hl());
                self.registers.f = if self.registers.a == 0 { FLAG_Z } else { 0 };
                8
            }

            Instruction::Xor { register } => {
                let value = self.registers.get8(register);
                self.registers.a ^= value;
                self.registers.f = if self.registers.a == 0 { FLAG_Z } else { 0 };
                4
            }

            Instruction::XorImmediate { value } => {
                self.registers.a ^= value;
                self.registers.f = if self.registers.a == 0 { FLAG_Z } else { 0 };
                8
            }

            Instruction::XorHl => {
                self.registers.a ^= self.read8(bus, self.registers.hl());
                self.registers.f = if self.registers.a == 0 { FLAG_Z } else { 0 };
                8
            }

            Instruction::And { register } => {
                self.registers.a &= self.registers.get8(register);
                self.registers.f = if self.registers.a == 0 {
                    FLAG_Z | FLAG_H
                } else {
                    FLAG_H
                };
                4
            }

            Instruction::AndImmediate { value } => {
                self.registers.a &= value;
                self.registers.f = if self.registers.a == 0 {
                    FLAG_Z | FLAG_H
                } else {
                    FLAG_H
                };
                4
            }

            Instruction::LoadHighRamC => {
                let addr = 0xFF00 + u16::from(self.registers.c);
                self.write8(bus, addr, self.registers.a);
                8
            }

            Instruction::LoadHigh { offset } => {
                self.write8(bus, 0xFF00 + u16::from(offset), self.registers.a);
                12
            }

            Instruction::ReadHigh { offset } => {
                self.registers.a = self.read8(bus, 0xFF00 + u16::from(offset));
                12
            }

            Instruction::Increment { register } => {
                let result = self.registers.get8(register).wrapping_add(1);
                self.registers.f &= FLAG_C;
                if result == 0 {
                    self.registers.f |= FLAG_Z;
                }
                if self.registers.get8(register) & 0x0F == 0x0F {
                    self.registers.f |= FLAG_H;
                }
                self.registers.set8(register, result);
                4
            }

            Instruction::Decrement { register } => {
                let result = self.registers.get8(register).wrapping_sub(1);
                self.registers.f &= FLAG_C;
                self.registers.f |= FLAG_N;
                if result == 0 {
                    self.registers.f |= FLAG_Z;
                }
                if self.registers.get8(register) & 0x0F == 0x00 {
                    self.registers.f |= FLAG_H;
                }
                self.registers.set8(register, result);
                4
            }

            Instruction::Increment16 { register } => {
                self.registers
                    .set16(register, self.registers.get16(register).wrapping_add(1));
                8
            }

            Instruction::Decrement16 { register } => {
                self.registers
                    .set16(register, self.registers.get16(register).wrapping_sub(1));
                8
            }

            Instruction::DecrementHL => {
                let value = self.read8(bus, self.registers.hl());
                self.registers.f &= FLAG_C;
                self.registers.f |= FLAG_N;
                if value & 0x0F == 0x00 {
                    self.registers.f |= FLAG_H;
                }
                // Decrementing 1 = 0
                if value == 1 {
                    self.registers.f |= FLAG_Z;
                }
                self.write8(bus, self.registers.hl(), value.wrapping_sub(1));
                12
            }

            Instruction::StoreHl { register } => {
                self.write8(bus, self.registers.hl(), self.registers.get8(register));
                8
            }

            Instruction::LoadHl { register } => {
                let value = self.read8(bus, self.registers.hl());
                self.registers.set8(register, value);
                8
            }

            Instruction::Call { addr } => {
                self.push(bus, self.registers.pc);
                self.registers.pc = addr;
                12
            }

            Instruction::CallConditional { condition, addr } => {
                if condition.evaluate(self.registers.f) {
                    self.push(bus, self.registers.pc);
                    self.registers.pc = addr;
                    24
                } else {
                    12
                }
            }

            Instruction::Return => {
                self.registers.pc = self.pop(bus);
                16
            }

            Instruction::ReturnConditional { condition } => {
                if condition.evaluate(self.registers.f) {
                    self.registers.pc = self.pop(bus);
                    20
                } else {
                    8
                }
            }

            Instruction::Push { register } => {
                self.push(bus, self.registers.get16(register));
                16
            }

            Instruction::Pop { register } => {
                let value = self.pop(bus);
                self.registers.set16(register, value);
                12
            }

            Instruction::RotateALeft => {
                let value = self.registers.a;
                let carry = (value >> 7) & 1;
                let last = if self.registers.f & FLAG_C != 0 { 1 } else { 0 };
                let result = (value << 1) | last;
                self.registers.f = if carry != 0 { FLAG_C } else { 0 };
                self.registers.a = result;
                4
            }

            Instruction::RotateARight => {
                let value = self.registers.a;
                let carry = value & 1;
                let msb = if self.registers.f & FLAG_C != 0 {
                    1 << 7
                } else {
                    0
                };
                let result = (value >> 1) | msb;
                self.registers.f = if carry != 0 { FLAG_C } else { 0 };
                if result == 0 {
                    self.registers.f |= FLAG_Z;
                }
                self.registers.a = result;
                4
            }

            Instruction::Compare { value } => {
                self.registers.f = FLAG_N;
                if self.registers.a == value {
                    self.registers.f |= FLAG_Z;
                }
                if self.registers.a < value {
                    self.registers.f |= FLAG_C;
                }
                if (self.registers.a & 0x0F) < (value & 0x0F) {
                    self.registers.f |= FLAG_H;
                }
                8
            }

            Instruction::CompareHl => {
                let value = self.read8(bus, self.registers.hl());
                self.registers.f = FLAG_N;
                if self.registers.a == value {
                    self.registers.f |= FLAG_Z;
                }
                if self.registers.a < value {
                    self.registers.f |= FLAG_C;
                }
                if (self.registers.a & 0x0F) < (value & 0x0F) {
                    self.registers.f |= FLAG_H;
                }
                8
            }

            Instruction::StoreA { addr } => {
                self.write8(bus, addr, self.registers.a);
                12
            }

            Instruction::LoadA { addr } => {
                self.registers.a = self.read8(bus, addr);
                16
            }

            Instruction::Subtract { register } => {
                self.registers.f = FLAG_N;
                let value = self.registers.get8(register);
                if self.registers.a < value {
                    self.registers.f |= FLAG_C;
                }
                if (self.registers.a & 0x0F) < (value & 0x0F) {
                    self.registers.f |= FLAG_H
                }
                self.registers.a = self.registers.a.wrapping_sub(1);
                if self.registers.a == 0 {
                    self.registers.f |= FLAG_Z;
                }
                4
            }

            Instruction::SubImmediate { value } => {
                self.registers.f = FLAG_N;
                if self.registers.a < value {
                    self.registers.f |= FLAG_C;
                }
                if (self.registers.a & 0x0F) < (value & 0x0F) {
                    self.registers.f |= FLAG_H
                }
                self.registers.a = self.registers.a.wrapping_sub(1);
                if self.registers.a == 0 {
                    self.registers.f |= FLAG_Z;
                }
                8
            }

            Instruction::AddImmediate { value } => {
                self.registers.f = 0;
                if (self.registers.a as u16) + (value as u16) > 0xFF {
                    self.registers.f |= FLAG_C;
                }
                if (self.registers.a & 0x0F) + (value & 0x0F) > 0x0F {
                    self.registers.f |= FLAG_H;
                }
                self.registers.a = self.registers.a.wrapping_add(value);
                if self.registers.a == 0 {
                    self.registers.f |= FLAG_Z;
                }
                8
            }

            Instruction::AddHl => {
                self.registers.f = 0;
                let value = self.read8(bus, self.registers.hl());
                if (self.registers.a as u16) + (value as u16) > 0xFF {
                    self.registers.f |= FLAG_C;
                }
                if (self.registers.a & 0x0F) + (value & 0x0F) > 0x0F {
                    self.registers.f |= FLAG_H;
                }
                self.registers.a = self.registers.a.wrapping_add(value);
                if self.registers.a == 0 {
                    self.registers.f |= FLAG_Z;
                }
                8
            }

            Instruction::AddCarryImmediate { value } => {
                let carry = if self.registers.f & FLAG_C != 0 { 1 } else { 0 };
                let result = self.registers.a.wrapping_add(value).wrapping_add(carry);
                self.registers.f = 0;
                if result == 0 {
                    self.registers.f |= FLAG_Z;
                }
                if (self.registers.a as u16) + (value as u16) + (carry as u16) > 0xFF {
                    self.registers.f |= FLAG_C;
                }
                if (self.registers.a & 0x0F) + (value & 0x0F) + (carry & 0x0F) > 0x0F {
                    self.registers.f |= FLAG_H;
                }
                self.registers.a = result;
                8
            }

            Instruction::DisableInterrupts => {
                self.ime = false;
                4
            }

            Instruction::ShiftRight { register } => {
                let value = self.registers.get8(register);
                let carry = value & 1;
                self.registers.set8(register, value);
                self.registers.f = if carry != 0 { FLAG_C } else { 0 };
                if self.registers.get8(register) == 0 {
                    self.registers.f |= FLAG_Z;
                }
                8
            }

            Instruction::RotateLeft { register } => {
                let value = self.registers.get8(register);
                let carry = (value >> 7) & 1;
                let last = if self.registers.f & FLAG_C != 0 { 1 } else { 0 };
                let result = (value << 1) | last;
                self.registers.f = if carry != 0 { FLAG_C } else { 0 };
                if result == 0 {
                    self.registers.f |= FLAG_Z;
                }
                self.registers.set8(register, result);
                8
            }

            Instruction::RotateRight { register } => {
                let value = self.registers.get8(register);
                let carry = value & 1;
                let msb = if self.registers.f & FLAG_C != 0 {
                    1 << 7
                } else {
                    0
                };
                let result = (value >> 1) | msb;
                self.registers.f = if carry != 0 { FLAG_C } else { 0 };
                if result == 0 {
                    self.registers.f |= FLAG_Z;
                }
                self.registers.set8(register, result);
                8
            }

            Instruction::TestBit { bit, register } => {
                let value = self.registers.get8(register);
                let mask = 1u8 << bit;

                let carry = self.registers.f & FLAG_C;
                self.registers.f = carry | FLAG_H;
                if value & mask == 0 {
                    self.registers.f |= FLAG_Z
                }

                8
            }
        }
    }

    fn fetch_ocpode<B: Bus>(&mut self, bus: &mut B) -> u8 {
        let pc = self.registers.pc;
        let opcode = self.read8(bus, pc);

        if self.halt_bug {
            self.halt_bug = false;
        } else {
            self.registers.pc = pc.wrapping_add(1);
        }

        opcode
    }

    fn fetch8<B: Bus>(&mut self, bus: &mut B) -> u8 {
        let pc = self.registers.pc;
        let value = self.read8(bus, pc);

        self.registers.pc = pc.wrapping_add(1);

        value
    }

    fn fetch16<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let low = self.fetch8(bus);
        let high = self.fetch8(bus);

        u16::from_le_bytes([low, high])
    }

    fn push<B: Bus>(&mut self, bus: &mut B, value: u16) {
        self.registers.sp = self.registers.sp.wrapping_sub(2);
        self.write8(bus, self.registers.sp, value as u8);
        self.write8(bus, self.registers.sp.wrapping_add(1), (value >> 8) as u8);
    }

    fn pop<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let low = self.read8(bus, self.registers.sp);
        let high = self.read8(bus, self.registers.sp.wrapping_add(1));
        self.registers.sp = self.registers.sp.wrapping_add(2);
        u16::from_le_bytes([low, high])
    }

    fn read8<B: Bus>(&mut self, bus: &mut B, address: u16) -> u8 {
        bus.read8(address)
    }

    fn write8<B: Bus>(&mut self, bus: &mut B, address: u16, value: u8) {
        bus.write8(address, value);
    }
}

/// Architectural register state of the SM83 CPU.
///
/// The SM83 exposes seven general-purpose 8-bit registers,
/// a flags register, a 16-bit stack pointer, and a 16-bit
/// program counter.
///
/// The registers may also be accessed as the 16-bit pairs
/// AF, BC, DE, and HL.
///
/// The lower four bits of F are unused and must remain zero.
/// Use [`Registers::set_af`] when modifying the AF pair.
#[derive(Debug, Default, Clone, Copy)]
pub struct Registers {
    /// Accumulator register (`A`).
    ///
    /// The accumulator is an 8-bit register used by many
    /// arithmetic, logical, and data movement instructions.
    ///
    /// Numerous instructions implicitly operate on `A`,
    /// including addition, subtraction, bitwise operations,
    /// and comparisons.
    pub a: u8,
    /// Flag register (`F`).
    ///
    /// Stores the processor's condition flags in its
    /// upper four bits:
    ///
    /// | Bit | Flag | Meaning |
    /// |-----|------|---------|
    /// | 7 | `Z` | Zero |
    /// | 6 | `N` | Subtraction |
    /// | 5 | `H` | Half-carry |
    /// | 4 | `C` | Carry |
    ///
    /// Bits `0..=3` are unused and must remain zero
    /// during correct CPU emulation.
    ///
    /// The flags are modified by arithmetic, logical,
    /// and certain other instructions.
    pub f: u8,

    /// General-purpose register `B`.
    ///
    /// An 8-bit register that can also serve as the
    /// high byte of the 16-bit `BC` register pair.
    pub b: u8,
    /// General-purpose register `C`.
    ///
    /// An 8-bit register that can also serve as the
    /// low byte of the 16-bit `BC` register pair.
    ///
    /// Certain instructions use `C` as an offset
    /// for accessing high-memory I/O addresses.
    pub c: u8,
    /// General-purpose register `D`.
    ///
    /// An 8-bit register that can also serve as the
    /// high byte of the 16-bit `DE` register pair.
    pub d: u8,
    /// General-purpose register `E`.
    ///
    /// An 8-bit register that can also serve as the
    /// low byte of the 16-bit `DE` register pair.
    pub e: u8,
    /// General-purpose register `H`.
    ///
    /// An 8-bit register that can also serve as the
    /// high byte of the 16-bit `HL` register pair.
    ///
    /// The `HL` pair is frequently used as a memory
    /// address by load and arithmetic instructions.
    pub h: u8,
    /// General-purpose register `L`.
    ///
    /// An 8-bit register that can also serve as the
    /// low byte of the 16-bit `HL` register pair.
    ///
    /// Together with `H`, it forms the commonly used
    /// 16-bit memory pointer `HL`.
    pub l: u8,

    /// Stack pointer register (`SP`).
    ///
    /// A 16-bit register identifying the current
    /// stack position in the CPU address space.
    ///
    /// Stack operations, subroutine calls, returns,
    /// and interrupt handling use this register.
    ///
    /// The SM83 stack grows toward lower addresses.
    pub sp: u16,
    /// Program counter register (`PC`).
    ///
    /// A 16-bit register tracking the CPU's instruction
    /// execution position.
    ///
    /// During instruction execution, this register is
    /// updated according to instruction length, control
    /// flow, and interrupt behaviour.
    pub pc: u16,
}

impl Registers {
    /// Returns the combined AF register pair.
    ///
    /// The lower four bits of F are masked out.
    pub fn af(&self) -> u16 {
        u16::from_be_bytes([self.a, self.f & FLAG_MASK])
    }

    /// Returns the combined BC register pair.
    pub fn bc(&self) -> u16 {
        u16::from_be_bytes([self.b, self.c])
    }

    /// Returns the combined DE register pair.
    pub fn de(&self) -> u16 {
        u16::from_be_bytes([self.d, self.e])
    }

    /// Returns the combined HL register pair
    pub fn hl(&self) -> u16 {
        u16::from_be_bytes([self.h, self.l])
    }

    /// Sets the AF register pair.
    ///
    /// The upper byte is written to A. The lower byte is
    /// written to F after clearing its unused lower nybble.
    pub fn set_af(&mut self, value: u16) {
        let [a, f] = value.to_be_bytes();

        self.a = a;
        self.f = f & FLAG_MASK;
    }

    /// Sets the BC register pair.
    pub fn set_bc(&mut self, value: u16) {
        let [b, c] = value.to_be_bytes();

        self.b = b;
        self.c = c;
    }

    /// Sets the DE register pair.
    pub fn set_de(&mut self, value: u16) {
        let [d, e] = value.to_be_bytes();

        self.d = d;
        self.e = e;
    }

    /// Sets the HL register pair.
    pub fn set_hl(&mut self, value: u16) {
        let [h, l] = value.to_be_bytes();

        self.h = h;
        self.l = l;
    }

    /// Reads the value of an individual 8-bit register.
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

    /// Writes a value to an individual 8 bit register.
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

    /// Reads the value of a 16-bit register pair.
    pub fn get16(&self, register: Register16) -> u16 {
        match register {
            Register16::AF => self.af(),
            Register16::BC => self.bc(),
            Register16::DE => self.de(),
            Register16::HL => self.hl(),
            Register16::SP => self.sp,
        }
    }

    /// Writes a value to a 16-bit register pair.
    pub fn set16(&mut self, register: Register16, value: u16) {
        match register {
            Register16::AF => self.set_af(value),
            Register16::BC => self.set_bc(value),
            Register16::DE => self.set_de(value),
            Register16::HL => self.set_hl(value),
            Register16::SP => self.sp = value,
        }
    }
}

/// Identifies an individual 8-bit SM83 register.
///
/// Does not include `F`, which is manipulated through flag
/// operations and the AF register pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Register8 {
    /// Accumulator register.
    A,
    /// General-purpose B register.
    B,
    /// General-purpose C register.
    C,
    /// General-purpse D register.
    D,
    /// General-purpose E register.
    E,
    /// General-prupose H register.
    H,
    /// General-purpose L register.
    L,
}

impl std::fmt::Display for Register8 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Identifies a 16-bit register pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Register16 {
    /// AF register pair.
    AF,
    /// General purpose B and C registers.
    BC,
    /// General purpose D and E registers,
    DE,
    /// General purpose H and L registers.
    HL,
    /// Stack pointer.
    SP,
}

impl std::fmt::Display for Register16 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Conditions used for conditional execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    /// The Zero Flag is not set.
    NotZero,
    /// The Zero Flag is set.
    Zero,
    /// The Carry Flag is not set.
    NotCarry,
    /// The Carry Flag is set.
    Carry,
}

impl Condition {
    /// Evaluate the condition based on the given flags.
    pub fn evaluate(self, flags: u8) -> bool {
        match self {
            Self::NotZero => flags & FLAG_Z == 0,
            Self::Zero => flags & FLAG_Z != 0,
            Self::NotCarry => flags & FLAG_C == 0,
            Self::Carry => flags & FLAG_C != 0,
        }
    }
}

impl std::fmt::Display for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotZero => write!(f, "NZ"),
            Self::Zero => write!(f, "Z"),
            Self::NotCarry => write!(f, "NC"),
            Self::Carry => write!(f, "C"),
        }
    }
}

/// Snapshot of the SM83's execution-control state.
///
/// Contains information about interrupt handling and
/// processor execution modes, separate from the architectural
/// registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuStatus {
    /// Whether maskable interrupts are enabled.
    pub ime: bool,

    /// Whether the CPU is halted.
    pub halted: bool,

    /// Whether the CPU is stopped.
    pub stopped: bool,

    /// Remaining instruction boundaries before a
    /// scheduled interrupt-master-enable takes effect.
    pub ime_enable_delay: u8,
}

/// Identifies an SM83 opcode and its instruction table.
///
/// The SM83 uses a primary opcode table and a separate table
/// selected by the `0xCB` prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    /// Opcode from the primary instruction table.
    Base(u8),
    /// Opcode from the CB-prefixed instruction table.
    Cb(u8),
}

/// A decoded SM83 instruction.
///
/// Immediate operands are resolved during decoding.
/// Execution operates on this representation rather than
/// fetching immediate values again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    /// No operation
    Nop,

    /// Load an immediate 8-bit value into a register.
    LoadImmediate8 {
        /// The register to load into.
        register: Register8,
        /// The value to load into the register.
        value: u8,
    },

    /// Load the value of the source register into the destination register.
    LoadSourceDest {
        /// The source register.
        source: Register8,
        /// The destination register.
        dest: Register8,
    },

    /// Load an immediate 16-bit value into a register pair.
    LoadImmediate16 {
        /// The register pair to load into.
        register: Register16,
        /// The value to load into the register pair.
        value: u16,
    },

    /// Jump to an address.
    Jump {
        /// The address to jump to.
        addr: u16,
    },

    /// Jump to the address pointed to by HL.
    JumpHl,

    /// Perform a jump relative to the current location.
    JumpRelative {
        /// The offset to jump to.
        offset: i8,
    },

    /// Perform a conditional jump relative to the current location.
    JumpRelativeConditional {
        /// The condition to evaluate.
        condition: Condition,
        /// The offset to jump to.
        offset: i8,
    },

    /// Add the contents of a register pair to HL.
    AddHlRegister {
        /// The register pair to add.
        register: Register16,
    },

    /// Load the value of A into the memory address pointed to by HL, and increment HL.
    LoadIncHl,

    /// Load the value of A into the memory address pointed to by HL, and decrement HL.
    LoadDecHl,

    /// Load the value pointed at by HL into A, and increment HL
    LoadAIncHl,

    /// Load the value at the memory address pointed to by a register pair into A.
    LoadRegPair {
        /// The register pair to fetch from.
        register: Register16,
    },

    /// Store the contents of A at the memory address pointed to by a register pair.
    StoreRegPair {
        /// The register pair to store into.
        register: Register16,
    },

    /// OR the A register with the contents of a register.
    Or {
        /// The register to OR with.
        register: Register8,
    },

    /// OR the A register with the contents pointed to by HL.
    OrHl,

    /// XOR the A register with the contents of a register.
    Xor {
        /// The register to XOR with
        register: Register8,
    },

    /// XOR the A register with an immediate 8-bit value.
    XorImmediate {
        /// The value to XOR with.
        value: u8,
    },

    /// XOR the A register with the value pointed to by HL.
    XorHl,

    /// AND the A register with the contents of a register.
    And {
        /// The register to AND with.
        register: Register8,
    },

    /// AND the A register with an immediate 8-bit value
    AndImmediate {
        /// The value to AND with.
        value: u8,
    },

    /// Load the value in A to $FF00 + C.
    LoadHighRamC,

    /// Load the the value in A to $FF00 + offset
    LoadHigh {
        /// The offset to write to.
        offset: u8,
    },

    /// Load the value at $FF00 + offset into A
    ReadHigh {
        /// The offset to write from.
        offset: u8,
    },

    /// Increment a register by 1.
    Increment {
        /// The register to increment.
        register: Register8,
    },

    /// Decrement a register by 1.
    Decrement {
        /// The register to decrement.
        register: Register8,
    },

    /// Increment a 16-bit register pair.
    Increment16 {
        /// The register pair to increment.
        register: Register16,
    },

    /// Decrement a 16-bit register pair.
    Decrement16 {
        /// The register pair to decrement.
        register: Register16,
    },

    /// Decremnt the value pointed to by HL.
    DecrementHL,

    /// Load the value of a register into the memory address pointed to by HL.
    StoreHl {
        /// The register to load.
        register: Register8,
    },

    /// Load the value pointed to by HL into a register.
    LoadHl {
        /// The register to load into.
        register: Register8,
    },

    /// Call a subroutine.
    Call {
        /// The address of the subroutine to jump to.
        addr: u16,
    },

    /// Conditionally call a subroutine.
    CallConditional {
        /// The condition to evaluate.
        condition: Condition,

        /// The address of the subroutine.
        addr: u16,
    },

    /// Return from a subroutine.
    Return,

    /// Conditionally return from a subroutine.
    ReturnConditional {
        /// The condition to evaluate.
        condition: Condition,
    },

    /// Push a register pair onto the stack.
    Push {
        /// The register pair to push.
        register: Register16,
    },

    /// Pop from the stack into a register pair.
    Pop {
        /// The register pair to pop into.
        register: Register16,
    },

    /// Rotate the A register left.
    RotateALeft,

    /// Rotate the A register right.
    RotateARight,

    /// Compare register A.
    Compare {
        /// The value to compare A with
        value: u8,
    },

    /// Compare register A with the value at the address pointed to by HL.
    CompareHl,

    /// Store the contents of register A at a memory address.
    StoreA {
        /// The address to write to.
        addr: u16,
    },

    /// Load the value at a given memory address into A.
    LoadA {
        /// The address to store at.
        addr: u16,
    },

    /// Subtract the value of a register from A.
    Subtract {
        /// The register to subtract.
        register: Register8,
    },

    /// Subtract an immediate 8-bit value to A.
    SubImmediate {
        /// The immediate value to subtract.
        value: u8,
    },

    /// Add an immediate 8-bit value to A.
    AddImmediate {
        /// The immediate value to add.
        value: u8,
    },

    /// Add the value pointed to by HL to A.
    AddHl,

    /// Add an immediate value to A and the carry flag.
    AddCarryImmediate {
        /// The immediate value to add.
        value: u8,
    },

    /// Disable interrupts.
    DisableInterrupts,

    /// Shift the contents of a register to the right.
    ShiftRight {
        /// The register to shift.
        register: Register8,
    },

    /// Rotate the contents of a register to the left.
    RotateLeft {
        /// The register to rotate left.
        register: Register8,
    },

    /// Rotate the contents of a register to the right.
    RotateRight {
        /// The register to rotate.
        register: Register8,
    },

    /// Test if a bit of a register is set
    TestBit {
        /// The bit to test.
        bit: u8,
        /// The register to test.
        register: Register8,
    },
}

impl std::fmt::Display for Instruction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Instruction::Nop => write!(f, "NOP"),
            Instruction::LoadImmediate8 { register, value } => {
                write!(f, "LD {register}, ${value:02X}")
            }
            Instruction::LoadSourceDest { source, dest } => write!(f, "LD {dest}, {source}"),
            Instruction::LoadImmediate16 { register, value } => {
                write!(f, "LD {register}, ${value:04X}")
            }
            Instruction::Jump { addr } => write!(f, "JP ${addr:04X}"),
            Instruction::JumpHl => write!(f, "JP HL"),
            Instruction::JumpRelative { offset } => write!(f, "JR @{offset:+}"),
            Instruction::JumpRelativeConditional { condition, offset } => {
                write!(f, "JR {condition}, @{offset:+}")
            }
            Instruction::AddHlRegister { register } => write!(f, "ADD HL, {register}"),
            Instruction::LoadIncHl => write!(f, "LD (HL+), A"),
            Instruction::LoadDecHl => write!(f, "LD (HL-), A"),
            Instruction::LoadAIncHl => write!(f, "LD A, (HL+)"),
            Instruction::LoadRegPair { register } => write!(f, "LD A, ({register})"),
            Instruction::StoreRegPair { register } => write!(f, "LD ({register}), A"),
            Instruction::Or { register } => write!(f, "OR {register}"),
            Instruction::OrHl => write!(f, "OR (HL)"),
            Instruction::Xor { register } => write!(f, "XOR {register}"),
            Instruction::XorHl => write!(f, "XOR HL"),
            Instruction::XorImmediate { value } => write!(f, "XOR ${value:02X}"),
            Instruction::And { register } => write!(f, "AND {register}"),
            Instruction::AndImmediate { value } => write!(f, "AND ${value:02X}"),
            Instruction::LoadHighRamC => write!(f, "LD (C), A"),
            Instruction::LoadHigh { offset } => write!(f, "LDH (${offset:02x}), A"),
            Instruction::ReadHigh { offset } => write!(f, "LDH A, (${offset:02x})"),
            Instruction::Increment { register } => write!(f, "INC {register}"),
            Instruction::Decrement { register } => write!(f, "DEC {register}"),
            Instruction::Increment16 { register } => write!(f, "INC {register}"),
            Instruction::Decrement16 { register } => write!(f, "DEC {register}"),
            Instruction::DecrementHL => write!(f, "DEC (HL)"),
            Instruction::StoreHl { register } => write!(f, "LD (HL), {register}"),
            Instruction::LoadHl { register } => write!(f, "LD {register}, (HL)"),
            Instruction::Call { addr } => write!(f, "CALL ${addr:04X}"),
            Instruction::CallConditional { condition, addr } => {
                write!(f, "CALL {condition} #{addr:04X}")
            }
            Instruction::Return => write!(f, "RET"),
            Instruction::ReturnConditional { condition } => write!(f, "RET {condition}"),
            Instruction::Push { register } => write!(f, "PUSH {register}"),
            Instruction::Pop { register } => write!(f, "POP {register}"),
            Instruction::RotateALeft => write!(f, "RLA"),
            Instruction::RotateARight => write!(f, "RRA"),
            Instruction::Compare { value } => write!(f, "CP ${value:02X}"),
            Instruction::CompareHl => write!(f, "CP (HL)"),
            Instruction::StoreA { addr } => write!(f, "LD (${addr:04X}), A"),
            Instruction::LoadA { addr } => write!(f, "LD A, (${addr:04X})"),
            Instruction::Subtract { register } => write!(f, "SUB {register}"),
            Instruction::SubImmediate { value } => write!(f, "SUB %${value:02X}"),
            Instruction::AddImmediate { value } => write!(f, "ADD ${value:02X}"),
            Instruction::AddHl => write!(f, "ADD (HL)"),
            Instruction::AddCarryImmediate { value } => write!(f, "ADC ${value:02X}"),
            Instruction::DisableInterrupts => write!(f, "DI"),

            Instruction::ShiftRight { register } => write!(f, "SRL {register}"),
            Instruction::RotateLeft { register } => write!(f, "RL {register}"),
            Instruction::RotateRight { register } => write!(f, "RR {register}"),
            Instruction::TestBit { bit, register } => write!(f, "BIT {bit}, {register}"),
        }
    }
}

/// Describes an instruction successfully executed by the CPU.
///
/// Can be used by debuggers, tracers, and test harnesses
/// without requiring a second instruction decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepResult {
    /// Program counter before instruction fetching.
    pub pc: u16,
    /// Opcode executed.
    pub opcode: Opcode,
    /// Decoded instruction and resolved immediate operands.
    pub instruction: Instruction,
    /// CPU T-cycles consumed by the instruction.
    ///
    /// A T-cycle is one period of the CPU clock. In normal
    /// speed mode, most SM84 machine cycles contain four
    /// T-cycles.
    pub cycles: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DecodedInstruction {
    pc: u16,
    opcode: Opcode,
    instruction: Instruction,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Access {
        Read(u16),
        Write(u16, u8),
    }

    struct TestBus {
        memory: Vec<u8>,
        accesses: Vec<Access>,
    }

    impl TestBus {
        fn new() -> Self {
            Self {
                memory: vec![0; 0x10000],
                accesses: Vec::new(),
            }
        }

        fn load(&mut self, address: u16, program: &[u8]) {
            let start = usize::from(address);
            let end = start + program.len();

            self.memory[start..end].copy_from_slice(program);
        }
    }

    impl Bus for TestBus {
        fn read8(&mut self, address: u16) -> u8 {
            self.accesses.push(Access::Read(address));

            self.memory[usize::from(address)]
        }

        fn write8(&mut self, address: u16, value: u8) {
            self.accesses.push(Access::Write(address, value));

            self.memory[usize::from(address)] = value;
        }
    }

    fn cpu_at(pc: u16) -> Sm83 {
        Sm83::with_registers(Registers {
            pc,
            ..Registers::default()
        })
    }

    #[test]
    fn register_pairs_are_big_endian() {
        let mut registers = Registers::default();

        registers.set_bc(0x1234);
        registers.set_de(0x5678);
        registers.set_hl(0x9ABC);

        assert_eq!(registers.b, 0x12);
        assert_eq!(registers.c, 0x34);

        assert_eq!(registers.bc(), 0x1234);
        assert_eq!(registers.de(), 0x5678);
        assert_eq!(registers.hl(), 0x9ABC);
    }

    #[test]
    fn af_masks_lower_flag_bits() {
        let mut registers = Registers::default();

        registers.set_af(0x12FF);

        assert_eq!(registers.a, 0x12);
        assert_eq!(registers.f, 0xF0);
        assert_eq!(registers.af(), 0x12F0);
    }

    #[test]
    fn nop_advances_pc_and_consumes_four_cycles() {
        let mut bus = TestBus::new();
        bus.load(0x0100, &[0x00]);

        let mut cpu = cpu_at(0x0100);

        let result = cpu.step_with_trace(&mut bus).unwrap();

        assert_eq!(result.pc, 0x0100);
        assert_eq!(result.opcode, Opcode::Base(0x00));
        assert_eq!(result.instruction, Instruction::Nop);
        assert_eq!(result.cycles, 4);

        assert_eq!(cpu.registers().pc, 0x0101);

        assert_eq!(bus.accesses, vec![Access::Read(0x0100)]);
    }

    #[test]
    fn unimplemented_opcode_returns_error() {
        let mut bus = TestBus::new();

        bus.load(0x0100, &[0x01]);

        let mut cpu = cpu_at(0x0100);

        let result = cpu.step(&mut bus);

        assert_eq!(
            result,
            Err(CpuError::Unimplemented {
                pc: 0x0100,
                opcode: Opcode::Base(0x01),
            })
        );

        assert_eq!(cpu.registers().pc, 0x0100);
    }

    #[test]
    fn cb_prefix_reads_the_second_opcode() {
        let mut bus = TestBus::new();

        // CB 11 = RL C, not implemented yet.
        bus.load(0x0100, &[0xCB, 0x11]);

        let mut cpu = cpu_at(0x0100);

        let result = cpu.step(&mut bus);

        assert_eq!(
            result,
            Err(CpuError::Unimplemented {
                pc: 0x0100,
                opcode: Opcode::Cb(0x11),
            })
        );

        assert_eq!(cpu.registers().pc, 0x0100);

        assert_eq!(
            bus.accesses,
            vec![Access::Read(0x0100), Access::Read(0x0101),]
        );
    }

    #[test]
    fn illegal_opcode_is_distinguished() {
        let mut bus = TestBus::new();

        bus.load(0x0100, &[0xD3]);

        let mut cpu = cpu_at(0x0100);

        assert_eq!(
            cpu.step(&mut bus),
            Err(CpuError::Illegal {
                pc: 0x0100,
                opcode: 0xD3,
            })
        );
    }

    #[test]
    fn instruction_trace_contains_decoded_details() {
        let mut bus = TestBus::new();

        bus.load(0x0100, &[0x00]);

        let mut cpu = cpu_at(0x0100);

        let result = cpu.step_with_trace(&mut bus).unwrap();

        assert_eq!(result.pc, 0x0100);
        assert_eq!(result.opcode, Opcode::Base(0x00));
        assert_eq!(result.cycles, 4);

        assert_eq!(result.instruction.to_string(), "NOP");
    }
}
