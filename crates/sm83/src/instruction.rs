use std::fmt;

use crate::{FLAG_C, FLAG_Z};

/// A decoded SM83 instruction, with immediate bytes already resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    /// NOP
    Nop,
    /// Load an 8-bit destination from a source.
    Load8 {
        /// Destination.
        dst: Operand8,
        /// Source.
        src: Source8,
    },
    /// Load a 16-bit immediate into a register pair or SP.
    Load16 {
        /// Destination.
        dst: Register16,
        /// Immediate value.
        value: u16,
    },
    /// Load A from an indirect address.
    LoadAFrom(Address8),
    /// Store A to an indirect address.
    StoreATo(Address8),
    /// Increment an 8-bit operand.
    Inc8(Operand8),
    /// Decrement an 8-bit operand.
    Dec8(Operand8),
    /// Increment a 16-bit register.
    Inc16(Register16),
    /// Decrement a 16-bit register
    Dec16(Register16),
    /// Add a 16-bit register pair to HL.
    AddHL(Register16),
    /// Accumulator ALU operation.
    Alu {
        /// ALU operation.
        op: AluOp,
        /// Source operand.
        src: Source8,
    },
    /// Decimal adjust accumulator.
    Daa,
    /// Invert the bits of A.
    Complement,
    /// Set the carry flag.
    SetCarry,
    /// Flip the carry flag.
    InvertCarry,
    /// Absolute jump.
    Jump {
        /// Condition to evaluate. `None` for an unconditional jump.
        condition: Option<Condition>,
        /// Destination address.
        address: u16,
    },
    /// Indirect jump through HL.
    JumpHL,
    /// Signed relative jump.
    JumpRelative {
        /// Condition to evaluate. `None` for an unconditional jump.
        condition: Option<Condition>,
        /// Signed 8-bit displacement relative to the next instruction.
        offset: i8,
    },
    /// Subroutine call.
    Call {
        /// Condition to evaluate. `None` for an unconditional call.
        condition: Option<Condition>,
        /// Subroutine address.
        address: u16,
    },
    /// Subroutine return.
    Return {
        /// Condition to evaluate. `None` for an unconditional return.
        condition: Option<Condition>,
    },
    /// Return from interrupt
    ReturnInterrupt,
    /// Jump to a prefined reset vector.
    Reset(u16),
    /// Push a register pair onto the stack.
    Push(Register16),
    /// Pop a register pair onto the stack.
    Pop(Register16),
    /// Store the stack pointer at an immediate address.
    StoreStackPointer(u16),
    /// Load the value of HL into the stack pointer.
    LoadStackPointerHl,
    /// Add a signed offset to the stack pointer.
    AddStackPointer(i8),
    /// Add a signed offset to the stack pointer, and store the result in HL.
    StoreOffsetStackPointer(i8),

    /// Special accumulator rotation (always clears Z).
    RotateA(RotateKind),
    /// CB-prefixed operation.
    Cb {
        /// Extended opcode operation.
        op: CbOperation,
        /// Target for the operation.
        target: Operand8,
    },
    /// Enable interrupts.
    EnableInterrupts,
    /// Disable interrupts.
    DisableInterrupts,
    /// Enter the CPU's lower-power HALT state.
    Halt,
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nop => f.write_str("NOP"),
            Self::Load8 { dst, src } => write!(f, "LD {dst}, {src}"),
            Self::Load16 { dst, value } => write!(f, "LD {dst}, ${value:04X}"),
            Self::LoadAFrom(addr) => match addr {
                Address8::HighC | Address8::HighImmediate(_) => write!(f, "LDH A, {addr}"),
                _ => write!(f, "LD A, {addr}"),
            },
            Self::StoreATo(addr) => match addr {
                Address8::HighC | Address8::HighImmediate(_) => write!(f, "LDH {addr}, A"),
                _ => write!(f, "LD {addr}, A"),
            },
            Self::Inc8(dst) => write!(f, "INC {dst}"),
            Self::Dec8(dst) => write!(f, "DEC {dst}"),
            Self::Inc16(dst) => write!(f, "INC {dst}"),
            Self::Dec16(dst) => write!(f, "DEC {dst}"),
            Self::AddHL(src) => write!(f, "ADD HL, {src}"),
            Self::Daa => f.write_str("DAA"),
            Self::Complement => f.write_str("CPL"),
            Self::SetCarry => f.write_str("SCF"),
            Self::InvertCarry => f.write_str("CCF"),
            Self::Alu { op, src } => write!(f, "{op}, {src}"),
            Self::Jump { condition, address } => match condition {
                Some(cond) => write!(f, "JP {cond}, ${address:04X}"),
                None => write!(f, "JP ${address:04X}"),
            },
            Self::JumpHL => f.write_str("JP HL"),
            Self::JumpRelative { condition, offset } => match condition {
                Some(cond) => write!(f, "JR {cond}, {offset:+}"),
                None => write!(f, "JR {offset:+}"),
            },
            Self::Call { condition, address } => match condition {
                Some(cond) => write!(f, "CALL {cond}, ${address:04X}"),
                None => write!(f, "CALL ${address:04X}"),
            },
            Self::Return { condition } => match condition {
                Some(cond) => write!(f, "RET {cond}"),
                None => f.write_str("RET"),
            },
            Self::ReturnInterrupt => f.write_str("RETI"),
            Self::Reset(addr) => write!(f, "RST {addr:02X}"),
            Self::Push(value) => write!(f, "PUSH {value}"),
            Self::Pop(destination) => write!(f, "POP {destination}"),
            Self::StoreStackPointer(addr) => write!(f, "LD (${addr:04X}), SP"),
            Self::LoadStackPointerHl => f.write_str("LD SP, HL"),
            Self::AddStackPointer(offset) => write!(f, "ADD SP, {offset:+}"),
            Self::StoreOffsetStackPointer(offset) => write!(f, "LD HL, SP+{offset:+}"),
            Self::RotateA(kind) => f.write_str(match kind {
                RotateKind::Rlc => "RLCA",
                RotateKind::Rrc => "RRCA",
                RotateKind::Rl => "RLA",
                RotateKind::Rr => "RLA",
            }),
            Self::Cb { op, target } => match op {
                CbOperation::Rotate(kind) => write!(
                    f,
                    "{}, {target}",
                    match kind {
                        RotateKind::Rlc => "RLC",
                        RotateKind::Rrc => "RRC",
                        RotateKind::Rl => "RL",
                        RotateKind::Rr => "RR",
                    }
                ),
                CbOperation::Sla => write!(f, "SLA {target}"),
                CbOperation::Sra => write!(f, "SRA {target}"),
                CbOperation::Swap => write!(f, "SWAP {target}"),
                CbOperation::Srl => write!(f, "SRL {target}"),
                CbOperation::Bit(bit) => write!(f, "BIT {bit}, {target}"),
                CbOperation::Res(bit) => write!(f, "RES {bit}, target"),
                CbOperation::Set(bit) => write!(f, "SET {bit}, target"),
            },
            Self::EnableInterrupts => f.write_str("EI"),
            Self::DisableInterrupts => f.write_str("DI"),
            Self::Halt => f.write_str("HALT"),
        }
    }
}

