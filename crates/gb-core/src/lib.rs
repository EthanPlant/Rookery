//! Game Boy and Game Boy Color machine emulation.
//!
//! This crate implements the machine-level integration of the
//! original Game Boy (DMG) and Game Boy Color (CGB).
//!
//! Both hardware models share the same underlying machine
//! implementation, with model-specific behavior selected
//! through [`GameBoyModel`].
//!
//! # Architecture
//!
//! The [`GameBoy`] structure owns the components required to
//! represent an emulated Game Boy machine.
//!
//! These currently include:
//!
//! - A cartridge ROM image.
//! - An SM83 CPU instance.
//! - A native-resolution RGB framebuffer.
//! - Logical button input state.
//! - An audio sample buffer.
//! - A diagnostic virtual clock.
//!
//! As development progresses, the machine will also manage
//! the memory bus and Game Boy hardware peripherals.
//!
//! The machine implements the shared [`Machine`] trait,
//! allowing frontends to interact with it without depending
//! on Game Boy-specific implementation details.
//!
//! # Hardware models
//!
//! The crate supports two hardware configurations:
//!
//! | Model | Description |
//! |-------|-------------|
//! | [`GameBoyModel::Dmg`] | Original monochrome Game Boy |
//! | [`GameBoyModel::Cgb`] | Game Boy Color |
//!
//! The Game Boy Color extends the original Game Boy hardware
//! with additional graphics capabilities, memory banking,
//! and optional double-speed CPU execution.
//!
//! Compatible cartridges may run on either model.
//!
//! Cartridges explicitly marked as CGB-only cannot be
//! initialized on a DMG machine.
//!
//! The hardware model is selected during machine
//! construction and remains fixed for its lifetime.
//!
//! # Clock domains
//!
//! The Game Boy LCD operates using a base clock frequency
//! of 4,194,304 Hz.
//!
//! Both DMG and CGB hardware use this base frequency for
//! LCD timing.
//!
//! The CGB additionally supports double-speed CPU operation,
//! in which CPU-related timing changes without doubling
//! the LCD refresh rate.
//!
//! Future implementations must distinguish CPU clock cycles
//! from LCD or system timing where necessary.
//!
//! The constants [`CLOCK_HZ`] and [`CLOCKS_PER_FRAME`]
//! describe the base LCD clock domain.
//!
//! # Video
//!
//! Both hardware models have a native display resolution
//! of 160 by 144 pixels.
//!
//! The framebuffer uses the platform-independent RGB
//! representation provided by `handheld-common`.
//!
//! Hardware-specific color conversion and pixel rendering
//! will be implemented by the graphics subsystem.
//!
//! # CPU
//!
//! Both DMG and CGB machines use the Sharp SM83 CPU.
//!
//! The processor is implemented separately in the `sm83`
//! crate and owned by the [`GameBoy`] machine.
//!
//! The CPU does not directly own cartridge ROM, video
//! memory, or hardware peripheral implementations.
//!
//! # M0 limitations
//!
//! The current implementation provides machine
//! initialization and diagnostic infrastructure only.
//!
//! It does not yet implement:
//!
//! - CPU instruction execution.
//! - Memory mapping or cartridge bank switching.
//! - Graphics rendering or LCD timing.
//! - Audio generation.
//! - Hardware timer operation.
//! - Interrupt delivery.
//! - DMA transfers.
//! - Hardware joypad register behavior.
//!
//! Advancing the virtual clock does not execute any
//! hardware components.
//!
//! The CPU is initialized using its default placeholder
//! state rather than a hardware-accurate boot or post-boot
//! register configuration.
//!
//! # References
//!
//! - [Pan Docs](https://gbdev.io/pandocs/)
//! - [Game Boy CPU Opcodes](https://gbdev.io/gb-opcodes/optables/)

#![warn(missing_docs)]

/// Game Boy memory bus and address mapping.
mod bus;
mod timer;

