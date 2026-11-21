use gb_core::{GameBoy, GameBoyError, GameBoyStepError};
use rookery_common::{Machine, Rgb, rom::Rom};
use thiserror::Error;

/// Maximum emulated T-cycles before declaring the test stuck.
///
/// The ROM normally reaches its screenshot breakpoint substantially earlier.
pub const DEFAULT_MAX_CYCLES: u64 = 10_000_000;

/// dmg-acid2 v1.0 intentionally executes LD B,B after
/// rendering ten frames to use as a breakpoint.
///
/// This byte sequence identifies that specific instruction:
///
///   DEC B
///   JR NZ, +1
///   LD B, B          <- screenshot breakpoint
///   LD A, $08
///   LDH [$45], A
///
/// Looking for the surrounding instructions avoids treating
/// an unrelated LD B,B as a completion marker.
const BREAKPOINT_SIGNATURE: &[u8] = &[0x05, 0x20, 0x01, 0x40, 0x3E, 0x08, 0xE0, 0x45];

/// Captured result at dmg-acid2's screenshot breakpoint
#[derive(Debug)]
pub struct Acid2Report {
    /// T-cycles elapsed since machine construction.
    pub cycles: u64,

    /// Number of instructions executed.
    pub instructions: u64,

    /// Program coubnter of the screenshot breakpoint.
    pub pc: u16,

    /// Native 160x144 RGB framebuffer.
    pub pixels: Vec<Rgb>,
}

/// Failures when executing dmg-acid2.
#[derive(Debug, Error)]
pub enum Acid2Error {
    #[error("dmg-acid2 v1.0 breakpoint signature not found")]
    MissingBreakpoint,

    #[error("multiple dmg-acid2 breakpoint signatures found")]
    AmbiguousBreakpoint,

    #[error("cannot initialize DMG: {0}")]
    Initialize(#[from] GameBoyError),

    #[error("CPU error at PC ${pc:04X} after {cycles} cycles: {source}")]
    Execution {
        pc: u16,
        cycles: u64,
        #[source]
        source: GameBoyStepError,
    },

    #[error("CPU clock stalled at PC ${pc:04X} after {cycles} cycles")]
    Stalled { pc: u16, cycles: u64 },

    #[error("dmg-acid2 timed out at PC ${pc:04X} after {cycles} cycles")]
    Timeout { pc: u16, cycles: u64 },
}

/// Run dmg-acid2 until its deliberatel screenshot breakpoint.
///
/// Uses the DMG model without a boot ROM, matching the
/// emulator's normal post-boot execution configuration.
///
/// # Errors
///
/// Returns an error if initialization or execution fails,
/// the breakpoint cannot be identified, the clock stalls,
/// or the cycle budget is exceeded.
pub fn run_dmg_acid2(rom: Rom, max_cycles: u64) -> Result<Acid2Report, Acid2Error> {
    let breakpoint = screenshot_breakpoint(rom.bytes())?;

    let mut gb = GameBoy::new(rom, gb_core::GameBoyModel::Dmg)?;
    let mut instructions = 0;

    while gb.virtual_clock() < max_cycles {
        let pc = gb.cpu().registers().pc;

        if pc == breakpoint {
            return Ok(Acid2Report {
                cycles: gb.virtual_clock(),
                instructions,
                pc,
                pixels: gb.framebuffer().pixels().to_vec(),
            });
        }

        let before = gb.virtual_clock();

        if let Err(source) = gb.step_instruction() {
            return Err(Acid2Error::Execution {
                pc,
                cycles: gb.virtual_clock(),
                source,
            });
        }

        instructions += 1;

        if gb.virtual_clock() < before {
            return Err(Acid2Error::Stalled {
                pc,
                cycles: gb.virtual_clock(),
            });
        }
    }

    Err(Acid2Error::Timeout {
        pc: gb.cpu().registers().pc,
        cycles: gb.virtual_clock(),
    })
}

/// Find dmg-acid2's intentional breakpoint.
fn screenshot_breakpoint(rom: &[u8]) -> Result<u16, Acid2Error> {
    let fixed_bank = &rom[..rom.len().min(0x4000)];

    let mut matches = fixed_bank
        .windows(BREAKPOINT_SIGNATURE.len())
        .enumerate()
        .filter_map(|(offset, bytes)| (bytes == BREAKPOINT_SIGNATURE).then_some(offset));

    let Some(offset) = matches.next() else {
        return Err(Acid2Error::MissingBreakpoint);
    };

    if matches.next().is_some() {
        return Err(Acid2Error::AmbiguousBreakpoint);
    }

    Ok((offset + 3) as u16)
}
