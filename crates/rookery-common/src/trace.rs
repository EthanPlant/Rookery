//! Instruction tracing interfaces and diagnostic output.
//!
//! This module provides platform-independent infrastructure for
//! recording and formatting instructions executed by the emulation
//! cores.
//!
//! Tracing is intended primarily for debugging, instruction-level
//! testing, and comparison against reference emulator output.
//!
//! # Architecture
//!
//! Instruction tracing separates the production of trace records
//! from their presentation and storage.
//!
//! An emulation core is responsible for collecting information
//! about executed instructions and constructing an
//! [`InstructionRecord`].
//!
//! A [`TraceSink`] receives those records and determines how
//! they should be handled.
//!
//! The provided [`TextTraceSink`] implementation writes
//! human-readable instruction records to an arbitrary
//! [`std::io::Write`] destination.
//!
//! This separation allows additional tracing implementations
//! to be introduced without modifying CPU implementations.
//!
//! Potential future sinks include:
//!
//! - Binary trace files.
//! - Structured JSON output.
//! - In-memory trace buffers.
//! - Instruction comparison and verification tools.
//! - Debugger interfaces.
//!
//! # Instruction representation
//!
//! Trace records use a common representation for SM83,
//! ARM, and Thumb instructions.
//!
//! The instruction set is identified explicitly using
//! [`InstructionSet`].
//!
//! Program counter values and raw opcodes are represented
//! using `u32`, accommodating the address and instruction
//! widths of all currently planned architectures.
//!
//! These values are not normalized or validated by the
//! tracing infrastructure.
//!
//! # Timing
//!
//! Each instruction record includes a cycle count supplied
//! by the emulation core.
//!
//! The interpretation of this count depends on the instruction
//! set and CPU implementation. In particular, CPU clock cycles
//! should not be assumed to be interchangeable with other
//! hardware clock domains.
//!
//! The tracing infrastructure does not advance virtual time
//! or calculate instruction timings.

use std::io::{self, Write};

/// Identifies the instruction set associated with a trace record.
///
/// The emulator supports three instruction sets across its
/// planned hardware platforms:
///
/// - SM83 instructions for Game Boy and Game Boy Color.
/// - ARM instructions for Game Boy Advance.
/// - Thumb instructions for Game Boy Advance.
///
/// ARM and Thumb are execution states of the same ARM7TDMI
/// processor, rather than separate emulated CPUs.
///
/// This enum identifies the instruction encoding used for
/// a particular trace record. It does not describe the
/// machine's complete hardware configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstructionSet {
    // The SM83 instruction set used by Game Boy hardware.
    ///
    /// SM83 instructions use an 8-bit opcode encoding,
    /// with additional instruction bytes and a `0xCB`
    /// prefix for extended operations.
    ///
    /// The emulation core is responsible for representing
    /// prefixed instructions consistently in trace records.
    Sm83,
    /// The 32-bit ARM instruction set used by ARM7TDMI.
    ///
    /// ARM instructions are encoded as 32-bit words.
    ///
    /// The Game Boy Advance CPU can switch between ARM
    /// and Thumb execution states.
    Arm,
    /// The 16-bit Thumb instruction set used by ARM7TDMI.
    ///
    /// Thumb instructions are encoded as 16-bit halfwords.
    ///
    /// Thumb uses the same underlying processor registers
    /// and execution state as ARM, with a different
    /// instruction encoding.
    Thumb,
}

