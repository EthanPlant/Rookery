//! External-ROM integration tests, enabled with `--features blargg-roms`.
//! These are intentionally not silently skipped when ROM files are absent.

use std::{env, path::PathBuf};

use gb_core::GameBoyModel;
use rookery_common::rom::Rom;
use rookery_test::blargg::{DEFAULT_MAX_CYCLES, run_serial_test};

fn rom_root() -> PathBuf {
    env::var_os("BLARGG_ROM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/blargg"))
}

fn run_case(filename: &str) {
    let path = rom_root().join(filename);
    assert!(
        path.is_file(),
        "missing Blargg fixture {}. Clone retrio/gb-test-roms into roms/blargg/ or set BLARGG_ROM_DIR to the checkout root",
        path.display()
    );

    let rom =
        Rom::load(&path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let report = run_serial_test(rom, GameBoyModel::Dmg, DEFAULT_MAX_CYCLES)
        .unwrap_or_else(|error| panic!("{filename}: {error}"));

    // Only printed with `--nocapture` or when the test fails.
    eprintln!(
        "{filename}: Passed in {} instructions / {} T-cycles (PC ${:04X})\n{}",
        report.instructions, report.cycles, report.pc, report.serial
    );
}

macro_rules! blargg_case {
    ($name:ident, $filename:literal) => {
        #[test]
        fn $name() {
            run_case($filename);
        }
    };
}

blargg_case!(special, "cpu_instrs/individual/01-special.gb");
blargg_case!(interrupts, "cpu_instrs/individual/02-interrupts.gb");
blargg_case!(op_sp_hl, "cpu_instrs/individual/03-op sp,hl.gb");
blargg_case!(op_r_imm, "cpu_instrs/individual/04-op r,imm.gb");
blargg_case!(op_rp, "cpu_instrs/individual/05-op rp.gb");
blargg_case!(ld_r_r, "cpu_instrs/individual/06-ld r,r.gb");
blargg_case!(
    control_flow,
    "cpu_instrs/individual/07-jr,jp,call,ret,rst.gb"
);
blargg_case!(misc_instructions, "cpu_instrs/individual/08-misc instrs.gb");
blargg_case!(op_r_r, "cpu_instrs/individual/09-op r,r.gb");
blargg_case!(bit_ops, "cpu_instrs/individual/10-bit ops.gb");
blargg_case!(op_a_hl, "cpu_instrs/individual/11-op a,(hl).gb");
blargg_case!(instr_timing, "instr_timing/instr_timing.gb");
