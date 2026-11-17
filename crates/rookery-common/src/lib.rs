//! This crate provides platform-independent abstractions used by the Game Boy, Game Boy Color, and Game Boy
//! Advance emulation cores.
//!
//! It defines common representations for video output, audio samples, button input, machine identification,
//! and virutal clock tracking.
//!
//! The interfaces in this crate deliberately avoid dependencies on operating systems, graphics APIs, and audio
//! backends. Frontends such as SDL can consume these types without introducing platform-specific dependencies
//! into the emulation cores.
//!
//! Hardware-specific behaviour, including CPU execution, memory mapping, and peripheral emulation, belongs in
//! the respective machine crates.

/// Cartridge ROM loading, header parsing, and platform detection.
pub mod rom;
/// Instruction tracing interfaces and diagnostic output formats.
pub mod trace;

use thiserror::Error;

/// Common frontend-facing interface for an emulated machine.
///
/// Frontends can interact with machines through this interface without
/// depending on hardware-specific implementations.
pub trait Machine {
    /// Returns the hardware model represented by this machine.
    ///
    /// The returned model remains constant for the lifetime of the machine instance.
    fn kind(&self) -> MachineKind;

    /// Returns the current virtualk clock counter.
    ///
    /// The counter represents elapsed ticks in the machine's chosen clock domain.
    fn virtual_clock(&self) -> u64;

    /// Advances the machine's virtual clock. This machine updates the clock counter
    /// without executing CPU instructions, rendering graphics, generating audio,
    /// or advancing hardware peripherals.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::Overflow`] if the counter would overflow.
    fn advance_clock_only(&mut self, cycles: u64) -> Result<(), ClockError>;

    /// Returns the machine's current framebuffer.
    ///
    /// The framebuffer contains RGB pixels at the emulated hardware's
    /// native screen resolution.
    ///
    /// The caller may read the pixels but cannot modify them through this interface.
    ///
    /// Framebuffer contents are not guaranteed to represent a completed frame
    /// until frame execution is implemented.
    fn framebuffer(&self) -> &Framebuffer;

    /// Returns the machine's current logical button state.
    ///
    /// The returned value is a copy and may be inspected without
    /// modifying machine state.
    fn buttons(&self) -> Buttons;

    /// Replaces the machine's current logical button state.
    ///
    /// This sets the complete input state rather than updating individual buttons.
    ///
    /// Machine implementations are responsible for translating these logical states
    /// into their hardware-specific input register representations.
    fn set_buttons(&mut self, buttons: Buttons);

    /// Removes and returns all currently buffered audio samples.
    ///
    /// Samples are returned in chronological order as stereo floating-point sample frames.
    ///
    /// After the call, the machine's audio output buffer is empty.
    ///
    /// An empty vector indicates that no audio samples are currently available.
    fn drain_audio(&mut self) -> Vec<StereoSample>;
}

/// Identifies the hardware family targeted by a cartridge ROM.
///
/// Game Boy and Game Boy Color cartridges share the [`Platform::Gb`] family.
/// The actual hardware model is selected appropriately using [`MachineKind`] or a
/// machine-sepcific model type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Original Game Boy or Game Boy Color cartridge.
    Gb,
    /// Game Boy Advance cartridge.
    Agb,
}

/// Identiifes the specific emulated hardware model.
///
/// Unlike [`Platform`], which describes a cartridge's hardware family,
/// this enum identifies the machine being emulated.
///
/// For exazmple, a Game Boy cartridge may run on either a DMG or CGB machine,
/// depending on its compatibility requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineKind {
    /// Original monochrome Game Boy (DMG-001)
    Dmg,
    /// Game Boy Color (CGB-001)
    Cgb,
    /// Game Boy Advance (AGB-001)
    Agb,
}

