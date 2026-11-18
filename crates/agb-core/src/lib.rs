//! Game Boy Advance machine emulation.
//!
//! This crate implements the machine-level integration of the
//! Game Boy Advance (AGB).
//!
//! Unlike the original Game Boy and Game Boy Color, the GBA
//! uses an ARM7TDMI processor and a substantially different
//! memory map, graphics system, and collection of peripherals.
//!
//! Consequently, GBA hardware is represented by a separate
//! machine implementation rather than extending the Game Boy
//! machine.
//!
//! # Architecture
//!
//! The [`Gba`] structure owns the state associated with a
//! single emulated Game Boy Advance instance.
//!
//! This currently includes:
//!
//! - A cartridge ROM image.
//! - An ARM7TDMI CPU instance.
//! - An optional BIOS image.
//! - A native-resolution RGB framebuffer.
//! - Logical button input state.
//! - An audio sample buffer.
//! - A diagnostic virtual clock.
//!
//! As development progresses, the machine will also manage
//! the GBA memory bus and hardware peripherals.
//!
//! The machine implements the shared [`Machine`] trait,
//! allowing frontends to interact with it through the
//! same interface used by the Game Boy machine.
//!
//! # CPU
//!
//! The Game Boy Advance uses an ARM7TDMI processor
//! implementing the ARMv4T architecture.
//!
//! The processor supports two instruction sets:
//!
//! - ARM, using 32-bit instruction encodings.
//! - Thumb, using 16-bit instruction encodings.
//!
//! Both instruction sets operate on the same underlying
//! processor state.
//!
//! The CPU is implemented separately in the `arm7tdmi`
//! crate and owned by the [`Gba`] machine.
//!
//! Memory access, interrupt delivery, and peripheral
//! scheduling will be integrated at the machine level.
//!
//! # Clock and video timing
//!
//! The GBA operates with a master clock frequency of
//! 16,777,216 Hz.
//!
//! The LCD timing system consists of 228 scanlines per
//! frame, each lasting 1,232 master clock cycles.
//!
//! Of these scanlines:
//!
//! - 160 are visible.
//! - 68 belong to vertical blanking.
//!
//! This gives 280,896 master clock cycles per frame,
//! corresponding to a nominal refresh rate of
//! approximately 59.73 Hz.
//!
//! The constants [`CLOCK_HZ`] and [`CLOCKS_PER_FRAME`]
//! describe this timing domain.
//!
//! Future implementations must account for instruction
//! execution timing, memory access costs, DMA transfers,
//! and peripheral scheduling.
//!
//! # Video
//!
//! The Game Boy Advance has a native display resolution
//! of 240 by 160 pixels.
//!
//! The framebuffer uses the platform-independent RGB
//! representation provided by `handheld-common`.
//!
//! The eventual graphics implementation will be responsible
//! for converting the GBA's native color representation
//! into framebuffer pixels.
//!
//! Frontends are responsible for scaling and presenting
//! the completed framebuffer.
//!
//! # BIOS
//!
//! The Game Boy Advance contains a 16 KiB system BIOS.
//!
//! The BIOS provides startup functionality, exception
//! handling, and software interrupt services used by
//! GBA software.
//!
//! The machine constructor accepts an optional BIOS
//! image supplied by the caller.
//!
//! The current implementation verifies only that the
//! supplied image contains exactly [`BIOS_SIZE`] bytes.
//!
//! It does not verify the BIOS contents, execute the BIOS,
//! or emulate its software interrupt services.
//!
//! Future implementations may support either execution
//! of a user-supplied BIOS or high-level emulation of
//! selected BIOS functionality.
//!
//! # Cartridge hardware
//!
//! Cartridge ROM loading and header inspection are handled
//! by the shared `handheld-common` crate.
//!
//! This crate will eventually implement GBA-specific
//! cartridge behavior, including save memory and optional
//! cartridge peripherals.
//!
//! The current implementation does not support cartridge
//! SRAM, Flash, EEPROM, RTC, or other cartridge hardware.
//!
//! # Limitations
//!
//! The current implementation provides machine
//! initialization and diagnostic infrastructure only.
//!
//! It does not yet implement:
//!
//! - ARM or Thumb instruction execution.
//! - CPU reset and exception handling.
//! - BIOS execution or software interrupt services.
//! - Memory mapping and bus access.
//! - Graphics rendering or LCD timing.
//! - Audio generation.
//! - Hardware timers.
//! - Interrupt delivery.
//! - DMA transfers.
//! - Cartridge save hardware.
//! - Hardware keypad register behavior.
//!
//! Advancing the diagnostic virtual clock does not
//! execute instructions or update hardware peripherals.
//!
//! The ARM7TDMI is initialized using its default placeholder
//! state rather than an accurate processor reset state.
//!
//! # References
//!
//! - [GBATEK](https://mgba-emu.github.io/gbatek/)
//! - [Tonc](https://gbadev.net/tonc/)
//! - [ARM Architecture Reference Manual](https://developer.arm.com/documentation/ddi0100/latest/)