use rookery_common::{
    AudioBuffer, Buttons, ClockError, Framebuffer, Machine, MachineKind, StereoSample,
    VirtualClock,
    rom::{CgbSupport, Rom, RomHeader},
};
use sm83::{CpuError, Registers, Sm83, StepResult};
use thiserror::Error;
use tracing::warn;

use crate::bus::GbBus;

/// Native Game Boy display width, in pixels.
pub const SCREEN_WIDTH: usize = 160;
/// Native Game Boy display height, in pixels.
pub const SCREEN_HEIGHT: usize = 144;

/// Base Game Boy LCD clock frequency, in hertz.
///
/// The LCD timing system operates at 4,194,304 clock
/// ticks per second.
///
/// On the original DMG, this also corresponds to the
/// normal CPU clock frequency.
///
/// The Game Boy Color supports a double-speed mode in
/// which CPU-related timing operates at twice the
/// normal rate while LCD timing remains based on
/// this frequency.
///
/// Consequently, this constant should not be treated
/// as the effective CPU frequency in all CGB configurations.
///
/// Future scheduling implementations must distinguish
/// the relevant hardware clock domains.
pub const CLOCK_HZ: u32 = 4_194_304;
/// Number of base LCD clock ticks in one Game Boy frame.
///
/// Game Boy LCD timing consists of 154 scanlines,
/// each lasting 456 base clock ticks.
///
/// Of these:
///
/// - 144 scanlines are visible.
/// - 10 scanlines belong to vertical blanking.
///
/// This gives:
///
/// `456 * 154 = 70,224` clock ticks per frame.
///
/// Together with [`CLOCK_HZ`], this corresponds to
/// a nominal refresh rate of approximately 59.73 Hz.
///
/// This constant describes LCD timing and remains
/// unchanged during CGB double-speed CPU execution.
pub const CLOCKS_PER_FRAME: u64 = 456 * 154;

/// Selects the Game Boy hardware model to emulate.
///
/// The selected model determines which hardware
/// capabilities and compatibility rules apply.
///
/// DMG and CGB share the same CPU architecture and
/// much of the same hardware behavior, but the CGB
/// provides additional capabilities.
///
/// The model is specified when constructing a
/// [`GameBoy`] instance and cannot be changed afterward
/// through the public interface.
///
/// # Cartridge compatibility
///
/// Cartridges marked as CGB-only require
/// [`GameBoyModel::Cgb`].
///
/// Cartridges that support original Game Boy hardware
/// may be initialized under either model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameBoyModel {
    /// Original Game Boy hardware, commonly called DMG.
    ///
    /// The DMG provides a monochrome LCD, the standard
    /// Game Boy memory configuration, and normal-speed
    /// SM83 CPU execution.
    ///
    /// It does not support CGB-specific color graphics,
    /// additional memory banks, or double-speed operation.
    ///
    /// The current implementation rejects cartridges
    /// explicitly marked as CGB-only when this model
    /// is selected.
    Dmg,
    /// Game Boy Color hardware, commonly called CGB.
    ///
    /// The CGB extends the original Game Boy with
    /// color graphics, additional memory banking,
    /// improved DMA functionality, and optional
    /// double-speed CPU execution.
    ///
    /// It can also operate compatible DMG software
    /// using Game Boy compatibility behavior.
    Cgb,
}

/// Errors that may occur while constructing a Game Boy.
///
/// These errors describe failures in cartridge platform
/// validation and hardware model compatibility.
#[derive(Debug, Error)]
pub enum GameBoyError {
    /// The supplied ROM does not target Game Boy hardware.
    ///
    /// A [`GameBoy`] requires a cartridge containing
    /// [`RomHeader::Gb`] metadata.
    ///
    /// Game Boy Advance cartridges must instead be
    /// loaded by the GBA machine implementation.
    #[error("expected a Game Boy cartridge")]
    WrongPlatform,

