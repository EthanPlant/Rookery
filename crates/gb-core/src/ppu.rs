use rookery_common::{Framebuffer, Rgb};

use crate::ppu::Mode::OamScan;

/// Native Game Boy display width, in pixels.
pub const WIDTH: usize = 160;
/// Native Game Boy display height, in pixels.
pub const HEIGHT: usize = 144;

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

const DMG_SHADES: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];

#[derive(Debug, Clone)]
pub struct Ppu {
    framebuffer: Framebuffer,

    vram: [u8; 0x2000],
    bg_color_ids: [u8; WIDTH],
    oam: [u8; 0xA0],

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

            vram: [0xFF; 0x2000],
            bg_color_ids: [0; WIDTH],
            oam: [0xFF; 0xA0],

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
                        self.render_background_line();
                        self.render_objects_line();
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
                self.scy = value;
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

    pub fn read_vram(&self, address: u16) -> u8 {
        self.vram[usize::from(address - 0x8000)]
    }

    pub fn write_vram(&mut self, address: u16, value: u8) {
        self.vram[usize::from(address - 0x8000)] = value;
    }

    pub fn read_oam(&self, address: u16) -> u8 {
        self.oam[usize::from(address - 0xFE00)]
    }

    pub fn write_oam(&mut self, address: u16, value: u8) {
        self.oam[usize::from(address) - 0xFE00] = value;
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

    fn render_background_line(&mut self) {
        let y = usize::from(self.line);

        if y >= HEIGHT {
            return;
        }

        let start = y * WIDTH;
        let row = &mut self.framebuffer.pixels_mut()[start..start + WIDTH];

        // On DMG, LCDC bit 0 disables both the BG and window.
        // Background pixels appear white when disabled.
        if self.lcdc & 0x01 == 0 {
            row.fill(WHITE);
        }

        let map_base = if self.lcdc & 0x08 != 0 {
            0x1C00
        } else {
            0x1800
        };
        let unsigned_tiles = self.lcdc & 0x10 != 0;
        let bg_y = self.scy.wrapping_add(self.line);

        let tile_row = usize::from(bg_y / 8);
        let pixel_y = usize::from(bg_y % 8);

        for (x, pixel) in row.iter_mut().enumerate() {
            let bg_x = self.scx.wrapping_add(x as u8);

            let tile_column = usize::from(bg_x / 8);
            let pixel_x = bg_x % 8;

            let map_offset = map_base + tile_row * 32 + tile_column;
            let tile_id = self.vram[map_offset];

            let tile_base = if unsigned_tiles {
                usize::from(tile_id) * 16
            } else {
                let signed_id = i32::from(tile_id as i8);

                (0x1000 + signed_id * 16) as usize
            };

            let data_offset = tile_base + pixel_y * 2;

            let low = self.vram[data_offset];
            let high = self.vram[data_offset + 1];

            let shift = 7 - pixel_x;
            let low_bit = (low >> shift) & 1;
            let high_bit = (high >> shift) & 1;

            let color_id = low_bit | (high_bit << 1);
            self.bg_color_ids[x] = color_id; // Preserve the original 2bpp colour index for sprite priority.

            let shade_id = (self.bgp >> (color_id * 2)) & 0x03;
            let shade = DMG_SHADES[usize::from(shade_id)];

            *pixel = Rgb {
                r: shade,
                g: shade,
                b: shade,
            };
        }
    }

    fn render_objects_line(&mut self) {
        if self.line >= 144 || self.lcdc & 0x02 == 0 {
            return;
        }

        let height = if self.lcdc & 0x04 != 0 { 16 } else { 8 };
        let line = i16::from(self.line);

        // OAM scan selects the first ten objects who fall on this scanline.
        let mut selected = [0; 10];
        let mut count = 0;

        for index in 0..40 {
            let base = index * 4;
            let top = i16::from(self.oam[base]) - 16;

            if line >= top && line < top + height {
                selected[count] = index;
                count += 1;

                if count == selected.len() {
                    break;
                }
            }
        }

        selected[..count].sort_unstable_by_key(|&index| (self.oam[index * 4 + 1], index));

        let row_start = usize::from(self.line) * WIDTH;

        for screen_x in 0..WIDTH {
            let x = screen_x as i16;

            for &index in &selected[..count] {
                let base = index * 4;

                let obj_y = i16::from(self.oam[base]) - 16;
                let obj_x = i16::from(self.oam[base + 1]) - 8;
                let mut tile = self.oam[base + 2];
                let flags = self.oam[base + 3];

                let local_x = x - obj_x;
                if !(0..8).contains(&local_x) {
                    continue;
                }

                let mut local_y = line - obj_y;

                // Y-flip mirrors the entire object.
                if flags & 0x40 != 0 {
                    local_y = height - 1 - local_y;
                }

                if height == 16 {
                    tile &= 0xFE;
                    tile += (local_y / 8) as u8;
                }

                let tile_row = (local_y % 8) as usize;

                let data_offset = usize::from(tile) * 16 + tile_row * 2;
                let low = self.vram[data_offset];
                let high = self.vram[data_offset + 1];

                // X flip
                let bit = if flags & 0x20 != 0 {
                    local_x as u8
                } else {
                    7 - local_x as u8
                };

                let colour_id = ((low >> bit) & 1) | (((high >> bit) & 1) << 1);
                // Object colour index 0 is transparent.
                if colour_id == 0 {
                    continue;
                }

                let behind_bg = flags & 0x80 != 0;
                let bg_opaque = self.lcdc & 0x01 != 0 && self.bg_color_ids[screen_x] != 0;

                if !(behind_bg && bg_opaque) {
                    let palette = if flags & 0x10 != 0 {
                        self.obp1
                    } else {
                        self.obp0
                    };

                    let shade_id = (palette >> (colour_id * 2)) & 0x03;
                    let shade = DMG_SHADES[usize::from(shade_id)];

                    self.framebuffer.pixels_mut()[row_start + screen_x] = Rgb {
                        r: shade,
                        g: shade,
                        b: shade,
                    }
                }

                // A non-transparent object pixel has won priority.
                break;
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

    fn sprite_test_ppu() -> Ppu {
        let mut ppu = Ppu::new(true);

        ppu.vram.fill(0);

        // LCD enabled, BG enabled, OBJ enabled, 8x8 sprites,
        // unsigned background tile addressing.
        ppu.write(0xFF40, 0x93);

        // Identity grayscale palettes.
        ppu.write(0xFF47, 0xE4);
        ppu.write(0xFF48, 0xE4);
        ppu.write(0xFF49, 0xE4);

        ppu
    }

    fn write_object(ppu: &mut Ppu, index: usize, y: u8, x: u8, tile: u8, flags: u8) {
        let address = 0xFE00 + (index as u16) * 4;

        ppu.write_oam(address, y);
        ppu.write_oam(address + 1, x);
        ppu.write_oam(address + 2, tile);
        ppu.write_oam(address + 3, flags);
    }

    fn render_line(ppu: &mut Ppu) {
        ppu.render_background_line();
        ppu.render_objects_line();
    }

    fn pixel(ppu: &Ppu, x: usize, y: usize) -> Rgb {
        ppu.framebuffer().pixels()[y * WIDTH + x]
    }

    fn grey(value: u8) -> Rgb {
        Rgb {
            r: value,
            g: value,
            b: value,
        }
    }

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
    fn object_pixel_zero_is_transparent() {
        let mut ppu = sprite_test_ppu();

        // Tile 1, first row: colour 1 followed by zeros.
        ppu.write_vram(0x8010, 0x80);

        // Object at screen coordinates (0, 0).
        write_object(&mut ppu, 0, 16, 8, 1, 0);

        render_line(&mut ppu);

        assert_eq!(pixel(&ppu, 0, 0), grey(0xAA));
        assert_eq!(pixel(&ppu, 1, 0), grey(0xFF));
    }

    #[test]
    fn object_flip_mirrors_both_axes() {
        let mut ppu = sprite_test_ppu();

        // Only the first pixel of tile 1's last row is set.
        ppu.write_vram(0x801E, 0x80);

        // Flip horizontally and vertically.
        write_object(&mut ppu, 0, 16, 8, 1, 0x60);

        render_line(&mut ppu);

        assert_eq!(pixel(&ppu, 0, 0), grey(0xFF));
        assert_eq!(pixel(&ppu, 7, 0), grey(0xAA));
    }

    #[test]
    fn sixteen_pixel_objects_use_tile_pairs() {
        let mut ppu = sprite_test_ppu();

        // Enable 8x16 objects.
        ppu.write(0xFF40, 0x97);

        // Tile 2: colour 1 in first pixel.
        ppu.write_vram(0x8020, 0x80);

        // Tile 3: colour 2 in first pixel.
        ppu.write_vram(0x8031, 0x80);

        // Odd tile index must be rounded down to 2.
        write_object(&mut ppu, 0, 16, 8, 3, 0);

        render_line(&mut ppu);
        assert_eq!(pixel(&ppu, 0, 0), grey(0xAA));

        ppu.line = 8;
        render_line(&mut ppu);

        assert_eq!(pixel(&ppu, 0, 8), grey(0x55));
    }

    #[test]
    fn only_first_ten_objects_on_line_are_selected() {
        let mut ppu = sprite_test_ppu();

        ppu.write_vram(0x8010, 0x80);

        // Ten invisible objects at X=0. They still count
        // against the scanline limit.
        for index in 0..10 {
            write_object(&mut ppu, index, 16, 0, 1, 0);
        }

        // Eleventh object would otherwise be visible.
        write_object(&mut ppu, 10, 16, 8, 1, 0);

        render_line(&mut ppu);

        assert_eq!(pixel(&ppu, 0, 0), grey(0xFF));
    }

    #[test]
    fn smaller_x_coordinate_wins_on_dmg() {
        let mut ppu = sprite_test_ppu();

        // Tile 1 contains colour 1 across the row.
        ppu.write_vram(0x8010, 0xFF);

        // Tile 2 contains colour 2 across the row.
        ppu.write_vram(0x8021, 0xFF);

        // First OAM object starts at screen X=8.
        write_object(&mut ppu, 0, 16, 16, 1, 0);

        // Second object starts at screen X=4.
        write_object(&mut ppu, 1, 16, 12, 2, 0);

        render_line(&mut ppu);

        // Both overlap X=8, but the second wins.
        assert_eq!(pixel(&ppu, 8, 0), grey(0x55));
    }

    #[test]
    fn object_priority_uses_raw_bg_colour_id() {
        let mut ppu = sprite_test_ppu();

        // BG colour 2 at X=0; BG colour 0 at X=1.
        ppu.write_vram(0x8001, 0x80);

        // Object colour 1 at X=0 and X=1.
        ppu.write_vram(0x8010, 0xC0);

        // OBJ behind nonzero BG colours.
        write_object(&mut ppu, 0, 16, 8, 1, 0x80);

        render_line(&mut ppu);

        // BG wins at X=0.
        assert_eq!(pixel(&ppu, 0, 0), grey(0x55));

        // OBJ wins where BG colour index is zero.
        assert_eq!(pixel(&ppu, 1, 0), grey(0xAA));
    }

    #[test]
    fn object_can_use_second_palette() {
        let mut ppu = sprite_test_ppu();

        ppu.write_vram(0x8010, 0x80);

        // In OBP1=$1B, colour index 1 maps to shade 2.
        ppu.write(0xFF49, 0x1B);

        write_object(&mut ppu, 0, 16, 8, 1, 0x10);

        render_line(&mut ppu);

        assert_eq!(pixel(&ppu, 0, 0), grey(0x55));
    }
}