/// Describes a single instruction for diagnostic tracing.
///
/// An instruction record contains identifying information,
/// execution timing, and a human-readable disassembly
/// supplied by the emulation core.
///
/// Records are intended to describe individual instructions,
/// rather than complete CPU state snapshots.
///
/// # Ownership
///
/// The disassembly string is borrowed for lifetime `'a`.
///
/// This allows a CPU implementation to provide a reference
/// to an existing string without requiring an allocation
/// for every trace record.
///
/// Trace sinks that need to retain a record beyond the call
/// to [`TraceSink::record`] must copy any borrowed data.
///
/// # Ordering
///
/// Each record contains an instruction index provided by
/// the caller.
///
/// The tracing infrastructure does not assign indices,
/// enforce sequential numbering, or guarantee chronological
/// ordering.
///
/// Those responsibilities belong to the trace producer.
///
/// # Timing
///
/// The cycle count describes the instruction's execution
/// cost as reported by the CPU implementation.
///
/// The tracing infrastructure does not verify this value
/// against an instruction timing table.
///
/// # Examples
///
/// ```
/// use rookery_common::trace::{
///     InstructionRecord,
///     InstructionSet,
/// };
///
/// let instruction = InstructionRecord {
///     index: 0,
///     isa: InstructionSet::Sm83,
///     pc: 0x0100,
///     opcode: 0x00,
///     cycles: 4,
///     disassembly: "NOP",
/// };
///
/// assert_eq!(instruction.pc, 0x0100);
/// assert_eq!(instruction.cycles, 4);
/// ```
#[derive(Debug, Clone)]
pub struct InstructionRecord<'a> {
    /// Sequential identifier assigned to this instruction.
    ///
    /// Typically represents the number of instructions
    /// recorded since tracing began, starting at zero.
    ///
    /// The index is supplied by the caller and is not
    /// validated or incremented by the trace sink.
    pub index: u64,
    /// Instruction set used to interpret this instruction.
    ///
    /// This determines whether the opcode represents an
    /// SM83, ARM, or Thumb instruction.
    ///
    /// The instruction set is recorded explicitly so that
    /// traces from multiple CPU architectures can share
    /// the same output infrastructure.
    pub isa: InstructionSet,
    /// Program counter associated with this instruction.
    ///
    /// The intended convention is the address of the
    /// instruction being executed, before the instruction
    /// modifies the program counter.
    ///
    /// The value is represented as `u32` to accommodate
    /// the GBA's 32-bit address space.
    ///
    /// SM83 implementations should store their 16-bit
    /// program counter using zero extension.
    ///
    /// ARM implementations must account for architectural
    /// program counter visibility and pipeline behaviour
    /// when constructing this value.
    ///
    /// This convention is not enforced by the trace sink.
    pub pc: u32,
    /// Raw instruction opcode supplied by the CPU.
    ///
    /// The value is stored as `u32` to accommodate ARM's
    /// 32-bit instruction encoding.
    ///
    /// Smaller instruction encodings are represented
    /// using the lower bits of the value.
    ///
    /// The current trace interface does not define a
    /// canonical representation for multi-byte SM83
    /// instructions or `0xCB`-prefixed opcodes.
    ///
    /// That convention must be established when the SM83
    /// instruction decoder is implemented.
    ///
    /// Operand bytes are not necessarily included in
    /// this field.
    pub opcode: u32,
    /// Number of clock cycles consumed by this instruction.
    ///
    /// The value is supplied by the CPU implementation
    /// after determining the instruction's execution cost.
    ///
    /// For instructions with conditional timing, this
    /// should represent the number of cycles actually
    /// consumed by the executed path.
    ///
    /// The unit is CPU-implementation-specific and must
    /// be interpreted in the context of the instruction
    /// set and machine clock configuration.
    ///
    /// This field does not necessarily represent elapsed
    /// display, audio, or system master-clock cycles.
    pub cycles: u32,
    /// Human-readable disassembly of the instruction.
    ///
    /// The string is produced by the emulation core or
    /// its instruction decoder.
    ///
    /// It may include the instruction mnemonic, operands,
    /// immediate values, or other useful diagnostic text.
    ///
    /// The tracing infrastructure does not parse, validate,
    /// or generate disassembly strings.
    ///
    /// The string is borrowed for lifetime `'a` and must
    /// remain valid while a trace sink processes the record.
    pub disassembly: &'a str,
}

