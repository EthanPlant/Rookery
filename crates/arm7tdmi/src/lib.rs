//! ARM7TDMI CPU emulation.
//!
//! This crate provides the CPU implementation intended for
//! the Game Boy Advance (AGB).
//!
//! The Game Boy Advance uses an ARM7TDMI processor implementing
//! the ARMv4T architecture.
//!
//! The processor supports two instruction sets:
//!
//! - ARM: 32-bit instruction encodings.
//! - Thumb: 16-bit instruction encodings.
//!
//! Both instruction sets operate on the same underlying
//! processor architecture and register state.
//!
//! # Architecture
//!
//! The ARM7TDMI is a 32-bit processor with sixteen general
//! register names, a current program status register, and
//! additional banked registers associated with processor modes.
//!
//! The processor supports multiple operating modes, including
//! privileged exception modes and unprivileged execution.
//!
//! Its architecture includes:
//!
//! - ARM and Thumb instruction decoding.
//! - Conditional ARM instruction execution.
//! - Arithmetic and logical operations.
//! - Register-based shifting.
//! - Load and store instructions.
//! - Branching and control flow.
//! - Banked registers.
//! - Processor status registers.
//! - Exceptions and interrupt handling.
//!
//! # Register model
//!
//! The architectural register names are `R0` through `R15`.
//!
//! Registers `R13`, `R14`, and `R15` have conventional
//! special roles:
//!
//! - `R13`: Stack pointer (`SP`).
//! - `R14`: Link register (`LR`).
//! - `R15`: Program counter (`PC`).
//!
//! The processor also maintains a Current Program Status
//! Register (`CPSR`) containing condition flags, execution
//! state, interrupt masks, and processor mode information.
//!
//! Some processor modes use banked registers and saved
//! program status registers.
//!
//! # Execution
//!
//! The eventual CPU interpreter will execute ARM and Thumb
//! instructions using a shared processor state.
//!
//! The owning Game Boy Advance machine will provide memory
//! access and integrate the CPU with the hardware scheduler,
//! interrupt controller, DMA, and other peripherals.
//!
//! The CPU crate will not directly own cartridge ROM,
//! system memory, graphics, or audio hardware.
//!
//! # Current limitations
//!
//! The current implementation is an architectural skeleton.
//!
//! It does not implement instruction execution, processor
//! modes, register banking, exceptions, or pipeline behaviour.
//!
//! Although it stores a CPSR value and a Thumb-state flag,
//! these fields are not yet maintained according to ARMv4T
//! architectural rules.
//!
//! All stored values are initialized to zero or false.
//!
//! This does not represent a valid ARM7TDMI reset state
//! or a correctly initialized processor execution context.
//!
//! # References
//!
//! - [GBATEK](https://mgba-emu.github.io/gbatek/)
//! - [ARM Architecture Reference Manual](https://developer.arm.com/documentation/ddi0100/latest/)

#![warn(missing_docs)]

// Emulated ARM7TDMI processor.
///
/// This structure represents the CPU used by the
/// Game Boy Advance.
///
/// It will eventually contain the architectural and
/// internal execution state required to emulate both
/// ARM and Thumb instructions.
///
/// # Register storage
///
/// The current implementation stores sixteen 32-bit
/// register values corresponding to the architectural
/// register names `R0` through `R15`.
///
/// This is only a partial representation of the
/// processor's complete register state.
///
/// The ARM7TDMI includes banked registers that are
/// selected according to the current processor mode.
///
/// Register banking is not yet implemented.
///
/// # Processor status
///
/// The processor stores a 32-bit CPSR value and a
/// separate boolean indicating Thumb execution state.
///
/// In a complete implementation, ARM/Thumb state is
/// represented by the T bit of CPSR.
///
/// The current boolean is provisional and is not
/// synchronized with the corresponding CPSR bit.
///
/// Future implementations must ensure that architectural
/// state transitions update these values consistently.
///
/// # Execution
///
/// No instructions are currently executed.
///
/// The processor does not perform instruction fetching,
/// memory access, exception handling, or pipeline updates.
///
/// # Initialization
///
/// The derived [`Default`] implementation initializes
/// every register and the CPSR to zero, and the Thumb
/// execution flag to false.
///
/// This is not a valid architectural reset state.
///
/// In particular, a zero CPSR does not encode a valid
/// ARM processor mode.
///
/// Actual reset and startup behaviour will be implemented
/// when the ARM7TDMI interpreter is introduced.
///
/// # Examples
///
/// ```
/// use arm7tdmi::Arm7tdmi;
///
/// let cpu = Arm7tdmi::new();
///
/// assert_eq!(cpu.registers().len(), 16);
/// assert_eq!(cpu.cpsr(), 0);
/// assert!(!cpu.is_thumb());
/// ```
#[derive(Debug, Default)]
pub struct Arm7tdmi {
    registers: [u32; 16],
    cpsr: u32,
    thumb: bool,
}

