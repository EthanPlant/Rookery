use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use gb_core::GameBoy;
use tempfile::NamedTempFile;

const SAVE_INTERNAL: Duration = Duration::from_secs(5);

pub struct SaveManager {
    path: Option<PathBuf>,
    last_flush: Instant,
}

impl SaveManager {
    pub fn load(rom_path: &Path, gb: &mut GameBoy) -> Result<Self> {
        let path = gb
            .battery_ram()
            .filter(|ram| !ram.is_empty())
            .map(|_| rom_path.with_extension("sav"));

        if let Some(ref save_path) = path {
            match fs::read(save_path) {
                Ok(bytes) => {
                    gb.load_battery_ram(&bytes)
                        .with_context(|| format!("restoring SRAM from {}", save_path.display()))?;
                    eprintln!("Loaded battery-backed SRAM");
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    eprintln!("No existing battery save; starting fresh")
                }
                Err(e) => {
                    return Err(e)
                        .with_context(|| format!("reading SRAM save {}", save_path.display()));
                }
            }
        }

        Ok(Self {
            path,
            last_flush: Instant::now(),
        })
    }

    pub fn maybe_flush(&mut self, gb: &mut GameBoy) -> Result<()> {
        if self.last_flush.elapsed() < SAVE_INTERNAL {
            return Ok(());
        }

        self.flush(gb)?;
        self.last_flush = Instant::now();

        Ok(())
    }

    pub fn flush(&mut self, gb: &mut GameBoy) -> Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };

        if !gb.battery_ram_dirty() {
            return Ok(());
        }

        let ram = gb
            .battery_ram()
            .with_context(|| format!("persisting SRAM to {}", path.display()))?;
        atomic_write(path, ram)
            .with_context(|| format!("persisting SRAM to {}", path.display()))?;

        gb.acknowledge_battery_save();
        self.last_flush = Instant::now();

        Ok(())
    }
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temp = NamedTempFile::new_in(parent)
        .with_context(|| format!("creating temporary save in {}", parent.display()))?;

    temp.write_all(data)
        .context("writing temporary SRAM image")?;
    temp.flush().context("flushing temporary SRAM image")?;
    temp.as_file()
        .sync_all()
        .context("synchronizing temporary SRAM image")?;
    temp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("replacing SRAM file {}", path.display()))?;

    // On Unix, also synchronize the directory entry after rename.
    #[cfg(unix)]
    {
        fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .with_context(|| format!("synchronizing save directory {}", parent.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::GameBoyModel;
    use rookery_common::{Platform, rom::Rom};

    fn test_machine() -> GameBoy {
        let mut bytes = vec![0; 0x8000];
        bytes[0x147] = 0x03; // MBC1 + RAM + battery
        bytes[0x149] = 0x02; // 8 KiB SRAM

        let rom = Rom::from_bytes(bytes, Some(Platform::Gb)).unwrap();

        GameBoy::new(rom, GameBoyModel::Dmg).unwrap()
    }

    #[test]
    fn atomic_write_creates_and_replaces_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("game.sav");

        atomic_write(&path, &[1, 2, 3]).unwrap();
        assert_eq!(fs::read(&path).unwrap(), &[1, 2, 3]);

        atomic_write(&path, &[4, 5, 6]).unwrap();
        assert_eq!(fs::read(&path).unwrap(), &[4, 5, 6]);
    }

    #[test]
    fn existing_sram_is_restored() {
        let dir = tempfile::tempdir().unwrap();
        let rom_path = dir.path().join("game.gb");
        let save_path = dir.path().join("game.sav");

        let bytes = vec![0x42; 0x2000];
        fs::write(&save_path, &bytes).unwrap();

        let mut gb = test_machine();

        SaveManager::load(&rom_path, &mut gb).unwrap();

        assert_eq!(gb.battery_ram().unwrap(), bytes);
        assert!(!gb.battery_ram_dirty());
    }

    #[test]
    fn invalid_existing_save_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let rom_path = dir.path().join("game.gb");
        let save_path = dir.path().join("game.sav");

        fs::write(&save_path, [1, 2, 3]).unwrap();

        let mut gb = test_machine();
        assert!(SaveManager::load(&rom_path, &mut gb).is_err());

        assert_eq!(fs::read(&save_path).unwrap(), &[1, 2, 3]);
    }
}