/// A single pixel represented by 8-bit RGB.
///
/// Each channel ranges from `0` to `255`, where `0` represents minimum intensity, and `255` represents maximum intensity.
///
/// This represenation is independent of the native pixel formats used by the emulated hardware.
/// Hardware-specific color values must be converted to RGB before being written to a framebuffer.
///
/// Pixels do not contain an alpha channel.
///
/// The `repr(C)` attribute guarantees the field order and layout, allowing the type
/// to be used with compatibile RGB24 renderers.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// A two-dimensional framebuffer containing RGB pixels.
///
/// Pixels are stored in a contiguous, row-major buffer, beginning with the top-left pixel
/// and proceeding from left to right, then top to bottom.
///
/// The pixel at `(x, y)` is located at `y * width + x`.
///
/// Framebuffer dimensions remain constante for the lifetime of the instance.
///
/// The framebuffer does not perform scaling, color correction, synchronization,
/// or presentation.
#[derive(Debug)]
pub struct Framebuffer {
    width: usize,
    height: usize,
    pixels: Vec<Rgb>,
}

impl Framebuffer {
    /// Creates a framebuffer with the specified dimensions.
    ///
    /// Every pixel is initialized to black.
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![Rgb::default(); width * height],
        }
    }

    /// Returns the width of the framebuffer in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Returns the height of the framebuffer in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Returns an immutable view of the framebuffer pixels.
    ///
    /// Pixels are stored in row-major order. The returned slice
    /// always contains `width * height* elements.
    pub fn pixels(&self) -> &[Rgb] {
        &self.pixels
    }

    /// Returns a mutable view of the framebuffer's pixels.
    ///
    /// This allows an emulation core to write directly to the framebuffer.
    ///
    /// Callers are responsible for respecting the framebuffer's dimensions
    /// when calculating pixel indcies.
    pub fn pixels_mut(&mut self) -> &mut [Rgb] {
        &mut self.pixels
    }

    /// Fills the entire framebuffer with a single colour.
    ///
    /// This operation overwrites every pixel, regardless of its previous contents.
    pub fn clear(&mut self, color: Rgb) {
        self.pixels.fill(color);
    }
}

/// A physical or logical button supported by the emulated hardware.
///
/// The shared button set contains all controls available on the DMG, CGB, and AGB systems.
///
/// The `L` and `R` buttons are specific to the Game Boy Advance. Emulation cores
/// are responsible for interpreting only the buttons supported by their respective hardware.
///
/// Discriminant values correspond to bit positions in [`Buttons`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Button {
    Right = 0,
    Left = 1,
    Up = 2,
    Down = 3,
    A = 4,
    B = 5,
    Select = 6,
    Start = 7,

    // AGB-specific
    L = 8,
    R = 9,
}

/// Represents the current pressed or released state of all buttons.
///
/// Button states are stored as a bitfield, with each bit corresponding to the
/// discriminant of a [`Button`] variant.
///
/// A set bit indicates that a button is pressed, and an unset bit represents
/// that the button is released.
///
/// This representation describes logical input state rather than hardware register values.
/// Emulation cores must translate these states into the appropriate hardware-specific input
/// registers.
///
/// For example, the Game Boy joypad register uses active-low button signals, whereas this
/// structure uses active-high bits.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Buttons {
    bits: u16,
}

impl Buttons {
    /// Creates a new button state with all buttons released.
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates the pressed state of a single button. The states of all
    /// other buttons remain unchanged.
    pub fn set(&mut self, button: Button, pressed: bool) {
        let mask = 1u16 << (button as u8);

        if pressed {
            self.bits |= mask
        } else {
            self.bits &= !mask
        }
    }

    /// Returns whether the specified button is currently pressed.
    pub fn is_pressed(&self, button: Button) -> bool {
        self.bits & (1u16 << (button as u8)) != 0
    }

    /// Returns the raw button-state bitfield.
    ///
    /// Each bit corresponds to a [`Button`] discriminant.
    /// A value of `1` indicates a pressed button.
    ///
    /// This value is not a hardware joypad register, and should not
    /// be written directly to emulated I/O registers.
    pub fn bits(&self) -> u16 {
        self.bits
    }
}

/// Errors that may occur with the audio interface.
#[derive(Debug, Error)]
pub enum AudioError {
    /// The audio buffer was initialized with a sample rate of `0`.
    #[error("sample rate is zero")]
    ZeroSampleRate,
}

/// A single stereo audio sample.
///
/// Each frame contains one floating-point sample for the left
/// and right channels.
///
/// Audio samples use a normalized floating-point representation,
/// conventionally ranging from `-1.0` to `1.0`. Audio backends are
/// responsible for any required clipping, conversion, or additional
/// processing.
///
/// One [`StereoSample`] represents both channels at a single sample instant, not
/// two successive sample frames.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct StereoSample {
    pub left: f32,
    pub right: f32,
}

