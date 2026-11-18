use std::{fs, path::PathBuf};

use agb_core::Gba;
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use gb_core::{GameBoy, GameBoyModel};
use rookery_common::{
    Machine, MachineKind, Platform,
    rom::{CgbSupport, Rom, RomHeader},
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

    /// Path to a user-supplied GBA BIOS
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

            let budget = cycles.unwrap_or_else(|| match machine.kind() {
                MachineKind::Dmg | MachineKind::Cgb => gb_core::CLOCKS_PER_FRAME,
                MachineKind::Agb => agb_core::CLOCKS_PER_FRAME,
            });

            machine
                .advance_clock_only(budget)
                .context("advancing virtual clock")?;

            println!();
            println!("Machine: {:?}", machine.kind());
            println!("Virtual clocks: {}", machine.virtual_clock());
            println!(
                "Framebuffer {}x{}",
                machine.framebuffer().width(),
                machine.framebuffer().height()
            );
        }

        Command::Trace { load, .. } => {
            let machine = load_machine(&load)?;

            println!("Initialized {:?} for instruction tracing", machine.kind());
        }

        Command::Test { suite } => match suite {
            TestSuite::Headers => run_header_tests()?,
        },
    }

    Ok(())
}

fn load_machine(args: &LoadArgs) -> Result<Box<dyn Machine>> {
    let rom =
        Rom::load(&args.rom).with_context(|| format!("loading ROM {}", args.rom.display()))?;
    print_rom_info(&rom);
    println!();

    match rom.platform() {
        Platform::Gb => {
            if args.bios.is_some() {
                bail!("--bios is only supported for GBA cartridges");
            }

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
                None => default_model,
            };

            let machine = GameBoy::new(rom, model)?;

            Ok(Box::new(machine))
        }
        Platform::Agb => {
            if args.model.is_some() {
                bail!("--model is only valid for Game Boy cartridges");
            }

            let bios = match &args.bios {
                Some(path) => {
                    let bytes = fs::read(path)
                        .with_context(|| format!("reading BIOS {}", path.display()))?;
                    Some(bytes)
                }
                None => None,
            };
            let machine = Gba::new(rom, bios)?;

            println!(
                "GBA BIOS: {}",
                if machine.bios_present() {
                    "supplied"
                } else {
                    "not supplied"
                }
            );

            Ok(Box::new(machine))
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

fn check_status(valid: bool) -> &'static str {
    if valid { "OK" } else { "INVALID" }
}