/// Identifies to base or CB-prefixed opcode table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    /// Base opcode.
    Base(u8),
    /// Opcode following the 0xCB prefix.
    Cb(u8),
}

/// 0xCB-prefixed bit, rotate and shift operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CbOperation {
    /// Rotate a byte.
    Rotate(RotateKind),
    /// Arithmetic left shift.
    Sla,
    /// Arithemtic right shift.
    Sra,
    /// Swap upper and lower nibbles.
    Swap,
    /// Logical right shift.
    Srl,
    /// Test a bit without modifying the operand.
    Bit(u8),
    /// Clear one bit.
    Res(u8),
    /// Set one bit
    Set(u8),
}

/// Accumulator arithmetic/logic operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AluOp {
    /// Add.
    Add,
    /// Add with carry.
    Adc,
    /// Subtract.
    Sub,
    /// Subtract with carry.
    Sbc,
    /// Bitwise AND.
    And,
    /// Bitwise XOR.
    Xor,
    /// Bitwise OR.
    Or,
    /// Compare without changing A.
    Cp,
}

impl AluOp {
    pub(crate) fn from_bits(bits: u8) -> Self {
        match bits & 7 {
            0 => Self::Add,
            1 => Self::Adc,
            2 => Self::Sub,
            3 => Self::Sbc,
            4 => Self::And,
            5 => Self::Xor,
            6 => Self::Or,
            7 => Self::Cp,
            _ => unreachable!(),
        }
    }
}

impl fmt::Display for AluOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Add => "ADD",
            Self::Adc => "ADC",
            Self::Sub => "SUB",
            Self::Sbc => "SBC",
            Self::And => "AND",
            Self::Xor => "XOR",
            Self::Or => "OR",
            Self::Cp => "CP",
        })
    }
}

