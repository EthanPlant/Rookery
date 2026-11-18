//! Cartridge ROM loading, header inspection, and platform detection.
//!
//! This module provides a common interface for loading Game Boy,
//! Game Boy Color, and Game Boy Advance cartridge ROM images.
//!
//! It is responsible for:
//!
//! - Reading ROM images from the filesystem.
//! - Identifying the target hardware family.
//! - Parsing cartridge header metadata.
//! - Inspecting header checksums and identification fields.
//! - Exposing ROM contents to the emulation cores.
//!
//! # Platform detection
//!
//! Platform detection primarily uses signatures found in cartridge
//! headers. File extensions are used as a fallback when the header
//! does not contain a recognized signature.
//!
//! Supported file extensions are:
//!
//! | Extension | Platform |
//! |-----------|----------|
//! | `.gb`     | Game Boy / Game Boy Color |
//! | `.gbc`    | Game Boy / Game Boy Color |
//! | `.gba`    | Game Boy Advance |
//!
//! The `.gb` and `.gbc` extensions identify the same hardware family.
//! The appropriate Game Boy hardware model is selected separately,
//! using information from the cartridge header and any explicit
//! model override.
//!
//! File extensions are not considered authoritative when a recognized
//! header signature identifies a different platform.
//!
//! # Header validation
//!
//! Header parsing and ROM validation are deliberately separate.
//!
//! A ROM may load successfully even if its header checksum or
//! identification fields are invalid. The resulting header exposes
//! validation results so that callers can inspect them.
//!
//! This is useful for ROM hacks, homebrew software, and development
//! images that may not have fully populated cartridge headers.
//!
//! Successful loading does not guarantee that a ROM can boot or run
//! correctly on original hardware or in the emulator.
//!
//! # Cartridge support
//!
//! This module does not implement memory bank controllers, cartridge
//! RAM, save storage, real-time clocks, or other cartridge hardware.

use std::{fs, path::Path};

use thiserror::Error;

use crate::Platform;

/// Original Game Boy cartridge-header logo.
const GB_LOGO: [u8; 48] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D,
    0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99,
    0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
];

/// Size of 1 KiB in bytes.
const KIB: usize = 1024;

/// An in-memory cartridge ROM image and its parsed header.
///
/// A [`Rom`] owns the complete byte buffer loaded from a
/// cartridge image, along with the metadata derived from
/// its header.
///
/// ROM contents are immutable through the public interface.
/// The emulation core may use the stored bytes for cartridge
/// ROM reads and memory bank controller operations.
///
/// The ROM loader does not allocate cartridge RAM, implement
/// bank switching, or manage save files.
///
/// # Loading
///
/// ROM images can be loaded from the filesystem using
/// [`Rom::load`] or constructed from an existing byte vector
/// using [`Rom::from_bytes`].
///
/// Header parsing is performed during construction.
///
/// # Validation
///
/// Construction requires a supported or hinted platform and
/// enough data to parse the corresponding cartridge header.
///
/// A successfully constructed ROM may still contain invalid
/// checksums, unsupported cartridge hardware, or malformed
/// program data.
#[derive(Debug, Clone)]
pub struct Rom {
    bytes: Vec<u8>,
    header: RomHeader,
}

