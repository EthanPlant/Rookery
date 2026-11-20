//! Headless runner for Blargg test ROMs that report through SB/SC serial.
//!
//! Test programs write a byte to SB ($FF01) and then $81 to SC ($FF02).
//! `GbBus` captures these bytes. We deliberately do not capture stdout.

use gb_core::{GameBoy, GameBoyError, GameBoyModel, GameBoyStepError};
use rookery_common::{rom::Rom, Machine};
use thiserror::Error;

/// 300 million DMG T-cycles is about 71.5 seconds of emulated time.
/// This is a budget of emulated time, not a wall-clock timeout.
pub const DEFAULT_MAX_CYCLES: u64 = 300_000_000;

/// Successful completion of a serial-output test ROM.
#[derive(Debug)]
pub struct BlarggReport {
    /// Number of instructions executed before the completion marker.
    pub instructions: u64,
    /// Number of elapsed emulated CPU T-cycles.
    pub cycles: u64,
    /// Final PC at the point of completion.
    pub pc: u16,
    /// Complete serial text produced by the test program.
    pub serial: String,
}

/// A failure while running a serial-output test ROM.
#[derive(Debug, Error)]
pub enum BlarggError {
    /// The ROM could not be used to construct the requested machine.
    #[error("cannot initialize Game Boy: {0}")]
    Initialize(#[from] GameBoyError),

    /// The CPU or machine halted execution with an error.
    #[error(
        "CPU stopped at PC ${pc:04X} after {cycles} T-cycles: {source}\nSerial output:\n{serial}"
    )]
    Execution {
        /// The final PC.
        pc: u16,
        /// Elapsed T-cycles.
        cycles: u64,
        /// Error reported by the machine.
        #[source]
        source: GameBoyStepError,
        /// Captured test text.
        serial: String,
    },

    /// The ROM explicitly printed `Failed` to its serial port.
    #[error(
        "Blargg reported failure at PC ${pc:04X} after {cycles} T-cycles\nSerial output:\n{serial}"
    )]
    Failed {
        /// Final PC.
        pc: u16,
        /// Elapsed T-cycles.
        cycles: u64,
        /// Captured test text.
        serial: String,
    },

    /// The ROM did not print a completion marker within the cycle budget.
    #[error("Blargg timed out at PC ${pc:04X} after {cycles} T-cycles\nSerial output:\n{serial}")]
    Timeout {
        /// Final PC.
        pc: u16,
        /// Elapsed T-cycles.
        cycles: u64,
        /// Captured test text.
        serial: String,
    },

    /// A CPU step failed to advance the machine clock.
    #[error("CPU clock stalled at PC ${pc:04X} after {cycles} T-cycles\nSerial output:\n{serial}")]
    Stalled {
        /// Final PC.
        pc: u16,
        /// Elapsed T-cycles.
        cycles: u64,
        /// Captured test text.
        serial: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Terminal {
    Passed,
    Failed,
}

/// Consumes only newly emitted bytes.  Checking the six-byte sliding window
/// avoids rescanning an ever-growing string on every CPU instruction.
#[derive(Default)]
struct ResultDetector {
    last: Vec<u8>,
}

impl ResultDetector {
    fn feed(&mut self, bytes: &[u8]) -> Option<Terminal> {
        for &byte in bytes {
            if self.last.len() == 6 {
                self.last.remove(0);
            }
            self.last.push(byte.to_ascii_lowercase());
            if self.last.as_slice() == b"passed" {
                return Some(Terminal::Passed);
            }
            if self.last.as_slice() == b"failed" {
                return Some(Terminal::Failed);
            }
        }
        None
    }
}

fn serial_text(gb: &GameBoy) -> String {
    String::from_utf8_lossy(gb.bus().serial_output()).into_owned()
}

/// Run a Blargg *individual* test ROM until `Passed`, `Failed`, or a timeout.
///
/// The combined `cpu_instrs.gb` suite should not be fed into this routine
/// until its final-result protocol is handled separately. Individual ROMs
/// emit one final result, making the serial marker unambiguous.
///
/// # Errors
///
/// Fails for initialization, CPU errors, explicit failure, a stalled clock,
/// or exceeding the emulated cycle budget.
pub fn run_serial_test(
    rom: Rom,
    model: GameBoyModel,
    max_cycles: u64,
) -> Result<BlarggReport, BlarggError> {
    let mut gb = GameBoy::new(rom, model)?;
    let mut detector = ResultDetector::default();
    let mut seen = 0usize;
    let mut instructions = 0u64;

    while gb.virtual_clock() < max_cycles {
        let before = gb.virtual_clock();
        if let Err(source) = gb.step_instruction() {
            return Err(BlarggError::Execution {
                pc: gb.cpu().registers().pc,
                cycles: gb.virtual_clock(),
                source,
                serial: serial_text(&gb),
            });
        }
        instructions += 1;

        if gb.virtual_clock() <= before {
            return Err(BlarggError::Stalled {
                pc: gb.cpu().registers().pc,
                cycles: gb.virtual_clock(),
                serial: serial_text(&gb),
            });
        }

        let serial = gb.bus().serial_output();
        if serial.len() > seen {
            let outcome = detector.feed(&serial[seen..]);
            seen = serial.len();
            if let Some(outcome) = outcome {
                let pc = gb.cpu().registers().pc;
                let cycles = gb.virtual_clock();
                let serial = serial_text(&gb);
                return match outcome {
                    Terminal::Passed => Ok(BlarggReport {
                        instructions,
                        cycles,
                        pc,
                        serial,
                    }),
                    Terminal::Failed => Err(BlarggError::Failed { pc, cycles, serial }),
                };
            }
        }
    }

    Err(BlarggError::Timeout {
        pc: gb.cpu().registers().pc,
        cycles: gb.virtual_clock(),
        serial: serial_text(&gb),
    })
}

#[cfg(test)]
mod tests {
    use super::{ResultDetector, Terminal};

    #[test]
    fn detects_success_across_serial_chunks() {
        let mut detector = ResultDetector::default();
        assert_eq!(detector.feed(b"cpu_instrs\nPas"), None);
        assert_eq!(detector.feed(b"sed"), Some(Terminal::Passed));
    }

    #[test]
    fn detects_failure_across_serial_chunks() {
        let mut detector = ResultDetector::default();
        assert_eq!(detector.feed(b"test 03\nFail"), None);
        assert_eq!(detector.feed(b"ed"), Some(Terminal::Failed));
    }

    #[test]
    fn matches_case_insensitively() {
        let mut detector = ResultDetector::default();
        assert_eq!(detector.feed(b"FAILED"), Some(Terminal::Failed));
    }
}
