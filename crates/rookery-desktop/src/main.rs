use std::{
    fs, io,
    path::{Path, PathBuf},
};

use agb_core::Gba;
use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Parser, Subcommand, ValueEnum};
use gb_core::{GameBoy, GameBoyModel};
use rookery_common::{
    Machine, Platform,
    rom::{CgbSupport, Rom, RomHeader},
    trace::{
        InstructionRecord,
        InstructionSet::{self},
        TextTraceSink, TraceSink,
    },
};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "rookery",
    version,
    about = "DMG/CGB/GBA emulator written in Rust"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect a ROM header.
    Inspect { rom: PathBuf },

    /// Initialize a machine and advance its clock.
    Run {
        #[command(flatten)]
        load: LoadArgs,

        /// Clock cycles to advance.
        #[arg(long)]
        cycles: Option<u64>,
    },

    /// Trace executed instructions.
    Trace {
        #[command(flatten)]
        load: LoadArgs,

        #[arg(long, default_value_t = 10_000)]
        instructions: u64,
    },

    /// Run built-in test suites.
    Test {
        #[command(subcommand)]
        suite: TestSuite,
    },
}

#[derive(Args)]
struct LoadArgs {
    /// Path to the cartridge ROM.
    rom: PathBuf,

    /// Override Game Boy hardware model.
    #[arg(long, value_enum)]
    model: Option<ModelArg>,

    /// Path to a user-supplied BIOS
    #[arg(long)]
    bios: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModelArg {
    Dmg,
    Cgb,
}

#[derive(Subcommand)]
enum TestSuite {
    /// Run cartridge-header smoke tests
    Headers,
}

enum LoadedMachine {
    Gb(Box<GameBoy>),
    Gba(Box<Gba>),
}

#[derive(Debug, Clone, Copy)]
enum ExecutionLimit {
    Cycles(u64),
    Instructions(u64),
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let default_filter = if cli.verbose > 0 { "debug" } else { "info" };

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    match cli.command {
        Command::Inspect { rom } => {
            let cartridge =
                Rom::load(&rom).with_context(|| format!("loading {}", rom.display()))?;

            print_rom_info(&cartridge);
        }

        Command::Run { load, cycles } => {
            let mut machine = load_machine(&load)?;

            match &mut machine {
                LoadedMachine::Gb(gb) => {
                    let budget = cycles.unwrap_or(1_000_000_000);

                    println!("Machine: {:?}", gb.model());
                    println!("Boot ROM mapped: {}", gb.bus().boot_rom_mapped());
                    println!("Starting PC: ${:04X}", gb.cpu().registers().pc);
                    println!("Cycle budget: {budget}");
                    println!();

                    run_gb(gb, ExecutionLimit::Cycles(budget), false)?;
                }
                LoadedMachine::Gba(gba) => {
                    let budget = cycles.unwrap_or(gb_core::CLOCKS_PER_FRAME);
                    gba.advance_clock_only(budget)?;
                    println!("Machine: GBA");
                    println!("Execution: clock-only (M0)");
                    println!("Virtual cycles: {}", gba.virtual_clock());
                    println!("CPU instructions executed: 0");
                }
            }
        }

        Command::Trace { load, instructions } => {
            let mut machine = load_machine(&load)?;

            match &mut machine {
                LoadedMachine::Gb(gb) => {
                    println!("Machine: {:?}", gb.model());
                    println!("Boot ROM mapped: {}", gb.bus().boot_rom_mapped());
                    println!("Starting PC: ${:04X}", gb.cpu().registers().pc);
                    println!();

                    run_gb(gb, ExecutionLimit::Instructions(instructions), true)?;
                }
                LoadedMachine::Gba(_) => {
                    bail!("GBA instruction tracing requires the ARM7TDMI interpreter");
                }
            }
        }

        Command::Test { suite } => match suite {
            TestSuite::Headers => run_header_tests()?,
        },
    }