    /// The cartridge requires Game Boy Color hardware,
    /// but the requested model is DMG.
    ///
    /// This error occurs when a cartridge declares
    /// [`GbColorSupport::CgbOnly`] and the caller
    /// selects [`GameBoyModel::Dmg`].
    ///
    /// Selecting [`GameBoyModel::Cgb`] permits
    /// initialization of the cartridge.
    #[error("CGB-only cartridge cannot run on DMG")]
    CgbRequired,

    /// The cartridge requires hardware that is not implemented yet.
    #[error("unsupported cartridge type: ${0:02X}")]
    UnsupportedCartridge(u8),
}

/// Error that may occur while executing a Game Boy instruction.
#[derive(Debug, Error)]
pub enum GameBoyStepError {
    /// The SM83 CPU encountered an execution error.
    #[error(transparent)]
    Cpu(#[from] CpuError),

    /// The virtual clock could not be advanced
    #[error(transparent)]
    Clock(#[from] ClockError),
}

/// An emulated Game Boy or Game Boy Color machine.
///
/// This structure owns the state associated with a
/// single emulated Game Boy instance.
///
/// It integrates the SM83 CPU and cartridge image with
/// the shared frontend-facing abstractions defined in
/// `rookery_common`.
///
/// # Ownership
///
/// The machine owns its cartridge ROM and CPU instance.
///
/// External callers cannot mutate these components
/// directly through the public interface.
///
/// The machine also owns its framebuffer, logical input
/// state, audio buffer, and virtual clock counter.
///
/// # Hardware model
///
/// The hardware model is selected during construction
/// using [`GameBoy::new`].
///
/// The model determines the value returned by
/// [`Machine::kind`] and establishes the compatibility
/// requirements for the supplied cartridge.
///
/// # Frontend integration
///
/// [`GameBoy`] implements the shared [`Machine`] trait.
///
/// This allows a frontend to:
///
/// - Identify the emulated hardware model.
/// - Read the framebuffer.
/// - Provide logical button input.
/// - Retrieve buffered audio samples.
/// - Inspect and advance the diagnostic virtual clock.
#[derive(Debug)]
pub struct GameBoy {
    model: GameBoyModel,
    cpu: Sm83,
    bus: GbBus,

    framebuffer: Framebuffer,
    buttons: Buttons,
    audio: AudioBuffer,
    clock: VirtualClock,
}

impl GameBoy {
    /// Constructs a Game Boy machine with the requested model.
    ///
    /// The constructor takes ownership of the supplied
    /// cartridge ROM and validates that it targets Game Boy
    /// hardware.
    ///
    /// It also verifies that the cartridge is compatible
    /// with the requested hardware model.
    ///
    /// # Initialization
    ///
    /// On successful construction, the machine contains:
    ///
    /// - The requested hardware model.
    /// - The supplied cartridge ROM.
    /// - A default SM83 CPU instance.
    /// - A 160-by-144 RGB framebuffer initialized to black.
    /// - A logical button state with all buttons released.
    /// - An empty stereo audio buffer configured for 48 kHz.
    /// - A virtual clock initialized to zero.
    ///
    /// The constructor also emits a structured diagnostic
    /// event using the `tracing` infrastructure.
    ///
    /// The event includes the selected hardware model,
    /// cartridge title, and cartridge type identifier.
    ///
    /// # Errors
    ///
    /// Returns [`GameBoyError::WrongPlatform`] if the ROM
    /// contains Game Boy Advance metadata rather than
    /// a Game Boy cartridge header.
    ///
    /// Returns [`GameBoyError::CgbRequired`] if the cartridge
    /// is marked as CGB-only and the requested model is DMG.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use gb_core::{GameBoy, GameBoyModel};
    /// use rookery_common::rom::Rom;
    ///
    /// let rom = Rom::load("game.gb")?;
    ///
    /// let machine = GameBoy::new(
    ///     rom,
    ///     GameBoyModel::Dmg,
    /// )?;
    ///
    /// assert_eq!(machine.model(), GameBoyModel::Dmg);
    ///
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(rom: Rom, model: GameBoyModel) -> Result<Self, GameBoyError> {
        Self::construct(rom, model, None)
    }

    /// Constructs a Game Boy machine with the provided DMG boot ROM.
    pub fn with_dmg_boot_rom(rom: Rom, boot_rom: [u8; 0x100]) -> Result<Self, GameBoyError> {
        Self::construct(rom, GameBoyModel::Dmg, Some(boot_rom))
    }

    /// Executes on CPU instruction and advances the machine's virtual clock.
    ///
    /// The clock is advanced by the T-cycle count reported by the CPU.
    ///
    /// # Errors
    ///
    /// Returns [`GameBoyStepError::Cpu`] if instruction execution fails.
    ///
    /// Returns [`GameBoyStepError::Clock`] if the virtual clock overflows.
    /// In that case, the instruction has already executed and its effects are not
    /// rolled back.
    pub fn step_instruction(&mut self) -> Result<StepResult, GameBoyStepError> {
        let result = self.cpu.step_with_trace(&mut self.bus)?;

        self.bus.advance_cycles(result.cycles);
        self.clock.advance(u64::from(result.cycles))?;

        Ok(result)
    }

    /// Returns the hardware model of this Game Boy.
    ///
    /// The model is selected during construction and
    /// remains unchanged for the lifetime of the instance.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use gb_core::{GameBoy, GameBoyModel};
    /// use rookery_common::rom::Rom;
    ///
    /// let rom = Rom::load("game.gbc")?;
    /// let machine = GameBoy::new(
    ///     rom,
    ///     GameBoyModel::Cgb,
    /// )?;
    ///
    /// assert_eq!(machine.model(), GameBoyModel::Cgb);
    ///
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn model(&self) -> GameBoyModel {
        self.model
    }

    /// Returns an immutable reference to the loaded cartridge.
    ///
    /// The returned [`Rom`] provides access to the cartridge's
    /// raw bytes, parsed header, and detected platform.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use gb_core::{GameBoy, GameBoyModel};
    /// use rookery_common::rom::Rom;
    ///
    /// let rom = Rom::load("game.gb")?;
    /// let machine = GameBoy::new(
    ///     rom,
    ///     GameBoyModel::Dmg,
    /// )?;
    ///
    /// println!(
    ///     "Cartridge size: {} bytes",
    ///     machine.rom().len()
    /// );
    ///
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn rom(&self) -> &Rom {
        self.bus.cartridge()
    }

    /// Returns an immutable reference to the SM83 CPU.
    ///
    /// This accessor allows callers to inspect CPU state
    /// without exposing mutable access to the processor.
    ///
    // # Examples
    ///
    /// ```no_run
    /// use gb_core::{GameBoy, GameBoyModel};
    /// use rookery_common::rom::Rom;
    ///
    /// let rom = Rom::load("game.gb")?;
    /// let machine = GameBoy::new(
    ///     rom,
    ///     GameBoyModel::Dmg,
    /// )?;
    ///
    /// let pc = machine.cpu().registers().pc;
    ///
    /// println!("PC: {pc:04X}");
    ///
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn cpu(&self) -> &Sm83 {
        &self.cpu
    }

    /// Returns a reference to the Game Boy memory bus.
    pub fn bus(&self) -> &GbBus {
        &self.bus
    }

    fn construct(
        rom: Rom,
        model: GameBoyModel,
        boot_rom: Option<[u8; 0x100]>,
    ) -> Result<Self, GameBoyError> {
        let RomHeader::Gb(header) = rom.header() else {
            return Err(GameBoyError::WrongPlatform);
        };

        if model == GameBoyModel::Dmg && header.color_support == CgbSupport::CgbOnly {
            return Err(GameBoyError::CgbRequired);
        }

        // We currently only support ROM-only cartridges. MBC1 is a workaround for Blargg.
        if header.cartridge_type != 0x00 && header.cartridge_type != 0x01 {
            return Err(GameBoyError::UnsupportedCartridge(header.cartridge_type));
        }

        let booting = boot_rom.is_some();

        let mut bus = match boot_rom {
            Some(bytes) => GbBus::with_boot_rom(rom, bytes),
            None => GbBus::new(rom),
        };

        if !booting && model == GameBoyModel::Dmg {
            bus.initialize_dmg_post_boot();
        }

        let cpu = match (booting, model) {
            (true, _) => Sm83::new(),
            (false, GameBoyModel::Dmg) => Sm83::with_dmg_boot(),
            (false, GameBoyModel::Cgb) => Sm83::with_registers(Registers {
                pc: 0x0100,
                ..Default::default()
            }), // TODO: CGB post-boot state.
        };

        Ok(Self {
            model,
            cpu,
            bus,

            framebuffer: Framebuffer::new(SCREEN_WIDTH, SCREEN_HEIGHT),
            buttons: Buttons::default(),
            audio: AudioBuffer::new(48_000).expect("Sample rate is non-zero"),
            clock: VirtualClock::default(),
        })
    }
}

impl Machine for GameBoy {
    fn kind(&self) -> MachineKind {
        match self.model {
            GameBoyModel::Dmg => MachineKind::Dmg,
            GameBoyModel::Cgb => MachineKind::Cgb,
        }
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
        self.buttons = buttons;
    }

    fn drain_audio(&mut self) -> Vec<StereoSample> {
        self.audio.drain()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rookery_common::Platform;

    fn test_rom(cgb_only: bool) -> Rom {
        let mut bytes = vec![0_u8; 0x8000];

        bytes[0x134..0x138].copy_from_slice(b"TEST");

        if cgb_only {
            bytes[0x143] = 0xC0;
        }

        Rom::from_bytes(bytes, Some(Platform::Gb)).unwrap()
    }

    #[test]
    fn initializes_dmg_machine() {
        let gb = GameBoy::new(test_rom(false), GameBoyModel::Dmg).unwrap();

        assert_eq!(gb.kind(), MachineKind::Dmg);
        assert_eq!(gb.framebuffer().width(), 160);
        assert_eq!(gb.framebuffer().height(), 144);
        assert_eq!(gb.virtual_clock(), 0);
    }

    #[test]
    fn rejects_cgb_only_rom_on_dmg() {
        let result = GameBoy::new(test_rom(true), GameBoyModel::Dmg);

        assert!(matches!(result, Err(GameBoyError::CgbRequired)));
    }

    #[test]
    fn accepts_cgb_only_rom_on_cgb() {
        let result = GameBoy::new(test_rom(true), GameBoyModel::Cgb);

        assert!(result.is_ok());
    }

    #[test]
    fn advances_virtual_clock() {
        let mut gb = GameBoy::new(test_rom(false), GameBoyModel::Dmg).unwrap();

        gb.advance_clock_only(1024).unwrap();

        assert_eq!(gb.virtual_clock(), 1024);
    }

    #[test]
    fn executes_instructions_from_cartridge_rom() {
        let mut bytes = vec![0; 0x8000];

        // NOP
        bytes[0x0100] = 0x00;

        let rom = Rom::from_bytes(bytes, Some(Platform::Gb)).unwrap();

        let mut gb = GameBoy::new(rom, GameBoyModel::Dmg).unwrap();

        let first = gb.step_instruction().unwrap();

        assert_eq!(first.cycles, 4);
        assert_eq!(gb.cpu().registers().pc, 0x0101);
        assert_eq!(gb.virtual_clock(), 4);
    }

    #[test]
    fn cpu_errors_do_not_advance_virtual_clock() {
        let mut bytes = vec![0; 0x8000];

        // Illegal SM83 opcode.
        bytes[0x0100] = 0xD3;

        let rom = Rom::from_bytes(bytes, Some(Platform::Gb)).unwrap();

        let mut gb = GameBoy::new(rom, GameBoyModel::Dmg).unwrap();

        assert!(gb.step_instruction().is_err());

        assert_eq!(gb.virtual_clock(), 0);
        assert_eq!(gb.cpu().registers().pc, 0x0100);
    }
}