/// Read-only source used by loads and accumulator ALU operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source8 {
    /// Register or (HL)
    Operand(Operand8),

    /// A byte fetched while decoding.
    Immediate(u8),
}

impl Source8 {
    pub(crate) fn is_memory(self) -> bool {
        matches!(self, Self::Operand(Operand8::IndirectHL))
    }

    pub(crate) fn is_immediate(self) -> bool {
        matches!(self, Self::Immediate(_))
    }
}

impl fmt::Display for Source8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Operand(operand) => write!(f, "{operand}"),
            Self::Immediate(value) => write!(f, "${value:02X}"),
        }
    }
}

/// Addressing modes used by loads that implicitly use register A.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Address8 {
    /// Memory pointed to by BC.
    BC,
    /// Memory pointed to by DE.
    DE,
    /// Memory pointed to by HL, then increment HL.
    HLInc,
    /// Memory pointed to by HL, then decrement HL.
    HLDec,
    /// Address $FF00 + C
    HighC,
    /// Address $FF00 + an immediate 8-bit offset.
    HighImmediate(u8),
    /// An immediate 16-bit address.
    Immediate(u16),
}

impl fmt::Display for Address8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BC => write!(f, "(BC)"),
            Self::DE => write!(f, "(DE)"),
            Self::HLInc => write!(f, "(HL+)"),
            Self::HLDec => write!(f, "(HL-)"),
            Self::HighC => write!(f, "(C)"),
            Self::HighImmediate(offset) => write!(f, "#{offset:02X}"),
            Self::Immediate(addr) => write!(f, "${addr:04X}"),
        }
    }
}

/// A condition tested against the CPU's flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    /// Z is clear.
    NotZero,
    /// Z is set.
    Zero,
    /// C is clear.
    NotCarry,
    /// C is set.
    Carry,
}

impl Condition {
    /// Test the condition against a set of flags
    pub fn evaluate(self, flags: u8) -> bool {
        match self {
            Self::NotZero => flags & FLAG_Z == 0,
            Self::Zero => flags & FLAG_Z != 0,
            Self::NotCarry => flags & FLAG_C == 0,
            Self::Carry => flags & FLAG_C != 0,
        }
    }

    pub(crate) fn from_bits(bits: u8) -> Self {
        match bits & 3 {
            0 => Self::NotZero,
            1 => Self::Zero,
            2 => Self::NotCarry,
            3 => Self::Carry,
            _ => unreachable!(),
        }
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotZero => "NZ",
            Self::Zero => "Z",
            Self::NotCarry => "NC",
            Self::Carry => "C",
        })
    }
}

/// A writable 8-bit operand, including the location pointed to by HL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand8 {
    /// A physical register.
    Register(Register8),
    /// The byte at address HL.
    IndirectHL,
}

impl Operand8 {
    /// Decode the 3-bit `r8` field used throughout the SM83 instruction map.
    pub fn from_bits(bits: u8) -> Self {
        match bits & 7 {
            0 => Self::Register(Register8::B),
            1 => Self::Register(Register8::C),
            2 => Self::Register(Register8::D),
            3 => Self::Register(Register8::E),
            4 => Self::Register(Register8::H),
            5 => Self::Register(Register8::L),
            6 => Self::IndirectHL,
            7 => Self::Register(Register8::A),
            _ => unreachable!(),
        }
    }

    pub(crate) fn is_memory(self) -> bool {
        matches!(self, Self::IndirectHL)
    }
}

impl fmt::Display for Operand8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Register(register) => write!(f, "{register}"),
            Self::IndirectHL => write!(f, "(HL)"),
        }
    }
}

/// A general-purpose 8-bit CPU register (F is intentionally excluded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Register8 {
    /// Accumulator.
    A,
    /// Register B.
    B,
    /// Register C.
    C,
    /// Register D.
    D,
    /// Register E.
    E,
    /// Register H.
    H,
    /// Register L.
    L,
}

impl fmt::Display for Register8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// A 16-bit register pair or the stack pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Register16 {
    /// AF pair.
    AF,
    /// BC pair.
    BC,
    /// DE pair.
    DE,
    /// HL pair.
    HL,
    /// Stack pointer.
    SP,
}

impl fmt::Display for Register16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Left/right circular or through-carry rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotateKind {
    /// Rotate left circularly.
    Rlc,
    /// Rotate right circularly.
    Rrc,
    /// Rotate left through carry.
    Rl,
    /// Rotate right through carry.
    Rr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecodedInstruction {
    pub pc: u16,
    pub opcode: Opcode,
    pub instruction: Instruction,
}