impl Rom {
    /// Loads a cartridge ROM image from the filesystem.
    ///
    /// The file is read completely into memory and passed to
    /// [`Rom::from_bytes`] for platform detection and header
    /// parsing.
    ///
    /// The file extension is used as a platform hint:
    ///
    /// - `.gb` and `.gbc` indicate [`Platform::Gb`].
    /// - `.gba` indicates [`Platform::Gba`].
    ///
    /// Extension matching is case-insensitive.
    ///
    /// A recognized header signature takes precedence over
    /// the extension. If they disagree, loading fails with
    /// [`RomError::PlatformMismatch`].
    ///
    /// # Errors
    ///
    /// Returns [`RomError::Io`] if the file cannot be read.
    ///
    /// Other errors may occur during platform detection or
    /// header parsing, as described by [`Rom::from_bytes`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rookery_common::rom::Rom;
    ///
    /// let rom = Rom::load("game.gb")?;
    ///
    /// println!("Platform: {:?}", rom.platform());
    /// println!("ROM size: {} bytes", rom.len());
    ///
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn load(path: impl AsRef<Path>) -> Result<Self, RomError> {
        let path = path.as_ref();
        let bytes = fs::read(path)?;

        let hint = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .and_then(|ext| match ext.as_str() {
                "gb" | "gbc" => Some(Platform::Gb),
                "gba" => Some(Platform::Agb),
                _ => None,
            });

        Self::from_bytes(bytes, hint)
    }

    /// Constructs a cartridge ROM from an existing byte buffer.
    ///
    /// This method takes ownership of the supplied bytes,
    /// detects the cartridge's platform, and parses its header.
    ///
    /// # Platform detection
    ///
    /// Detection uses the following signatures:
    ///
    /// - Game Boy: the 48-byte Nintendo logo at ROM offsets
    ///   `0x0104..0x0134`.
    /// - Game Boy Advance: the fixed value `0x96` at offset
    ///   `0x00B2`, together with a valid header checksum.
    ///
    /// If exactly one signature matches, its platform is selected.
    ///
    /// If neither matches, the supplied `hint` is used.
    ///
    /// If both match, the ROM is considered ambiguous and
    /// loading fails.
    ///
    /// A supplied hint that conflicts with a recognized
    /// signature also causes loading to fail.
    ///
    /// # Errors
    ///
    /// Returns [`RomError::UnknownFormat`] if no platform can
    /// be identified.
    ///
    /// Returns [`RomError::AmbiguousFormat`] if both platform
    /// signatures match.
    ///
    /// Returns [`RomError::PlatformMismatch`] if a recognized
    /// platform conflicts with the supplied hint.
    ///
    /// Returns [`RomError::TooSmall`] if the ROM is too short
    /// to contain the selected platform's minimum header.
    ///
    /// # Notes
    ///
    /// When a format hint is supplied, the loader may accept
    /// ROM data without a recognized platform signature.
    ///
    /// This allows unusual homebrew images to be inspected,
    /// but does not guarantee that they can be executed.
    ///
    /// Similarly, a ROM with a recognized Game Boy logo may
    /// load even when its header checksum is invalid.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rookery_common::Platform;
    /// use rookery_common::rom::Rom;
    ///
    /// let bytes = std::fs::read("game.gba")?;
    ///
    /// let rom = Rom::from_bytes(
    ///     bytes,
    ///     Some(Platform::Agb),
    /// )?;
    ///
    /// assert_eq!(rom.platform(), Platform::Agb);
    ///
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn from_bytes(bytes: Vec<u8>, hint: Option<Platform>) -> Result<Self, RomError> {
        // Attempt to determine the type of ROM this is.
        // For GB ROMs we check if the header contains the Nintendo logo.
        // For GBA roms we check the magic fixed value at 0xB2 and if the checksum is valid.
        let gb_signature = bytes.len() >= 0x150 && &bytes[0x104..0x134] == GB_LOGO.as_slice();
        let agb_signature = bytes.len() >= 0xC0
            && bytes[0xB2] == 0x96
            && AgbHeader::checksum(&bytes) == bytes[0xBD];
        let detected = match (gb_signature, agb_signature) {
            (true, false) => Some(Platform::Gb),
            (false, true) => Some(Platform::Agb),
            (false, false) => None, // Could not determine from ROM data (e.g. some homebrew)
            (true, true) => return Err(RomError::AmbiguousFormat),
        };

        if let (Some(expected), Some(actual)) = (hint, detected)
            && expected != actual
        {
            return Err(RomError::PlatformMismatch { expected, actual });
        }

        // A valid signatue takes priority, the extension is a fallback for unusual homebrew ROMs
        let platform = detected.or(hint).ok_or(RomError::UnknownFormat)?;

        // Absolute minimum size for a valid ROM (i.e. header with no other data)
        let minimum = match platform {
            Platform::Gb => 0x150,
            Platform::Agb => 0xC0,
        };

        if bytes.len() < minimum {
            return Err(RomError::TooSmall {
                platform,
                minimum,
                actual: bytes.len(),
            });
        }

        let header = match platform {
            Platform::Gb => RomHeader::Gb(GbHeader::parse(&bytes)),
            Platform::Agb => RomHeader::Agb(AgbHeader::parse(&bytes)),
        };

        Ok(Self { bytes, header })
    }