/// Receives instruction records produced by an emulation core.
///
/// A trace sink defines how instruction records are processed,
/// allowing trace generation to remain independent of output
/// formatting and storage.
///
/// Implementations may write records to files, print them
/// to a terminal, retain them in memory, or perform
/// diagnostic comparisons.
///
/// # Record lifetime
///
/// [`InstructionRecord`] contains borrowed data.
///
/// Implementations that need to retain information after
/// [`TraceSink::record`] returns must copy or otherwise
/// take ownership of the necessary fields.
///
/// # Error handling
///
/// Trace processing may fail, particularly when writing
/// records to external destinations.
///
/// Errors are returned as [`io::Error`] values and should
/// be handled by the caller.
///
/// The trait does not prescribe whether a tracing failure
/// should terminate emulation or merely disable tracing.
///
/// # Synchronization
///
/// This trait does not provide thread synchronization.
///
/// Callers are responsible for coordinating access when
/// a trace sink is shared between execution contexts.
pub trait TraceSink {
    /// Processes a single instruction trace record.
    ///
    /// The record is passed by immutable reference and
    /// remains owned by the caller.
    ///
    /// Implementations may format, write, compare, or copy
    /// information from the record.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if the sink cannot process
    /// the record successfully.
    ///
    /// For output-backed implementations, this typically
    /// indicates a write failure.
    ///
    /// The caller is responsible for determining how
    /// tracing errors affect continued emulation.
    fn record(&mut self, instruction: &InstructionRecord<'_>) -> io::Result<()>;
}

/// Writes instruction records as human-readable text.
///
/// Each record is formatted as a single line containing:
///
/// - Instruction index.
/// - Instruction set.
/// - Program counter.
/// - Raw opcode.
/// - Cycle count.
/// - Disassembly.
///
/// The output destination is generic over [`Write`],
/// allowing traces to be written to standard output,
/// files, memory buffers, or other writable streams.
///
/// # Output format
///
/// The current format is:
///
/// ```text
/// #00000000 Sm83 PC=0x00000100 OP=0x00000000 CY= 4 NOP
/// ```
///
/// Numeric values are displayed using fixed-width
/// hexadecimal or decimal formatting.
///
/// Program counter and opcode fields are rendered as
/// eight-digit hexadecimal values regardless of the
/// underlying instruction set.
///
/// This format is intended for human-readable diagnostics.
/// It is not a stable machine-readable serialization format.
///
/// # Buffering
///
/// The sink does not implement its own buffering.
///
/// Buffering behaviour is determined by the underlying
/// writer supplied during construction.
///
/// For example, wrapping a file in [`std::io::BufWriter`]
/// can reduce the number of filesystem write operations.
///
/// # Examples
///
/// ```
/// use rookery_common::trace::{
///     InstructionRecord,
///     InstructionSet,
///     TextTraceSink,
///     TraceSink,
/// };
///
/// let mut sink = TextTraceSink::new(Vec::new());
///
/// sink.record(&InstructionRecord {
///     index: 0,
///     isa: InstructionSet::Sm83,
///     pc: 0x0100,
///     opcode: 0x00,
///     cycles: 4,
///     disassembly: "NOP",
/// }).unwrap();
///
/// let bytes = sink.into_inner();
/// let output = String::from_utf8(bytes).unwrap();
///
/// assert!(output.contains("NOP"));
/// assert!(output.contains("PC=0x00000100"));
/// ```
pub struct TextTraceSink<W: Write> {
    writer: W,
}

impl<W: Write> TextTraceSink<W> {
    /// Creates a text trace sink using the supplied writer.
    ///
    /// The sink takes ownership of the writer and uses
    /// it for all subsequent trace output.
    ///
    /// No trace data is written during construction.
    ///
    /// # Examples
    ///
    /// ```
    /// use rookery_common::trace::TextTraceSink;
    ///
    /// let sink = TextTraceSink::new(Vec::<u8>::new());
    /// ```
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    /// Returns ownership of the underlying writer.
    ///
    /// This consumes the trace sink and returns the
    /// writer originally supplied during construction.
    ///
    /// It is useful for retrieving accumulated output
    /// from an in-memory buffer or accessing an owned
    /// output destination after tracing is complete.
    ///
    /// This method does not explicitly flush the writer
    /// before returning it.
    ///
    /// Callers using buffered writers are responsible
    /// for handling any required flushing or finalization.
    ///
    /// # Examples
    ///
    /// ```
    /// use rookery_common::trace::TextTraceSink;
    ///
    /// let sink = TextTraceSink::new(Vec::<u8>::new());
    ///
    /// let bytes = sink.into_inner();
    ///
    /// assert!(bytes.is_empty());
    /// ```
    pub fn into_inner(self) -> W {
        self.writer
    }
}

impl<W: Write> TraceSink for TextTraceSink<W> {
    fn record(&mut self, instruction: &InstructionRecord<'_>) -> io::Result<()> {
        writeln!(
            self.writer,
            "#{:08} {:?} PC={:#010X} OP={:#010X} CY={:>2} {}",
            instruction.index,
            instruction.isa,
            instruction.pc,
            instruction.opcode,
            instruction.cycles,
            instruction.disassembly
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_instruction_trace() {
        let mut tracer = TextTraceSink::new(Vec::new());

        tracer
            .record(&InstructionRecord {
                index: 0,
                isa: InstructionSet::Sm83,
                pc: 0x100,
                opcode: 0x00,
                cycles: 4,
                disassembly: "NOP",
            })
            .unwrap();

        let output = String::from_utf8(tracer.into_inner()).unwrap();

        assert!(output.contains("PC=0x00000100"));
        assert!(output.contains("NOP"));
        assert!(output.contains("CY= 4"));
    }
}
