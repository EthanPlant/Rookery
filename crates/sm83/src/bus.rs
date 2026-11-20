//! The CPU-facing byte-addressable memory interface.

/// Read and write the SM83's 16-bit address space.
///
/// Reads require `&mut self` because hardware registers can have read effects.
pub trait Bus {
    /// Read a byte through the CPU bus.
    fn read8(&mut self, address: u16) -> u8;

    /// Write a byte through the CPU bus.
    fn write8(&mut self, address: u16, value: u8);

    /// Returns enabled and requested interrupts.
    fn pending_interrupts(&self) -> u8;

    /// Acknowledges an interrupt by clearing its IF bit.
    fn acknowledge_interrupt(&mut self, mask: u8);
}
