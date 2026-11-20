use thiserror::Error;

use crate::{Bus, Instruction, Opcode, Registers, registers::FLAG_MASK};

/// The Sharp SM83 processor state.
#[derive(Debug, Clone, Default)]
pub struct Sm83 {
    pub(crate) registers: Registers,
    pub(crate) control: ExecutionControl,
}

impl Sm83 {
    /// Construct with zero-intialized registers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with registers initialized to the DMG post-boot state.
    pub fn with_dmg_boot() -> Self {
        Self {
            registers: Registers {
                a: 0x01,
                f: 0xB0, // Z, H, and C.
                b: 0xFF,
                c: 0x13,
                d: 0x00,
                e: 0xC1,
                h: 0x84,
                l: 0x03,
                pc: 0x0100,
                sp: 0xFFFE,
            },
            control: ExecutionControl::default(),
        }
    }

    /// Construct with a caller provided register date, masking unused F-bits.
    pub fn with_registers(mut registers: Registers) -> Self {
        registers.f &= FLAG_MASK;
        Self {
            registers,
            control: ExecutionControl::default(),
        }
    }

    /// Execute one instruction and return its T-cycle clost.
    ///
    /// # Errors
    ///
    /// Returns [`CpuError`] for an illegal or unimplemented opcode.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> Result<u32, CpuError> {
        Ok(self.step_with_trace(bus)?.cycles)
    }

    /// Execute one instruction and return its decoded trace information.
    ///
    /// On a decode error, PC is restored. Bus read side effects cannot be rolled
    /// back.
    ///
    /// # Errors
    ///
    /// Returns [`CpuError`] for an illegal or unimplemented opcode.
    pub fn step_with_trace<B: Bus>(&mut self, bus: &mut B) -> Result<StepResult, CpuError> {
        let pending = bus.pending_interrupts() & 0x1F;

        if self.control.halted {
            if pending == 0 {
                return Ok(StepResult {
                    pc: self.registers.pc,
                    kind: StepKind::Halted,
                    cycles: 4,
                });
            }

            self.control.halted = false;
        }

        if self.control.ime && pending != 0 {
            return Ok(self.service_interrupt(bus, pending));
        }

        let start_pc = self.registers.pc;
        let decoded = match self.decode(bus) {
            Ok(decoded) => decoded,
            Err(e) => {
                self.registers.pc = start_pc;
                return Err(e);
            }
        };

        let enable_ime_after_instruction = self.control.ime_enable_delay == 1;

        let cycles = self.execute(bus, decoded.instruction);

        if enable_ime_after_instruction
            && !matches!(decoded.instruction, Instruction::DisableInterrupts)
        {
            self.control.ime = true;
        }

        if self.control.ime_enable_delay > 0 {
            self.control.ime_enable_delay -= 1;
        }

        Ok(StepResult {
            pc: decoded.pc,
            kind: StepKind::Instruction {
                opcode: decoded.opcode,
                instruction: decoded.instruction,
            },
            cycles,
        })
    }

    /// Inspect the CPU registers.
    pub fn registers(&self) -> &Registers {
        &self.registers
    }

    /// Inspect the CPU's status.
    pub fn status(&self) -> CpuStatus {
        CpuStatus {
            ime: self.control.ime,
            halted: self.control.halted,
            stopped: self.control.stopped,
            ime_enable_delay: self.control.ime_enable_delay,
        }
    }

    pub(crate) fn fetch_opcode<B: Bus>(&mut self, bus: &mut B) -> u8 {
        let opcode = bus.read8(self.registers.pc);
        if self.control.halt_bug {
            self.control.halt_bug = false;
        } else {
            self.registers.pc = self.registers.pc.wrapping_add(1);
        }
        opcode
    }

    pub(crate) fn fetch8<B: Bus>(&mut self, bus: &mut B) -> u8 {
        let value = bus.read8(self.registers.pc);
        self.registers.pc = self.registers.pc.wrapping_add(1);
        value
    }

    pub(crate) fn fetch16<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let low = self.fetch8(bus);
        let high = self.fetch8(bus);
        u16::from_le_bytes([low, high])
    }

    pub(crate) fn push<B: Bus>(&mut self, bus: &mut B, value: u16) {
        self.registers.sp = self.registers.sp.wrapping_sub(1);
        bus.write8(self.registers.sp, (value >> 8) as u8);
        self.registers.sp = self.registers.sp.wrapping_sub(1);
        bus.write8(self.registers.sp, value as u8);
    }

    pub(crate) fn pop<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let low = bus.read8(self.registers.sp);
        self.registers.sp = self.registers.sp.wrapping_add(1);
        let high = bus.read8(self.registers.sp);
        self.registers.sp = self.registers.sp.wrapping_add(1);
        u16::from_le_bytes([low, high])
    }

    fn service_interrupt<B: Bus>(&mut self, bus: &mut B, pending: u8) -> StepResult {
        // Lower interrupt bits have higher priority.
        let bit = pending.trailing_zeros() as u8;

        let mask = 1 << bit;
        let vector = 0x0040 + u16::from(bit) * 8;

        let pc = self.registers.pc;

        // Normally, an interrupt returns to the next instruction.
        // If EI; HALT triggered the HALT bug, the return address
        // is instead the HALT instruction itself.
        let return_pc = if self.control.halt_bug {
            pc.wrapping_sub(1)
        } else {
            pc
        };

        self.control.ime = false;
        self.control.ime_enable_delay = 0;
        self.control.halted = false;
        self.control.halt_bug = false;

        bus.acknowledge_interrupt(mask);

        self.push(bus, return_pc);
        self.registers.pc = vector;

        StepResult {
            pc,
            kind: StepKind::Interrupt { vector },
            cycles: 20,
        }
    }
}

/// An illegal or not-yet-supported SM83 opcode.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CpuError {
    /// The opcode exists, but this interpreter doesn't implement it yet.
    #[error("unimplemented SM83 opcode {opcode:X?} at ${pc:04X}")]
    Unimplemented {
        /// The address where the opcode began.
        pc: u16,
        /// The unimplemented opcode.
        opcode: Opcode,
    },
    /// One of the SM83's undefined opcodes.
    #[error("illegal SM83 opcode {opcode:02X} at ${pc:04X}")]
    Illegal {
        /// The address where the opcode began.
        pc: u16,
        /// Invalid opcode.
        opcode: u8,
    },
}

/// A snapshot of non-register CPU control state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuStatus {
    /// Interrupt master enable.
    pub ime: bool,
    /// Whether instruction executed is halted.
    pub halted: bool,
    /// Whether the processor is stopped.
    pub stopped: bool,
    /// Remaining instructions before IME changes.
    pub ime_enable_delay: u8,
}

/// Execution metadata returned after a CPU step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepResult {
    /// PC at the beginning of the step.
    pub pc: u16,
    /// What the CPU did during this step.
    pub kind: StepKind,
    /// CPU clock periods consumed, in T-cycles.
    pub cycles: u32,
}

/// The type of operation completed by a CPU step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// A decoded instruction was executed.
    Instruction {
        /// The fetched opcode.
        opcode: Opcode,
        /// The decoded instruction.
        instruction: Instruction,
    },
    /// An interrupt was serviced.
    Interrupt {
        /// Interrupt handler address.
        vector: u16,
    },
    /// The CPU spent four T-cycles halted.
    Halted,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ExecutionControl {
    pub ime: bool,
    pub halted: bool,
    pub stopped: bool,
    pub ime_enable_delay: u8,
    pub halt_bug: bool,
}