#![warn(missing_docs)]

use arm7tdmi::Arm7tdmi;
use rookery_common::{
    AudioBuffer, Buttons, ClockError, Framebuffer, Machine, MachineKind, StereoSample,
    VirtualClock,
    rom::{Rom, RomHeader},
};
use thiserror::Error;
use tracing::info;

/// Native Game Boy Advance display width, in pixels.
pub const SCREEN_WIDTH: usize = 240;
/// Native Game Boy Advance display height, in pixels.
pub const SCREEN_HEIGHT: usize = 160;

/// Game Boy Advance master clock frequency, in hertz.
///
/// The GBA master clock operates at 16,777,216 Hz,
/// or approximately 16.78 MHz.
///
/// This frequency provides the base timing reference
/// for CPU execution and hardware scheduling.
///
/// Individual CPU instructions and memory accesses
/// may consume different numbers of clock cycles.
///
/// In particular, a memory access does not necessarily
/// complete in a single master clock cycle.
///
/// Future implementations must account for waitstates
/// and other hardware timing behavior.
pub const CLOCK_HZ: u32 = 16_777_216;
// Number of GBA master clock cycles in one video frame.
///
/// Each GBA video frame consists of 228 scanlines,
/// with 1,232 master clock cycles per scanline.
///
/// This gives:
///
/// `1232 * 228 = 280,896` master clock cycles per frame.
///
/// The scanlines are divided into:
///
/// - Lines `0..=159`: visible display.
/// - Lines `160..=227`: vertical blanking.
///
/// Together with [`CLOCK_HZ`], this corresponds to
/// a nominal refresh rate of approximately 59.73 Hz.
pub const CLOCKS_PER_FRAME: u64 = 1232 * 228;

/// Required size of a Game Boy Advance BIOS image, in bytes.
///
/// The GBA contains 16 KiB of internal system ROM,
/// corresponding to 16,384 bytes.
///
/// A BIOS image supplied to [`Gba::new`] must contain
/// exactly this number of bytes.
///
/// # Validation
///
/// The current implementation validates only the
/// length of the supplied image.
///
/// A correctly sized file is not necessarily a valid
/// or authentic GBA BIOS dump.
pub const BIOS_SIZE: usize = 16 * 1024;

/// Errors that may occur while constructing a GBA machine.
///
/// These errors describe cartridge platform mismatches
/// and invalid BIOS image lengths.
#[derive(Debug, Error)]
pub enum GbaError {
    /// The supplied ROM does not target GBA hardware.
    ///
    /// A [`Gba`] machine requires a cartridge containing
    /// [`RomHeader::Gba`] metadata.
    ///
    /// Original Game Boy and Game Boy Color cartridges
    /// must instead be handled by the Game Boy machine.
    ///
    /// The current GBA machine does not implement the
    /// hardware compatibility mode used by physical GBA
    /// systems to run older Game Boy cartridges.
    #[error("expected a Game Boy Advance cartridge")]
    WrongPlatform,

    /// A supplied BIOS image does not have the required size.
    ///
    /// The contained value is the actual number of bytes
    /// in the supplied image.
    ///
    /// A valid GBA BIOS image must contain exactly
    /// [`BIOS_SIZE`] bytes.
    ///
    /// This error does not indicate whether a correctly
    /// sized BIOS image contains valid firmware.
    #[error("invalid GBA BIOS size: expected 16384 bytes, got {0}")]
    InvalidBiosSize(usize),
}