impl Arm7tdmi {
    /// Constructs a new ARM7TDMI processor.
    ///
    /// The processor is initialized using its derived
    /// [`Default`] implementation.
    ///
    /// All sixteen stored registers and CPSR are set to
    /// zero, and the Thumb-state flag is set to false.
    ///
    /// No BIOS execution, exception initialization,
    /// processor mode selection, or pipeline setup
    /// is performed.
    ///
    /// # Examples
    ///
    /// ```
    /// use arm7tdmi::Arm7tdmi;
    ///
    /// let cpu = Arm7tdmi::new();
    ///
    /// assert_eq!(cpu.registers()[0], 0);
    /// assert_eq!(cpu.cpsr(), 0);
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the processor's currently stored registers.
    ///
    /// The returned array contains sixteen 32-bit values,
    /// indexed according to architectural register number.
    ///
    /// | Index | Conventional name |
    /// |-------|-------------------|
    /// | `0..=12` | `R0` through `R12` |
    /// | `13` | Stack pointer (`SP`) |
    /// | `14` | Link register (`LR`) |
    /// | `15` | Program counter (`PC`) |
    ///
    /// # Register banking
    ///
    /// The ARM7TDMI uses different physical registers
    /// for certain architectural register names depending
    /// on the active processor mode.
    ///
    /// This behaviour is not implemented currently.
    ///
    /// The returned array therefore represents only
    /// the current placeholder register storage.
    ///
    /// It must not be interpreted as a complete snapshot
    /// of every physical register in the processor.
    ///
    /// # Program counter
    ///
    /// During ARM and Thumb execution, reads of `R15`
    /// have architecture-specific pipeline semantics.
    ///
    /// The current implementation does not model those
    /// semantics or maintain a running program counter.
    ///
    /// # Examples
    ///
    /// ```
    /// use arm7tdmi::Arm7tdmi;
    ///
    /// let cpu = Arm7tdmi::new();
    ///
    /// let registers = cpu.registers();
    ///
    /// let sp = registers[13];
    /// let lr = registers[14];
    /// let pc = registers[15];
    ///
    /// assert_eq!((sp, lr, pc), (0, 0, 0));
    /// ```
    pub fn registers(&self) -> &[u32; 16] {
        &self.registers
    }

    /// Returns the stored Current Program Status Register.
    ///
    /// The CPSR is a 32-bit register that controls
    /// processor execution state.
    ///
    /// Architecturally significant fields include:
    ///
    /// | Bit(s) | Field | Meaning |
    /// |--------|-------|---------|
    /// | 31 | N | Negative condition flag |
    /// | 30 | Z | Zero condition flag |
    /// | 29 | C | Carry condition flag |
    /// | 28 | V | Overflow condition flag |
    /// | 7 | I | IRQ disable |
    /// | 6 | F | FIQ disable |
    /// | 5 | T | Thumb execution state |
    /// | 4–0 | M | Processor mode |
    ///
    /// Other bit positions are reserved in ARMv4T.
    ///
    /// # Limitations
    ///
    /// The current implementation stores CPSR without
    /// interpreting or validating its contents.
    ///
    /// Its initial value is zero, which does not encode
    /// a valid processor mode.
    ///
    /// The separate Thumb-state flag is not synchronized
    /// with the T bit of CPSR.
    ///
    /// These architectural invariants will be enforced
    /// when CPU execution and mode handling are implemented.
    ///
    /// # Examples
    ///
    /// ```
    /// use arm7tdmi::Arm7tdmi;
    ///
    /// let cpu = Arm7tdmi::new();
    ///
    /// assert_eq!(cpu.cpsr(), 0);
    /// ```
    pub fn cpsr(&self) -> u32 {
        self.cpsr
    }

    // Returns the processor's stored Thumb-state flag.
    ///
    /// Thumb is the ARM7TDMI's 16-bit instruction set.
    ///
    /// In a complete implementation, this execution state
    /// is controlled by bit 5 (the T bit) of CPSR.
    ///
    /// A value of `true` indicates the stored flag selects
    /// Thumb execution, while `false` indicates ARM.
    ///
    /// # Limitations
    ///
    /// This method currently returns a standalone boolean
    /// initialized to false.
    ///
    /// It does not decode the CPSR T bit, and no instruction
    /// execution or ARM/Thumb state transitions occur.
    ///
    /// It should therefore be treated as a placeholder
    /// accessor rather than evidence of a valid processor
    /// execution state.
    ///
    /// # Returns
    ///
    /// `true` if the stored Thumb-state flag is set;
    /// otherwise, `false`.
    ///
    /// # Examples
    ///
    /// ```
    /// use arm7tdmi::Arm7tdmi;
    ///
    /// let cpu = Arm7tdmi::new();
    ///
    /// assert!(!cpu.is_thumb());
    /// ```
    pub fn is_thumb(&self) -> bool {
        self.thumb
    }
}