/// A buffer of stereo audio sample frames.
///
/// Samples are stored in chronological order at a fixed output sample rate.
///
/// The buffer provides a simple mechanism for transferring generated audio from an
/// emulation core to a frontend. It does not perform resampling, filtering, mixing,
/// or real-time audio scheduling.
#[derive(Debug)]
pub struct AudioBuffer {
    sample_rate: u32,
    samples: Vec<StereoSample>,
}

impl AudioBuffer {
    /// Creates an empty audio buffer with the specified sample rate.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::ZeroSampleRate`] if the sample rate is zero.
    pub fn new(sample_rate: u32) -> Result<Self, AudioError> {
        if sample_rate == 0 {
            return Err(AudioError::ZeroSampleRate);
        }

        Ok(Self {
            sample_rate,
            samples: Vec::new(),
        })
    }

    /// Returns the configured audio sample rate in hertz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Appends a stereo sample frame to the buffer.
    pub fn push(&mut self, sample: StereoSample) {
        self.samples.push(sample)
    }

    /// Returns the number of stereo sample frames in the buffer.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Returns whether the buffer contains no audio sample frames.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Removes and returns all accumulated audio sample frames.
    ///
    /// The returned vector preserves the original sample order.
    ///
    /// The internal buffer is replaced with an empty vector, transferring
    /// ownership of its previous allocation to the caller without copying
    /// the samples.
    pub fn drain(&mut self) -> Vec<StereoSample> {
        std::mem::take(&mut self.samples)
    }
}

/// Errors that may occur while advancing a virtual clock.
#[derive(Debug, Error)]
pub enum ClockError {
    /// Advancing the clock would exceed [`u64::MAX`].
    #[error("virtual cycle counter overflow")]
    Overflow,
}

/// A monotonically increasing counter of virtual clock ticks.
///
/// This structure tracks elapsed virtual time independent of wall-clock time.
///
/// The meaning of a tick is determined by the owning machine. It may represent
/// a master clock cycle, CPU clock cycle, or another explicitly defined clock.
#[derive(Debug, Default)]
pub struct VirtualClock {
    cycles: u64,
}

impl VirtualClock {
    /// Returns the total number of accumulated virtual clock ticks.
    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Advances the virtual clocks by the specified number of ticks.
    ///
    /// A zero increment leaves the clock unchanged.
    ///
    /// This method does no perform emulation or advance any hardware components.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::Overflow`] if the resulting counter would exceed `u64::MAX`.
    /// On failure, the counter remains unchanged.
    pub fn advance(&mut self, delta: u64) -> Result<(), ClockError> {
        self.cycles = self.cycles.checked_add(delta).ok_or(ClockError::Overflow)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_are_independent() {
        let mut buttons = Buttons::new();

        buttons.set(Button::A, true);
        buttons.set(Button::Start, true);

        assert!(buttons.is_pressed(Button::A));
        assert!(buttons.is_pressed(Button::Start));
        assert!(!buttons.is_pressed(Button::B));

        buttons.set(Button::A, false);

        assert!(!buttons.is_pressed(Button::A));
        assert!(buttons.is_pressed(Button::Start));
    }

    #[test]
    fn framebuffer_has_expected_size() {
        let fb = Framebuffer::new(160, 144);

        assert_eq!(fb.width(), 160);
        assert_eq!(fb.height(), 144);
        assert_eq!(fb.pixels().len(), 160 * 144);
    }

    #[test]
    fn audio_drain_empties_buffer() {
        let mut audio = AudioBuffer::new(48_000).unwrap();

        audio.push(StereoSample {
            left: 0.5,
            right: -0.5,
        });

        assert_eq!(audio.drain().len(), 1);
        assert!(audio.is_empty());
    }

    #[test]
    fn clock_detects_overflow() {
        let mut clock = VirtualClock::default();

        clock.advance(u64::MAX).unwrap();

        assert!(clock.advance(1).is_err());
        assert_eq!(clock.cycles(), u64::MAX);
    }
}