/// An emulated Game Boy Advance machine.
///
/// This structure owns the state associated with a
/// single GBA instance.
///
/// It integrates the ARM7TDMI CPU and cartridge ROM
/// with the shared frontend-facing abstractions
/// defined by `handheld-common`.
///
/// # Ownership
///
/// The machine owns:
///
/// - Its cartridge ROM.
/// - Its optional BIOS image.
/// - Its ARM7TDMI CPU instance.
/// - Its framebuffer.
/// - Its logical input state.
/// - Its audio output buffer.
/// - Its virtual clock counter.
///
/// External callers cannot directly modify the CPU,
/// ROM, or BIOS through the current public interface.
///
/// # BIOS
///
/// A BIOS image may be provided during construction.
///
/// When present, its bytes are retained by the machine
/// for future use by the memory and execution subsystems.
///
/// The current implementation does not map or execute
/// the supplied BIOS.
///
/// The presence of BIOS data can be inspected using
/// [`Gba::bios_present`].
///
/// # Frontend integration
///
/// [`Gba`] implements the shared [`Machine`] trait.
///
/// This allows a frontend to:
///
/// - Identify the machine as GBA hardware.
/// - Read its framebuffer.
/// - Provide logical button input.
/// - Retrieve buffered audio samples.
/// - Inspect and advance its diagnostic virtual clock.
pub struct Gba {
    rom: Rom,
    bios: Option<Vec<u8>>,
    cpu: Arm7tdmi,

    framebuffer: Framebuffer,
    buttons: Buttons,
    audio: AudioBuffer,
    clock: VirtualClock,
}

impl Gba {
    /// Constructs a new Game Boy Advance machine.
    ///
    /// The constructor takes ownership of the supplied
    /// cartridge ROM and optional BIOS image.
    ///
    /// It validates that the cartridge targets GBA
    /// hardware and, if BIOS data is supplied, that
    /// its length matches [`BIOS_SIZE`].
    ///
    /// # Initialization
    ///
    /// On successful construction, the machine contains:
    ///
    /// - The supplied GBA cartridge ROM.
    /// - The optional BIOS image.
    /// - A default ARM7TDMI CPU instance.
    /// - A 240-by-160 RGB framebuffer initialized to black.
    /// - A logical button state with every button released.
    /// - An empty stereo audio buffer configured for 48 kHz.
    /// - A virtual clock initialized to zero.
    ///
    /// The constructor emits a structured diagnostic event
    /// through the `tracing` infrastructure.
    ///
    /// The event includes the presence of BIOS data
    /// and the size of the cartridge ROM.
    ///
    /// # Errors
    ///
    /// Returns [`GbaError::WrongPlatform`] if the cartridge
    /// does not contain a GBA header.
    ///
    /// Returns [`GbaError::InvalidBiosSize`] if a supplied
    /// BIOS image is not exactly 16 KiB long.
    ///
    /// # BIOS validation
    ///
    /// The constructor validates only BIOS image length.
    ///
    /// It does not verify the firmware's contents,
    /// authenticity, checksum, or expected instructions.
    ///
    /// A zero-filled buffer of the correct length is
    /// accepted, although it is not functional firmware.
    ///
    /// # Limitations
    ///
    /// This constructor does not:
    ///
    /// - Execute the GBA BIOS.
    /// - Initialize an architectural CPU reset state.
    /// - Map ROM or BIOS into the memory bus.
    /// - Initialize hardware registers.
    /// - Execute cartridge instructions.
    /// - Validate cartridge save hardware compatibility.
    ///
    /// A successfully initialized machine is not
    /// necessarily capable of running its cartridge.
    ///
    /// # Examples
    ///
    /// Construct a machine without BIOS data:
    ///
    /// ```no_run
    /// use agb_core::Gba;
    /// use rookery_common::rom::Rom;
    ///
    /// let rom = Rom::load("game.gba")?;
    ///
    /// let machine = Gba::new(rom, None)?;
    ///
    /// assert!(!machine.bios_present());
    ///
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// Construct a machine with a user-supplied BIOS:
    ///
    /// ```no_run
    /// use agb_core::Gba;
    /// use rookery_common::rom::Rom;
    ///
    /// let rom = Rom::load("game.gba")?;
    /// let bios = std::fs::read("gba_bios.bin")?;
    ///
    /// let machine = Gba::new(rom, Some(bios))?;
    ///
    /// assert!(machine.bios_present());
    ///
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(rom: Rom, bios: Option<Vec<u8>>) -> Result<Self, GbaError> {
        if !matches!(rom.header(), RomHeader::Agb(_)) {
            return Err(GbaError::WrongPlatform);
        }

        if let Some(ref bytes) = bios
            && bytes.len() != BIOS_SIZE
        {
            return Err(GbaError::InvalidBiosSize(bytes.len()));
        }

        info!(
            bios_present = bios.is_some(),
            rom_size = rom.len(),
            "initializing Game Boy Advance"
        );

        Ok(Self {
            rom,
            bios,
            cpu: Arm7tdmi::default(),

            framebuffer: Framebuffer::new(SCREEN_WIDTH, SCREEN_HEIGHT),
            buttons: Buttons::default(),
            audio: AudioBuffer::new(48_000).expect("Sample rate is non-zero"),
            clock: VirtualClock::default(),
        })
    }

