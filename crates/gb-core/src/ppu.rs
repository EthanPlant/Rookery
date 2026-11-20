use std::sync::RwLockReadGuard;

use rookery_common::{Framebuffer, Rgb};
use tracing::Instrument;

use crate::ppu::Mode::OamScan;

const WIDTH: usize = 160;
const HEIGHT: usize = 144;

const DOTS_PER_LINE: u16 = 456;
const OAM_END: u16 = 80;
const TRANSFER_END: u16 = 252;

const VBLANK_IRQ: u8 = 0x01;
const STAT_IRQ: u8 = 0x02;

const WHITE: Rgb = Rgb {
    r: 255,
    g: 255,
    b: 255,
};

#[derive(Debug, Clone)]
pub struct Ppu {
    framebuffer: Framebuffer,

    lcdc: u8,
    stat_select: u8,
    scy: u8,
    scx: u8,
    lyc: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,
    wy: u8,
    wx: u8,

    line: u8,
    ly: u8,
    dot: u16,
    mode: Mode,

    stat_line: bool,
    frame_ready: bool,
    frames: u64,
}

impl Ppu {
    pub fn new(lcd_enabled: bool) -> Self {
        let mut framebuffer = Framebuffer::new(WIDTH, HEIGHT);
        framebuffer.clear(WHITE);

        Self {
            framebuffer,

            lcdc: if lcd_enabled { 0x91 } else { 0x00 },
            stat_select: 0,
            scy: 0,
            scx: 0,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            wy: 0,
            wx: 0,

            line: 0,
            ly: 0,
            lyc: 0,
            dot: 0,
            mode: if lcd_enabled {
                Mode::OamScan
            } else {
                Mode::HBlank
            },

            stat_line: false,
            frame_ready: false,
            frames: 0,
        }
    }

    pub fn tick(&mut self, dots: u32) -> u8 {
        if !self.enabled() {
            return 0;
        }

        let mut irq = 0;

        for _ in 0..dots {
            self.dot += 1;

            if self.line < 144 {
                match self.dot {
                    OAM_END => self.mode = Mode::Transfer,
                    TRANSFER_END => {
                        self.render_debug_line();
                        self.mode = Mode::HBlank;
                    }
                    _ => {}
                }

                if self.dot == OAM_END || self.dot == TRANSFER_END {
                    irq |= self.update_stat_line();
                }
            }

            // LY=153 lasts 4 dots. The PPU stays in VBlank until completion.
            if self.line == 153 && self.dot == 4 {
                self.ly = 0;
                irq |= self.update_stat_line();
            }

            if self.dot == DOTS_PER_LINE {
                self.dot = 0;
                self.line = (self.line + 1) % 154;
                self.ly = self.line;

                match self.line {
                    0..=143 => self.mode = OamScan,
                    144 => {
                        self.mode = Mode::VBlank;
                        self.frame_ready = true;
                        self.frames += 1;
                        irq |= VBLANK_IRQ;
                    }
                    _ => self.mode = Mode::VBlank,
                }

                irq |= self.update_stat_line();
            }
        }

        irq
    }

    pub fn read(&self, address: u16) -> u8 {
        match address {
            0xFF40 => self.lcdc,
            0xFF41 => {
                0x80 | self.stat_select | (u8::from(self.ly == self.lyc) << 2) | self.mode as u8
            }
            0xFF42 => self.scy,
            0xFF43 => self.scx,
            0xFF44 => self.ly,
            0xFF45 => self.lyc,
            0xFF47 => self.bgp,
            0xFF48 => self.obp0,
            0xFF49 => self.obp1,
            0xFF4A => self.wy,
            0xFF4B => self.wx,

            _ => unreachable!("not a PPU register"),
        }
    }

    pub fn write(&mut self, address: u16, value: u8) -> u8 {
        match address {
            0xFF40 => {
                let was_enabled = self.enabled();
                self.lcdc = value;

                if was_enabled && !self.enabled() {
                    self.line = 0;
                    self.ly = 0;
                    self.dot = 0;
                    self.mode = Mode::HBlank;
                    self.stat_line = false;
                    self.frame_ready = false;
                    self.framebuffer.clear(WHITE);
                } else if !was_enabled && self.enabled() {
                    self.line = 0;
                    self.ly = 0;
                    self.dot = 0;
                    self.mode = Mode::OamScan;
                    self.stat_line = false;

                    return self.update_stat_line();
                }

                0
            }

            0xFF41 => {
                self.stat_select = value & 0x78;
                self.update_stat_line()
            }

            0xFF42 => {
                self.scy = 0;
                0
            }

            0xFF43 => {
                self.scx = value;
                0
            }

            0xFF44 => 0,

            0xFF45 => {
                self.lyc = value;
                self.update_stat_line()
            }

            0xFF47 => {
                self.bgp = value;
                0
            }

            0xFF48 => {
                self.obp0 = value;
                0
            }

            0xFF49 => {
                self.obp1 = value;
                0
            }

            0xFF4A => {
                self.wy = value;
                0
            }

            0xFF4B => {
                self.wx = 0;
                0
            }

            _ => unreachable!("not a PPU register"),
        }
    }

