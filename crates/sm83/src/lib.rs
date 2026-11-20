//! Sharp SM83 instruction interpreter for the DMG and CGB.
//!
//! One call to [`Sm83::step`] executes one instruction and returns CPU
//! T-cycles. Memory stays outside the CPU behind the [`Bus`] trait.
//!
//! Interrupt dispatch, HALT, STOP and EI are deliberately left for the
//! remaining CPU milestone. They should be integrated at instruction
//! boundaries, rather than guessed inside the memory bus.
#![warn(missing_docs)]

mod alu;
mod bus;
mod cpu;
mod decode;
mod execute;
mod instruction;
mod registers;
#[cfg(test)]
mod tests;

pub use bus::Bus;
pub use cpu::{CpuError, CpuStatus, Sm83, StepKind, StepResult};
pub use instruction::{
    Address8, AluOp, CbOperation, Condition, Instruction, Opcode, Operand8, Register8, Register16,
    RotateKind, Source8,
};
pub use registers::{FLAG_C, FLAG_H, FLAG_N, FLAG_Z, Registers};
