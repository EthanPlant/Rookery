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
}

impl std::fmt::Display for Instruction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Instruction::Nop => write!(f, "NOP"),
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