    /// Returns the hardware family identified for this ROM.
    ///
    /// This is derived from the parsed header variant and
    /// is independent of the eventual machine model.
    ///
    /// For example, both DMG and CGB cartridges return
    /// [`Platform::Gb`].
    pub fn platform(&self) -> Platform {
        match self.header {
            RomHeader::Gb(_) => Platform::Gb,
            RomHeader::Agb(_) => Platform::Agb,
        }
    }

    /// Returns the parsed cartridge header.
    ///
    /// The returned value contains platform-specific metadata
    /// and header validation results.
    ///
    /// Callers can inspect the returned [`RomHeader`] variant
    /// to access fields specific to GB or GBA cartridges.
    ///
    /// The header remains owned by the ROM.
    pub fn header(&self) -> &RomHeader {
        &self.header
    }

    /// Returns an immutable view of the complete ROM image.
    ///
    /// The returned slice includes the cartridge header and
    /// all remaining ROM data.
    ///
    /// The emulation core can use this data for memory reads,
    /// cartridge banking, or other read-only operations.
    ///
    /// Mutating cartridge hardware state must be handled
    /// separately by the emulation core.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the total ROM image length in bytes.
    ///
    /// This is the actual size of the loaded byte buffer,
    /// not the size declared in the cartridge header.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns whether the ROM contains no bytes.
    ///
    /// ROMs constructed successfully by the current loader
    /// cannot be empty because minimum header lengths are
    /// enforced during construction.
    ///
    /// This method is provided for consistency with
    /// standard Rust collection interfaces.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Errors that may occur while loading or identifying a cartridge ROM.
///
/// These errors describe failures in file loading, format detection,
/// or minimum header-size validation.
///
/// Invalid header checksums and identification fields do not
/// necessarily produce a [`RomError`]. Where possible, these
/// conditions are reported through the parsed header metadata.
#[derive(Debug, Error)]
pub enum RomError {
    /// An operating-system error occurred while reading a ROM file.
    #[error("failed to read ROM: {0}")]
    Io(#[from] std::io::Error),

    /// The ROM's hardware family could not be determined.
    ///
    /// This occurs when the ROM contains no recognized platform
    /// signature and no supported file-extension hint was provided.
    #[error("could not identify ROM format")]
    UnknownFormat,

    /// The ROM contains signatures matching multiple platforms.
    ///
    /// The loader refuses to choose a platform when both the
    /// Game Boy and Game Boy Advance signature checks succeed.
    #[error("ROM contains conflicting platform signatures")]
    AmbiguousFormat,

    /// A recognized ROM signature conflicts with the supplied
    /// platform hint.
    ///
    /// This typically occurs when a file extension indicates
    /// one platform but its header identifies another.
    #[error("file extension suggests {expected:?} but header identifies {actual:?}")]
    PlatformMismatch {
        /// The expected platform based on the file extension.
        expected: Platform,
        /// The platform determined by the header.
        actual: Platform,
    },

    /// The ROM is too small to contain the minimum header
    /// required for the selected platform.
    ///
    /// This error checks only the minimum parseable header length,
    /// not the full cartridge ROM size.
    ///
    /// A ROM may therefore pass this check while still being
    /// truncated or inconsistent with its declared ROM size.
    #[error("{platform:?} ROM too small: expected at least {minimum} bytes, got {actual}")]
    TooSmall {
        /// The platform this ROM is for.
        platform: Platform,
        /// The minimum possible ROM size (i.e. the length of the header).
        minimum: usize,
        /// The actual size of the ROM file, in bytes.
        actual: usize,
    },
}

/// Describes a Game Boy cartridge's declared Color compatibility.
///
/// This information is derived from the cartridge header byte at
/// address `0x0143`.
///
/// The flag indicates which Game Boy models the software was
/// designed to support. It does not determine which machine
/// instance should be constructed by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgbSupport {
    /// The cartridge does not declare Game Boy Color enhancements.
    ///
    /// This is the default classification for cartridges whose
    /// CGB flag is neither `0x80` nor `0xC0`.
    ///
    /// Such cartridges are generally intended for original
    /// Game Boy hardware, although they may also be playable
    /// in Game Boy Color compatibility mode.
    DmgOnly,
    /// The cartridge supports both original Game Boy and
    /// Game Boy Color hardware.
    ///
    /// This corresponds to a CGB flag value of `0x80`.
    ///
    /// When executed on Game Boy Color hardware, the game
    /// may use additional color or hardware features.
    CgbEnhanced,
    /// The cartridge requires Game Boy Color hardware.
    ///
    /// This corresponds to a CGB flag value of `0xC0`.
    ///
    /// The emulator should reject attempts to initialize
    /// this cartridge on an original DMG machine.
    CgbOnly,
}

/// Parsed metadata from a Game Boy or Game Boy Color cartridge header.
///
/// The Game Boy cartridge header occupies the region near the end
/// of the first ROM bank, primarily addresses `0x0100..=0x014F`.
///
/// This structure exposes the information needed to identify
/// cartridge compatibility and prepare the appropriate cartridge
/// hardware implementation.
///
/// All values are derived from the ROM image and are not guaranteed
/// to be valid or consistent with its actual contents.
///
/// # Validation
///
/// The header includes validation flags for the Nintendo logo and
/// header checksum. Invalid values are reported but do not
/// automatically prevent ROM loading.
///
/// Cartridge-specific hardware support is not checked here.
#[derive(Debug, Clone)]
pub struct GbHeader {
    /// Game title extracted from the cartridge header.
    ///
    /// The title begins at address `0x0134`.
    ///
    /// The interpreted title length depends on whether the
    /// cartridge declares Game Boy Color support:
    ///
    /// - DMG-only: bytes `0x0134..0x0144`.
    /// - CGB-compatible: bytes `0x0134..0x013F`.
    ///
    /// The extracted value is converted to a string using
    /// [`ascii_field`].
    ///
    /// Trailing spaces are removed, and decoding stops at
    /// the first null byte.
    pub title: String,
    /// Declared Game Boy Color compatibility.
    ///
    /// Determined from the CGB flag at address `0x0143`.
    pub color_support: CgbSupport,
    /// Raw cartridge type identifier.
    ///
    /// Read from address `0x0147`.
    ///
    /// The cartridge type identifies the memory bank controller
    /// and certain optional cartridge hardware features, such
    /// as external RAM, battery-backed saves, or timers.
    ///
    /// This module does not interpret or instantiate those
    /// hardware components.
    pub cartridge_type: u8,
    /// Raw cartridge ROM size code.
    ///
    /// Read from address `0x0148`.
    ///
    /// Use [`GbHeader::declared_rom_size`] to translate this
    /// code into an expected ROM size in bytes.
    pub rom_size_code: u8,
    /// Raw cartridge RAM size code.
    ///
    /// Read from address `0x0149`.
    ///
    /// Use [`GbHeader::declared_ram_size`] to translate this
    /// code into a declared RAM size in bytes.
    ///
    /// The effective RAM capacity may also depend on the
    /// cartridge's memory bank controller.
    pub ram_size_code: u8,
    /// Software version number declared by the cartridge.
    ///
    /// Read from address `0x014C`.
    pub version: u8,
    /// Whether the Game Boy header checksum is valid.
    ///
    /// The checksum is calculated over bytes
    /// `0x0134..=0x014C` and compared against the stored
    /// value at address `0x014D`.
    ///
    /// This does not validate the entire ROM image.
    pub header_checksum_valid: bool,
    /// Whether the cartridge contains the expected Nintendo logo.
    ///
    /// The logo is compared against the 48-byte reference
    /// sequence stored in [`GB_LOGO`].
    ///
    /// A matching logo is useful for platform identification,
    /// but it does not guarantee that the ROM is authentic
    /// or otherwise valid.
    pub logo_valid: bool,
}

impl GbHeader {
    /// Parses a Game Boy cartridge header from a ROM image.
    ///
    /// The caller must ensure that `bytes` contains at least
    /// `0x150` bytes, otherwise this function may panic.
    ///
    /// Header validation flags are populated independently
    /// of whether their corresponding checks pass.
    pub fn parse(bytes: &[u8]) -> Self {
        let color_support = match bytes[0x143] {
            0x80 => CgbSupport::CgbEnhanced,
            0xC0 => CgbSupport::CgbOnly,
            _ => CgbSupport::DmgOnly,
        };

        // Newer CGB headers can use 0x13F..0x142 for the manufacturer code
        let title_end = match color_support {
            CgbSupport::DmgOnly => 0x144,
            _ => 0x13F,
        };

        let title = ascii_field(&bytes[0x134..title_end]);

        Self {
            title,
            color_support,
            cartridge_type: bytes[0x147],
            rom_size_code: bytes[0x148],
            ram_size_code: bytes[0x149],
            version: bytes[0x14C],
            header_checksum_valid: Self::checksum(bytes) == bytes[0x14D],
            logo_valid: &bytes[0x104..0x134] == GB_LOGO.as_slice(),
        }
    }

