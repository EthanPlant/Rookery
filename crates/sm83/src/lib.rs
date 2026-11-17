//! Sharp SM83 CPU emulation.
//!
//! This crate provides the CPU implementation used by the
//! original Game Boy (DMG) and Game Boy Color (CGB).
//!
//! The SM83 is an 8-bit processor with an instruction set
//! related to, but distinct from, the Intel 8080 and Zilog Z80
//! families.
//!
//! It uses 8-bit general-purpose registers, a 16-bit address
//! space, and a combination of single-byte and multi-byte
//! instructions.
//!
//! # Architecture
//!
//! The CPU implementation is intended to remain independent
//! of the Game Boy's memory bus and hardware peripherals.
//!
//! The CPU owns its architectural register state, while the
//! emulated machine is responsible for supplying memory access,
//! interrupt signals, and hardware scheduling.
//!
//! This separation allows the CPU to be tested independently
//! of the complete Game Boy machine.
//!
//! # Registers
//!
//! The SM83 exposes eight 8-bit registers:
//!
//! - `A`: Accumulator.
//! - `F`: Flags.
//! - `B`, `C`: General-purpose register pair.
//! - `D`, `E`: General-purpose register pair.
//! - `H`, `L`: General-purpose register pair.
//!
//! The processor also maintains two 16-bit registers:
//!
//! - `SP`: Stack pointer.
//! - `PC`: Program counter.
//!
//! Certain 8-bit registers may be combined into 16-bit
//! register pairs, commonly designated `AF`, `BC`, `DE`,
//! and `HL`.
//!
//! # Execution
//!
//! The eventual instruction interpreter will support:
//!
//! - Instruction decoding and execution.
//! - Arithmetic and logical operations.
//! - CPU flag management.
//! - Conditional branches and control flow.
//! - Stack operations.
//! - Interrupt handling.
//! - HALT and STOP behaviour.
//! - Instruction timing.
//!
//! Instruction execution will operate through a memory bus
//! supplied by the owning machine.
//!
//! The CPU crate will not directly own cartridge ROM, video
//! memory, or hardware peripheral implementations.
//!
//! The current implementation contains only the foundational
//! register structures and CPU ownership interface.
//!
//! No instructions are decoded or executed.
//!
//! The processor does not yet maintain interrupt state,
//! implement power-on or post-boot initialization, or enforce
//! architectural register invariants.
//!
//! All registers are initialized to zero by default.
//! This is an implementation placeholder and must not be
//! interpreted as an accurate hardware startup state.
#![warn(missing_docs)]

/// Architectural register state of the SM83 processor.
///
/// This structure contains the CPU's eight 8-bit registers
/// and two 16-bit registers.
///
/// General-purpose registers may be combined into 16-bit
/// register pairs:
///
/// | Pair | High byte | Low byte |
/// |------|-----------|----------|
/// | `AF` | `A` | `F` |
/// | `BC` | `B` | `C` |
/// | `DE` | `D` | `E` |
/// | `HL` | `H` | `L` |
///
/// The upper four bits of `F` represent the processor's
/// arithmetic flags.
///
/// # Register invariants
///
/// On SM83 hardware, the lower four bits of the `F`
/// register are always zero.
///
/// # Initialization
///
/// The derived [`Default`] implementation initializes
/// every register to zero.
///
/// This does not represent the documented post-boot
/// register state of any particular Game Boy model.
///
/// Model-specific initialization will be implemented
/// separately by the CPU or owning machine.
///
/// # Examples
///
/// ```
/// use sm83::Registers;
///
/// let registers = Registers::default();
///
/// assert_eq!(registers.a, 0);
/// assert_eq!(registers.pc, 0);
/// assert_eq!(registers.sp, 0);
/// ```
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

/// Emulated Sharp SM83 processor.
///
/// This structure owns the CPU's register state and will
/// eventually implement the SM83 instruction interpreter.
///
/// The processor is intended to operate within either a
/// DMG or CGB machine.
///
/// Hardware-specific components, including cartridge
/// controllers, memory mapping, timers, graphics, and
/// audio, remain outside this structure.
///
/// # State ownership
///
/// Register state is stored privately and exposed through
/// the immutable [`Sm83::registers`] accessor.
///
/// This prevents external callers from directly modifying
/// the CPU's internal register state through an existing
/// processor instance.
///
/// Future execution and reset interfaces will be responsible
/// for controlled state transitions.
///
/// # Initialization
///
/// The default constructor initializes registers to zero.
///
/// This is a development placeholder rather than a valid
/// hardware boot or post-boot state.
///
/// # Examples
///
/// ```
/// use sm83::Sm83;
///
/// let cpu = Sm83::new();
///
/// assert_eq!(cpu.registers().pc, 0);
/// assert_eq!(cpu.registers().sp, 0);
/// ```
#[derive(Debug, Default)]
pub struct Sm83 {
    registers: Registers,
}

impl Sm83 {
    /// Constructs a new SM83 processor.
    ///
    /// The processor is initialized using its derived
    /// [`Default`] implementation.
    ///
    /// All architectural registers are initially zero.
    ///
    /// No boot ROM execution, hardware initialization,
    /// or post-boot register setup is performed.
    ///
    /// # Returns
    ///
    /// A new SM83 instance containing default register
    /// state.
    ///
    /// # Examples
    ///
    /// ```
    /// use sm83::Sm83;
    ///
    /// let cpu = Sm83::new();
    ///
    /// assert_eq!(cpu.registers().a, 0);
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns an immutable reference to the CPU registers.
    ///
    /// The returned reference provides access to the
    /// processor's current architectural register values.
    ///
    /// Callers may inspect the registers but cannot
    /// modify them through this accessor.
    ///
    /// The reference remains valid for the duration
    /// of its borrow.
    ///
    /// # Returns
    ///
    /// An immutable reference to the processor's
    /// [`Registers`] structure.
    ///
    /// # Examples
    ///
    /// ```
    /// use sm83::Sm83;
    ///
    /// let cpu = Sm83::new();
    /// let registers = cpu.registers();
    ///
    /// println!("PC: {:04X}", registers.pc);
    /// println!("SP: {:04X}", registers.sp);
    /// ```
    pub fn registers(&self) -> &Registers {
        &self.registers
    }
}