    /// Returns an immutable reference to the loaded cartridge.
    ///
    /// The returned [`Rom`] provides access to the cartridge's
    /// raw bytes, parsed header, and detected hardware family.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use agb_core::Gba;
    /// use rookery_common::rom::Rom;
    ///
    /// let rom = Rom::load("game.gba")?;
    /// let machine = Gba::new(rom, None)?;
    ///
    /// println!(
    ///     "Cartridge size: {} bytes",
    ///     machine.rom().len()
    /// );
    ///
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn rom(&self) -> &Rom {
        &self.rom
    }

    /// Returns an immutable reference to the ARM7TDMI CPU.
    ///
    /// This accessor allows callers to inspect the
    /// processor's current state without providing
    /// mutable access to the CPU.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use agb_core::Gba;
    /// use rookery_common::rom::Rom;
    ///
    /// let rom = Rom::load("game.gba")?;
    /// let machine = Gba::new(rom, None)?;
    ///
    /// let registers = machine.cpu().registers();
    ///
    /// println!("PC: {:08X}", registers[15]);
    ///
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn cpu(&self) -> &Arm7tdmi {
        &self.cpu
    }

    /// Returns whether the machine contains BIOS data.
    ///
    /// This method checks only whether a BIOS image
    /// was supplied during construction.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use agb_core::Gba;
    /// use rookery_common::rom::Rom;
    ///
    /// let rom = Rom::load("game.gba")?;
    /// let machine = Gba::new(rom, None)?;
    ///
    /// assert!(!machine.bios_present());
    ///
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bios_present(&self) -> bool {
        self.bios.is_some()
    }
}

impl Machine for Gba {
    fn kind(&self) -> MachineKind {
        MachineKind::Agb
    }

    fn virtual_clock(&self) -> u64 {
        self.clock.cycles()
    }

    fn advance_clock_only(&mut self, cycles: u64) -> Result<(), ClockError> {
        self.clock.advance(cycles)
    }

    fn framebuffer(&self) -> &Framebuffer {
        &self.framebuffer
    }

    fn buttons(&self) -> Buttons {
        self.buttons
    }

    fn set_buttons(&mut self, buttons: Buttons) {
        self.buttons = buttons
    }

    fn drain_audio(&mut self) -> Vec<StereoSample> {
        self.audio.drain()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rookery_common::Platform;

    fn test_rom() -> Rom {
        let mut bytes = vec![0_u8; 0xC0];

        bytes[0xA0..0xA4].copy_from_slice(b"TEST");
        bytes[0xB2] = 0x96;

        Rom::from_bytes(bytes, Some(Platform::Agb)).unwrap()
    }

    #[test]
    fn initializes_gba() {
        let gba = Gba::new(test_rom(), None).unwrap();

        assert_eq!(gba.kind(), MachineKind::Agb);
        assert_eq!(gba.framebuffer().width(), 240);
        assert_eq!(gba.framebuffer().height(), 160);
        assert!(!gba.bios_present());
    }

    #[test]
    fn accepts_valid_bios_size() {
        let bios = vec![0; BIOS_SIZE];

        let gba = Gba::new(test_rom(), Some(bios)).unwrap();

        assert!(gba.bios_present());
    }

    #[test]
    fn rejects_invalid_bios_size() {
        let result = Gba::new(test_rom(), Some(vec![0; 100]));

        assert!(matches!(result, Err(GbaError::InvalidBiosSize(100))));
    }
}