    // Returns the declared cartridge ROM size in bytes.
    ///
    /// The size is derived from the ROM size code stored
    /// at cartridge header address `0x0148`.
    ///
    /// Standard codes `0x00..=0x08` represent ROM sizes
    /// beginning at 32 KiB and doubling with each increment.
    ///
    /// Three additional codes describe uncommon intermediate
    /// ROM sizes:
    ///
    /// | Code | ROM size |
    /// |------|----------|
    /// | `0x52` | 1152 KiB |
    /// | `0x53` | 1280 KiB |
    /// | `0x54` | 1536 KiB |
    ///
    /// # Returns
    ///
    /// Returns `Some(size)` for a recognized size code,
    /// or `None` if the code is unsupported or unknown.
    ///
    /// The returned value describes the size declared by
    /// the header, not necessarily the actual ROM length.
    pub fn declared_rom_size(&self) -> Option<usize> {
        match self.rom_size_code {
            0x00..=0x08 => Some((32 * KIB) << self.rom_size_code),
            0x52 => Some(1152 * KIB),
            0x53 => Some(1280 * KIB),
            0x54 => Some(1536 * KIB),
            _ => None,
        }
    }

    /// Returns the declared external cartridge RAM size in bytes.
    ///
    /// The size is derived from the RAM size code stored at
    /// cartridge header address `0x0149`.
    ///
    /// Recognized codes are:
    ///
    /// | Code | RAM size |
    /// |------|----------|
    /// | `0x00` | None |
    /// | `0x01` | 2 KiB |
    /// | `0x02` | 8 KiB |
    /// | `0x03` | 32 KiB |
    /// | `0x04` | 128 KiB |
    /// | `0x05` | 64 KiB |
    ///
    /// # Returns
    ///
    /// Returns `Some(size)` for a recognized size code,
    /// including `Some(0)` when no external RAM is declared.
    ///
    /// Returns `None` for an unknown size code.
    ///
    /// The declared size does not account for unusual
    /// cartridge hardware or controller-specific RAM.
    pub fn declared_ram_size(&self) -> Option<usize> {
        match self.ram_size_code {
            0x00 => Some(0),
            0x01 => Some(2 * KIB),
            0x02 => Some(8 * KIB),
            0x03 => Some(32 * KIB),
            0x04 => Some(128 * KIB),
            0x05 => Some(64 * KIB),
            _ => None,
        }
    }