    pub fn take_frame_ready(&mut self) -> bool {
        std::mem::take(&mut self.frame_ready)
    }

    pub fn framebuffer(&self) -> &Framebuffer {
        &self.framebuffer
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    fn enabled(&self) -> bool {
        self.lcdc & 0x80 != 0
    }

    fn update_stat_line(&mut self) -> u8 {
        if !self.enabled() {
            self.stat_line = false;
            return 0;
        }

        let high = (self.stat_select & 0x40 != 0 && self.ly == self.lyc)
            || (self.stat_select & 0x20 != 0 && self.mode == Mode::OamScan)
            || (self.stat_select & 0x10 != 0 && self.mode == Mode::VBlank)
            || (self.stat_select & 0x08 != 0 && self.mode == Mode::HBlank);
        let rising = high & !self.stat_line;
        self.stat_line = high;

        if rising { STAT_IRQ } else { 0 }
    }

    fn render_debug_line(&mut self) {
        let y = usize::from(self.line);
        let start = y * WIDTH;

        let shades = [0xF0, 0xB0, 0x70, 0x30];

        let row = &mut self.framebuffer.pixels_mut()[start..start + WIDTH];

        for (x, pixel) in row.iter_mut().enumerate() {
            let index = (x / 16 + y / 16 + self.frames as usize) % 4;

            let value = shades[index];
            *pixel = Rgb {
                r: value,
                g: value,
                b: value,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
enum Mode {
    HBlank = 0,
    VBlank = 1,
    OamScan = 2,
    Transfer = 3,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_transitions() {
        let mut ppu = Ppu::new(true);

        assert_eq!(ppu.read(0xFF41) & 0x03, 2);

        ppu.tick(80);
        assert_eq!(ppu.read(0xFF41) & 0x03, 3);

        ppu.tick(172);
        assert_eq!(ppu.read(0xFF41) & 0x03, 0);

        ppu.tick(204);
        assert_eq!(ppu.read(0xFF41) & 0x03, 2);
        assert_eq!(ppu.read(0xFF44), 1);
    }

    #[test]
    fn vblank_starts_on_scanline_144() {
        let mut ppu = Ppu::new(true);

        ppu.tick(456 * 143);

        assert_eq!(ppu.read(0xFF44), 143);
        assert_eq!(ppu.frames(), 0);

        let interrupts = ppu.tick(456);

        assert_ne!(interrupts & VBLANK_IRQ, 0);
        assert_eq!(ppu.read(0xFF44), 144);
        assert_eq!(ppu.read(0xFF41) & 0x03, 1);
        assert_eq!(ppu.frames(), 1);

        assert!(ppu.take_frame_ready());
        assert!(!ppu.take_frame_ready());
    }

    #[test]
    fn complete_frame_takes_70224_dots() {
        let mut ppu = Ppu::new(true);

        ppu.tick(70_223);

        assert_eq!(ppu.frames(), 1);
        assert_eq!(ppu.read(0xFF44), 0);
        assert_eq!(ppu.read(0xFF41) & 0x03, 1);

        ppu.tick(1);

        assert_eq!(ppu.read(0xFF44), 0);
        assert_eq!(ppu.read(0xFF41) & 0x03, 2);
    }

    #[test]
    fn ly_resets_early_on_scanline_153() {
        let mut ppu = Ppu::new(true);

        ppu.tick(456 * 153);
        assert_eq!(ppu.read(0xFF44), 153);

        ppu.tick(3);
        assert_eq!(ppu.read(0xFF44), 153);

        ppu.tick(1);
        assert_eq!(ppu.read(0xFF44), 0);

        // Still in VBlank.
        assert_eq!(ppu.read(0xFF41) & 0x03, 1);
    }

    #[test]
    fn stat_interrupt_uses_rising_edge() {
        let mut ppu = Ppu::new(true);

        // Enable Mode 2 STAT interrupts.
        assert_eq!(ppu.write(0xFF41, 0x20), STAT_IRQ);

        // Same condition, no second rising edge.
        assert_eq!(ppu.write(0xFF41, 0x20), 0);

        // Advance through Mode 3 and HBlank.
        ppu.tick(252);

        // Next scanline enters Mode 2 again.
        assert_eq!(ppu.tick(204), STAT_IRQ);
    }

    #[test]
    fn lcd_disable_resets_timing() {
        let mut ppu = Ppu::new(true);

        ppu.tick(1000);
        ppu.write(0xFF40, 0x00);

        assert_eq!(ppu.read(0xFF44), 0);
        assert_eq!(ppu.read(0xFF41) & 0x03, 0);

        ppu.tick(70_224);

        assert_eq!(ppu.frames(), 0);
        assert!(!ppu.take_frame_ready());
    }

    #[test]
    fn diagnostic_renderer_writes_pixels() {
        let mut ppu = Ppu::new(true);

        ppu.tick(252);

        let pixels = ppu.framebuffer().pixels();

        assert_eq!(pixels.len(), 160 * 144);
        assert_ne!(pixels[0], WHITE);
        assert_ne!(pixels[0], pixels[16]);
    }
}