    Ok(())
}

fn run_gb(gb: &mut GameBoy, limit: ExecutionLimit, trace: bool) -> Result<u64> {
    if !trace {
        return execute_gb(gb, limit, None);
    }

    let stdout = io::stdout();
    let mut sink = TextTraceSink::new(stdout.lock());

    execute_gb(gb, limit, Some(&mut sink))
}

fn load_machine(args: &LoadArgs) -> Result<LoadedMachine> {
    let rom =
        Rom::load(&args.rom).with_context(|| format!("loading ROM {}", args.rom.display()))?;
    print_rom_info(&rom);
    println!();

    match rom.platform() {
        Platform::Gb => {
            let default_model = match rom.header() {
                RomHeader::Gb(header) => match header.color_support {
                    CgbSupport::DmgOnly => GameBoyModel::Dmg,
                    CgbSupport::CgbEnhanced | CgbSupport::CgbOnly => GameBoyModel::Cgb,
                },
                _ => unreachable!(),
            };

            let model = match args.model {
                Some(ModelArg::Dmg) => GameBoyModel::Dmg,
                Some(ModelArg::Cgb) => GameBoyModel::Cgb,
                // The only GB boot ROM currently supported is the 256-byte DMG ROM.
                None if args.bios.is_some() => GameBoyModel::Dmg,
                None => default_model,
            };

            let gb = if let Some(path) = &args.bios {
                ensure!(
                    model == GameBoyModel::Dmg,
                    "The 256-bute DMG ROM requires --model DMG"
                );

                let boot_rom = load_dmg_boot_rom(path)?;

                GameBoy::with_dmg_boot_rom(rom, boot_rom)?
            } else {
                GameBoy::new(rom, model)?
            };
            Ok(LoadedMachine::Gb(Box::new(gb)))
        }
        Platform::Agb => {
            ensure!(
                args.model.is_none(),
                "--model is only supported for Game Boy cartridges"
            );

            let bios = match &args.bios {
                Some(path) => {
                    let bytes = fs::read(path)
                        .with_context(|| format!("reading BIOS {}", path.display()))?;
                    Some(bytes)
                }
                None => None,
            };
            let machine = Gba::new(rom, bios)?;

            Ok(LoadedMachine::Gba(Box::new(machine)))
        }
    }
}

fn print_rom_info(rom: &Rom) {
    println!("ROM information");
    println!("---------------");
    println!("Platform: {:?}", rom.platform());
    println!("File size: {} bytes", rom.len());

    match rom.header() {
        RomHeader::Gb(header) => {
            println!("Title: {}", header.title);
            println!("Color Support: {:?}", header.color_support);
            println!("Cartridge type: {:#04X}", header.cartridge_type);
            println!("ROM size code: {:#04X}", header.rom_size_code);
            println!("RAM size code: {:#04X}", header.ram_size_code);
            println!("Declared ROM size: {:?}", header.declared_rom_size());
            println!("Declared RAM size: {:?}", header.declared_ram_size());
            println!(
                "Header checksum: {}",
                check_status(header.header_checksum_valid)
            );
            println!("Nintendo Logo: {}", check_status(header.logo_valid));

            if !header.header_checksum_valid || !header.logo_valid {
                println!("Warning: GB header validation failed");
            }

            if let Some(size) = header.declared_rom_size()
                && size != rom.len()
            {
                println!("Warning: declared ROM size differs from file size");
            }
        }
        RomHeader::Agb(header) => {
            println!("Title: {}", header.title);
            println!("Game Code: {}", header.game_code);
            println!("Maker code: {}", header.maker_code);
            println!("Version: {}", header.version);

            println!(
                "Fixed header byte: {}",
                check_status(header.fixed_value_valid)
            );
            println!(
                "Header checksum: {}",
                check_status(header.header_checksum_valid)
            );

            if !header.fixed_value_valid || !header.header_checksum_valid {
                println!("Warning: GBA header validation failed");
            }
        }
    }
}

fn run_header_tests() -> Result<()> {
    let results = rookery_test::run_header_tests();

    let mut passed = 0;

    for result in &results {
        if result.passed {
            println!("[PASS] {}", result.name);
            passed += 1;
        } else {
            println!("[FAIL] {}", result.name);
        }
    }

    println!();
    println!("{passed}/{} tests passed", results.len());

    if passed != results.len() {
        bail!("header test suite failed");
    }

    Ok(())
}

fn execute_gb(
    gb: &mut GameBoy,
    limit: ExecutionLimit,
    mut trace_sink: Option<&mut dyn TraceSink>,
) -> Result<u64> {
    let start_cycles = gb.virtual_clock();
    let mut instructions = 0;
    loop {
        let elapsed = gb.virtual_clock() - start_cycles;

        let finished = match limit {
            ExecutionLimit::Cycles(cycles) => elapsed >= cycles,
            ExecutionLimit::Instructions(instrs) => instructions >= instrs,
        };

        if finished {
            break;
        }

        let step = match gb.step_instruction() {
            Ok(step) => step,
            Err(e) => bail!("SM83 execution stopped: {e}"),
        };

        if let Some(sink) = trace_sink.as_deref_mut() {
            let disassembly = step.instruction.to_string();

            let opcode = match step.opcode {
                sm83::Opcode::Base(code) => u32::from(code),
                sm83::Opcode::Cb(code) => 0xCB00 | u32::from(code),
            };

            let record = InstructionRecord {
                index: instructions,
                isa: InstructionSet::Sm83,
                pc: u32::from(step.pc),
                opcode,
                cycles: step.cycles,
                disassembly: &disassembly,
            };

            sink.record(&record)
                .context("writing SM83 instruction trace")?;
        }

        instructions += 1;
    }

    Ok(instructions)
}

fn load_dmg_boot_rom(path: &Path) -> Result<[u8; 256]> {
    let bytes =
        fs::read(path).with_context(|| format!("reading DMG boot ROM {}", path.display()))?;

    ensure!(
        bytes.len() == 256,
        "invalid DMG boot ROM size: expected 256 bytes, got {} bytes",
        bytes.len()
    );

    let mut boot_rom = [0u8; 256];
    boot_rom.copy_from_slice(&bytes);

    Ok(boot_rom)
}

fn check_status(valid: bool) -> &'static str {
    if valid { "OK" } else { "INVALID" }
}