    fn checksum(bytes: &[u8]) -> u8 {
        bytes[0x134..=0x14C]
            .iter()
            .fold(0u8, |acc, &byte| acc.wrapping_sub(byte).wrapping_sub(1))
    }
}

/// Parsed metadata from a Game Boy Advance cartridge header.
///
/// The GBA cartridge header begins at ROM offset `0x00000000`
/// and includes the program entry point, Nintendo logo,
/// title, game code, maker code, and additional control fields.
///
/// This structure currently exposes only the metadata required
/// for basic identification and diagnostics.
///
/// The original entry-point instruction and Nintendo logo
/// data are retained in the ROM image but are not exposed
/// as individual fields by this structure.
///
/// # Validation
///
/// The current implementation checks the fixed header byte
/// and the header complement checksum.
///
/// It does not validate the complete Nintendo logo, verify
/// the entry-point instruction, or determine whether the
/// cartridge contains executable GBA software.
#[derive(Debug, Clone)]
pub struct AgbHeader {
    /// Game title extracted from bytes `0x00A0..0x00AC`.
    ///
    /// The title field is up to 12 bytes long and is
    /// converted to a string using [`ascii_field`].
    pub title: String,
    /// Four-character game identifier.
    ///
    /// Read from bytes `0x00AC..0x00B0`.
    ///
    /// The code is commonly used to identify a particular
    /// game and regional release.
    pub game_code: String,
    /// Two-character maker code.
    ///
    /// Read from bytes `0x00B0..0x00B2`.
    ///
    /// This field identifies the software publisher
    /// according to Nintendo's maker-code assignments.
    pub maker_code: String,
    /// Software version number.
    ///
    /// Read from byte `0x00BC`.
    pub version: u8,
    /// Whether the fixed header byte has the expected value.
    ///
    /// The GBA header stores the fixed value `0x96`
    /// at address `0x00B2`.
    ///
    /// This field is `true` when the byte matches.
    pub fixed_value_valid: bool,
    /// Whether the GBA header complement checksum is valid.
    ///
    /// The checksum is calculated over bytes
    /// `0x00A0..=0x00BC` and compared against the
    /// stored value at `0x00BD`.
    ///
    /// This checksum does not cover the entire cartridge ROM.
    pub header_checksum_valid: bool,
}

impl AgbHeader {
    /// Parses a Game Boy Advance cartridge header from a ROM image.
    ///
    /// The caller must ensure that `bytes` contains at least
    /// `0xC0` bytes, otherwise this function may panic.
    ///
    /// This function extracts identification fields and validates
    /// the fixed header byte and complement checksum.
    ///
    /// It does not validate the GBA Nintendo logo or entry-point
    /// instruction.
    pub fn parse(bytes: &[u8]) -> Self {
        Self {
            title: ascii_field(&bytes[0xA0..0xAC]),
            game_code: ascii_field(&bytes[0xAC..0xB0]),
            maker_code: ascii_field(&bytes[0xB0..0xB2]),
            version: bytes[0xBC],
            fixed_value_valid: bytes[0xB2] == 0x96,
            header_checksum_valid: Self::checksum(bytes) == bytes[0xBD],
        }
    }

    fn checksum(bytes: &[u8]) -> u8 {
        bytes[0xA0..=0xBC]
            .iter()
            .fold(0xE7u8, |acc, &byte| acc.wrapping_sub(byte))
    }
}

/// Parsed cartridge metadata for a supported hardware family.
///
/// Each variant contains the platform-specific header fields
/// associated with the loaded ROM.
///
/// This enum allows callers to inspect cartridge metadata
/// without depending on the corresponding emulation core.
#[derive(Debug, Clone)]
pub enum RomHeader {
    /// Game Boy or Game Boy Color ROM header.
    Gb(GbHeader),
    /// Game Boy Advance ROM header.
    Agb(AgbHeader),
}

fn ascii_field(bytes: &[u8]) -> String {
    bytes
        .iter()
        .copied()
        .take_while(|&b| b != 0)
        .map(|b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '\u{FFFD}'
            }
        })
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_gb_rom(cgb_flag: u8) -> Vec<u8> {
        let mut bytes = vec![0; 32 * 1024];

        bytes[0x104..0x134].copy_from_slice(&GB_LOGO);
        bytes[0x134..0x138].copy_from_slice(b"TEST");
        bytes[0x143] = cgb_flag;
        bytes[0x147] = 0x00;
        bytes[0x148] = 0x00;
        bytes[0x149] = 0x00;
        bytes[0x14D] = GbHeader::checksum(&bytes);

        bytes
    }

    fn make_gba_rom() -> Vec<u8> {
        let mut bytes = vec![0; 0xC0];

        bytes[0xA0..0xA4].copy_from_slice(b"TEST");
        bytes[0xAC..0xB0].copy_from_slice(b"TST1");
        bytes[0xB0..0xB2].copy_from_slice(b"01");
        bytes[0xB2] = 0x96;
        bytes[0xBD] = AgbHeader::checksum(&bytes);

        bytes
    }

    #[test]
    fn detects_dmg_rom() {
        let rom = Rom::from_bytes(make_gb_rom(0), None).unwrap();

        assert_eq!(rom.platform(), Platform::Gb);

        let RomHeader::Gb(header) = rom.header() else {
            panic!("expected GB header");
        };

        assert_eq!(header.title, "TEST");
        assert_eq!(header.color_support, CgbSupport::DmgOnly);
        assert!(header.header_checksum_valid);
        assert!(header.logo_valid);
        assert_eq!(header.declared_rom_size(), Some(32768));
    }

    #[test]
    fn detects_cgb_only_rom() {
        let rom = Rom::from_bytes(make_gb_rom(0xC0), None).unwrap();

        let RomHeader::Gb(header) = rom.header() else {
            panic!("expected GB header");
        };

        assert_eq!(header.color_support, CgbSupport::CgbOnly);
    }

    #[test]
    fn detects_gba_rom() {
        let rom = Rom::from_bytes(make_gba_rom(), None).unwrap();

        assert_eq!(rom.platform(), Platform::Agb);

        let RomHeader::Agb(header) = rom.header() else {
            panic!("expected GBA header");
        };

        assert_eq!(header.game_code, "TST1");
        assert!(header.fixed_value_valid);
        assert!(header.header_checksum_valid);
    }

    #[test]
    fn rejects_platform_mismatch() {
        let result = Rom::from_bytes(make_gba_rom(), Some(Platform::Gb));

        assert!(matches!(result, Err(RomError::PlatformMismatch { .. })));
    }

    #[test]
    fn reports_bad_gb_checksum() {
        let mut bytes = make_gb_rom(0);
        bytes[0x14D] ^= 0xFF;

        let rom = Rom::from_bytes(bytes, None).unwrap();

        let RomHeader::Gb(header) = rom.header() else {
            panic!("expected GB header");
        };

        assert!(!header.header_checksum_valid);
    }

    #[test]
    fn rejects_truncated_rom() {
        let result = Rom::from_bytes(vec![0; 100], Some(Platform::Gb));

        assert!(matches!(result, Err(RomError::TooSmall { .. })));
    }
}
