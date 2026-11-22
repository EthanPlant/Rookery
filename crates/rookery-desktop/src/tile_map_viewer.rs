use anyhow::{Result, anyhow, ensure};
use sdl3::{
    VideoSubsystem,
    pixels::{Color, PixelFormat},
    render::{FPoint, FRect, ScaleMode, WindowCanvas},
};

const MAP_SIZE: usize = 256;
const MAP_TILES: usize = 32;
const TILE_SIZE: usize = 8;

const GAP: usize = 8;
const SCALE: usize = 2;

const IMAGE_WIDTH: usize = MAP_SIZE * 2 + GAP;
const IMAGE_HEIGHT: usize = MAP_SIZE;

const DISPLAY_WIDTH: usize = IMAGE_WIDTH * SCALE;
const DISPLAY_HEIGHT: usize = IMAGE_HEIGHT * SCALE;
const PANEL_HEIGHT: usize = 64;

const GRAYSCALE: [[u8; 3]; 4] = [
    [0xFF, 0xFF, 0xFF],
    [0xAA, 0xAA, 0xAA],
    [0x55, 0x55, 0x55],
    [0x00, 0x00, 0x00],
];

const DMG_GREEN: [[u8; 3]; 4] = [
    [0x9B, 0xBC, 0x0F],
    [0x8B, 0xAC, 0x0F],
    [0x30, 0x62, 0x30],
    [0x0F, 0x38, 0x0F],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteMode {
    Raw,
    Bgp,
    Green,
}

impl PaletteMode {
    fn next(self) -> Self {
        match self {
            Self::Raw => Self::Bgp,
            Self::Bgp => Self::Green,
            Self::Green => Self::Raw,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Raw => "Raw",
            Self::Bgp => "BGP",
            Self::Green => "DMG Green",
        }
    }

    fn color(self, color_id: u8, bgp: u8) -> [u8; 3] {
        let shade = match self {
            Self::Raw => color_id,
            Self::Bgp | Self::Green => (bgp >> (color_id * 2)) & 0x03,
        };

        match self {
            Self::Green => DMG_GREEN[usize::from(shade)],
            Self::Raw | Self::Bgp => GRAYSCALE[usize::from(shade)],
        }
    }
}

fn tile_data_offset(tile: u8, lcdc: u8) -> usize {
    if lcdc & 0x10 != 0 {
        usize::from(tile) * 16
    } else {
        (0x1000_i32 + i32::from(tile as i8) * 16) as usize
    }
}

fn decode_maps(vram: &[u8; 0x2000], lcdc: u8, bgp: u8, palette: PaletteMode, output: &mut [u8]) {
    assert_eq!(output.len(), IMAGE_WIDTH * IMAGE_HEIGHT * 3,);

    output.fill(0);

    for map in 0..2 {
        let map_base = 0x1800 + map * 0x400;
        let image_x = map * (MAP_SIZE + GAP);

        for tile_y in 0..MAP_TILES {
            for tile_x in 0..MAP_TILES {
                let map_offset = map_base + tile_y * MAP_TILES + tile_x;

                let tile = vram[map_offset];
                let data_base = tile_data_offset(tile, lcdc);

                for pixel_y in 0..TILE_SIZE {
                    let low = vram[data_base + pixel_y * 2];
                    let high = vram[data_base + pixel_y * 2 + 1];

                    for pixel_x in 0..TILE_SIZE {
                        let bit = 7 - pixel_x;

                        let color_id = ((low >> bit) & 1) | (((high >> bit) & 1) << 1);

                        let color = palette.color(color_id, bgp);

                        let x = image_x + tile_x * 8 + pixel_x;
                        let y = tile_y * 8 + pixel_y;

                        let offset = (y * IMAGE_WIDTH + x) * 3;

                        output[offset..offset + 3].copy_from_slice(&color);
                    }
                }
            }
        }
    }
}

fn wrapped_viewport(scx: u8, scy: u8) -> Vec<(u16, u16, u16, u16)> {
    let scx = u16::from(scx);
    let scy = u16::from(scy);

    let first_width = (256 - scx).min(160);
    let second_width = 160 - first_width;

    let first_height = (256 - scy).min(144);
    let second_height = 144 - first_height;

    let xs = [(scx, first_width), (0, second_width)];

    let ys = [(scy, first_height), (0, second_height)];

    let mut rectangles = Vec::with_capacity(4);

    for (y, height) in ys {
        if height == 0 {
            continue;
        }

        for (x, width) in xs {
            if width > 0 {
                rectangles.push((x, y, width, height));
            }
        }
    }

    rectangles
}

/// A read-only window showing both DMG background maps.
pub struct TileMapViewer {
    canvas: WindowCanvas,
    pixels: Vec<u8>,
    palette: PaletteMode,
    visible: bool,

    // Map number, tile column, tile row.
    hovered: Option<(usize, usize, usize)>,
}

impl TileMapViewer {
    pub fn new(video: &VideoSubsystem, dmg_green: bool) -> Result<Self> {
        let window = video
            .window(
                "Rookery - Tile Map Viewer",
                DISPLAY_WIDTH as u32,
                (DISPLAY_HEIGHT + PANEL_HEIGHT) as u32,
            )
            .hidden()
            .build()
            .map_err(|e| anyhow!("{e}"))?;

        Ok(Self {
            canvas: window.into_canvas(),
            pixels: vec![0; IMAGE_WIDTH * IMAGE_HEIGHT * 3],
            palette: if dmg_green {
                PaletteMode::Green
            } else {
                PaletteMode::Bgp
            },
            visible: false,
            hovered: None,
        })
    }

    pub fn window_id(&self) -> u32 {
        self.canvas.window().id()
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn toggle(&mut self) {
        if self.visible {
            self.hide();
        } else {
            self.visible = true;
            let _ = self.canvas.window_mut().show();
        }
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.hovered = None;
        let _ = self.canvas.window_mut().hide();
    }

    pub fn cycle_palette(&mut self) {
        self.palette = self.palette.next();
    }

    pub fn mouse_moved(&mut self, x: f32, y: f32) {
        if x < 0.0 || y < 0.0 || x >= DISPLAY_WIDTH as f32 || y >= DISPLAY_HEIGHT as f32 {
            self.hovered = None;
            return;
        }

        let px = x as usize / SCALE;
        let py = y as usize / SCALE;

        let (map, local_x) = if px < MAP_SIZE {
            (0, px)
        } else if px >= MAP_SIZE + GAP {
            (1, px - MAP_SIZE - GAP)
        } else {
            self.hovered = None;
            return;
        };

        self.hovered = Some((map, local_x / TILE_SIZE, py / TILE_SIZE));
    }

    pub fn mouse_left(&mut self) {
        self.hovered = None;
    }

    pub fn render(
        &mut self,
        vram: &[u8; 0x2000],
        lcdc: u8,
        bgp: u8,
        scx: u8,
        scy: u8,
    ) -> Result<()> {
        if !self.visible {
            return Ok(());
        }

        decode_maps(vram, lcdc, bgp, self.palette, &mut self.pixels);

        let creator = self.canvas.texture_creator();
        let mut texture = creator
            .create_texture_streaming(PixelFormat::RGB24, IMAGE_WIDTH as u32, IMAGE_HEIGHT as u32)
            .map_err(|e| anyhow!("{e}"))?;

        texture.set_scale_mode(ScaleMode::Nearest);

        texture
            .update(None, &self.pixels, IMAGE_WIDTH * 3)
            .map_err(|e| anyhow!("{e}"))?;

        self.canvas.set_draw_color(Color::RGB(20, 20, 20));
        self.canvas.clear();

        self.canvas.copy(
            &texture,
            None,
            Some(FRect::new(
                0.0,
                0.0,
                DISPLAY_WIDTH as f32,
                DISPLAY_HEIGHT as f32,
            )),
        )?;

        self.draw_window_map_outline(lcdc)?;
        self.draw_background_viewport(lcdc, scx, scy)?;
        self.draw_hovered_tile()?;
        self.draw_info(vram, lcdc, scx, scy)?;

        ensure!(
            self.canvas.present(),
            "presenting tile map viewer: {}",
            sdl3::get_error(),
        );

        Ok(())
    }

    fn draw_window_map_outline(&mut self, lcdc: u8) -> Result<()> {
        if lcdc & 0xA1 != 0xA1 {
            // LCD on + Window enabled + DMG BG enabled.
            return Ok(());
        }

        let map = usize::from((lcdc >> 6) & 1);
        let x = (map * (MAP_SIZE + GAP) * SCALE) as f32;

        self.canvas.set_draw_color(Color::RGB(255, 200, 40));

        self.canvas.draw_rect(FRect::new(
            x,
            0.0,
            (MAP_SIZE * SCALE) as f32 - 1.0,
            (MAP_SIZE * SCALE) as f32 - 1.0,
        ))?;

        Ok(())
    }

    fn draw_background_viewport(&mut self, lcdc: u8, scx: u8, scy: u8) -> Result<()> {
        if lcdc & 0x81 != 0x81 {
            return Ok(());
        }

        let map = usize::from((lcdc >> 3) & 1);
        let map_x = (map * (MAP_SIZE + GAP) * SCALE) as f32;

        self.canvas.set_draw_color(Color::RGB(0, 255, 255));

        for (x, y, width, height) in wrapped_viewport(scx, scy) {
            self.canvas.draw_rect(FRect::new(
                map_x + f32::from(x) * SCALE as f32,
                f32::from(y) * SCALE as f32,
                f32::from(width) * SCALE as f32,
                f32::from(height) * SCALE as f32,
            ))?;
        }

        Ok(())
    }

    fn draw_hovered_tile(&mut self) -> Result<()> {
        let Some((map, column, row)) = self.hovered else {
            return Ok(());
        };

        let map_x = map * (MAP_SIZE + GAP);

        let x = (map_x + column * TILE_SIZE) * SCALE;
        let y = row * TILE_SIZE * SCALE;

        self.canvas.set_draw_color(Color::RGB(255, 0, 255));

        self.canvas.draw_rect(FRect::new(
            x as f32,
            y as f32,
            (TILE_SIZE * SCALE) as f32,
            (TILE_SIZE * SCALE) as f32,
        ))?;

        Ok(())
    }

    fn draw_info(&mut self, vram: &[u8; 0x2000], lcdc: u8, scx: u8, scy: u8) -> Result<()> {
        self.canvas.set_draw_color(Color::RGB(240, 240, 240));

        let bg_map = if lcdc & 0x08 != 0 { 0x9C00 } else { 0x9800 };

        let win_enabled = lcdc & 0xA1 == 0xA1;

        let win_map = if lcdc & 0x40 != 0 { 0x9C00 } else { 0x9800 };

        let top = DISPLAY_HEIGHT as f32 + 4.0;

        self.canvas.draw_debug_text(
            &format!(
                "MAP $9800                  MAP $9C00         \
                 F3: hide  P: palette [{}]",
                self.palette.name(),
            ),
            FPoint::new(2.0, top),
        )?;

        let status = if win_enabled {
            format!(
                "BG ${bg_map:04X}  WIN ${win_map:04X}  \
                 SCX={scx} SCY={scy} LCDC=${lcdc:02X}"
            )
        } else {
            format!(
                "BG ${bg_map:04X}  WIN OFF  \
                 SCX={scx} SCY={scy} LCDC=${lcdc:02X}"
            )
        };

        self.canvas
            .draw_debug_text(&status, FPoint::new(2.0, top + 14.0))?;

        self.canvas.draw_debug_text(
            "Cyan: BG viewport   Yellow: window map   Magenta: hovered tile",
            FPoint::new(2.0, top + 28.0),
        )?;

        if let Some((map, column, row)) = self.hovered {
            let map_offset = 0x1800 + map * 0x400 + row * 32 + column;

            let tile = vram[map_offset];
            let pattern_offset = tile_data_offset(tile, lcdc);

            let map_address = 0x8000 + map_offset;
            let tile_address = 0x8000 + pattern_offset;

            let info = format!(
                "Map {} ({column}, {row}) \
                 entry ${map_address:04X}=${tile:02X} \
                 -> tile data ${tile_address:04X}",
                map,
            );

            self.canvas
                .draw_debug_text(&info, FPoint::new(2.0, top + 42.0))?;
        } else {
            self.canvas.draw_debug_text(
                "Hover over a map tile to inspect its entry and tile-data address.",
                FPoint::new(2.0, top + 42.0),
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_addressing_respects_lcdc_bit_four() {
        // Unsigned mode: tile zero starts at $8000.
        assert_eq!(tile_data_offset(0x00, 0x10), 0);

        // Signed mode: tile zero starts at $9000.
        assert_eq!(tile_data_offset(0x00, 0x00), 0x1000);

        // Signed tile -128 starts at $8800.
        assert_eq!(tile_data_offset(0x80, 0x00), 0x0800);

        // Signed tile +127 starts at $97F0.
        assert_eq!(tile_data_offset(0x7F, 0x00), 0x17F0);
    }

    #[test]
    fn both_maps_decode_independently() {
        let mut vram = [0; 0x2000];
        let mut output = vec![0; IMAGE_WIDTH * IMAGE_HEIGHT * 3];

        // Map zero references tile 1.
        vram[0x1800] = 1;

        // Map one references tile 2.
        vram[0x1C00] = 2;

        // Tile 1, first pixel: colour 1.
        vram[0x0010] = 0x80;

        // Tile 2, first pixel: colour 2.
        vram[0x0021] = 0x80;

        decode_maps(&vram, 0x10, 0xE4, PaletteMode::Raw, &mut output);

        // First pixel of the left map.
        assert_eq!(&output[0..3], &[0xAA, 0xAA, 0xAA],);

        // First pixel of the right map.
        let right_offset = (MAP_SIZE + GAP) * 3;

        assert_eq!(&output[right_offset..right_offset + 3], &[0x55, 0x55, 0x55],);
    }

    #[test]
    fn viewport_without_wrapping_is_one_rectangle() {
        assert_eq!(wrapped_viewport(0, 0), vec![(0, 0, 160, 144)]);
    }

    #[test]
    fn viewport_wraps_across_both_edges() {
        let rectangles = wrapped_viewport(200, 220);

        assert_eq!(
            rectangles,
            vec![
                (200, 220, 56, 36),
                (0, 220, 104, 36),
                (200, 0, 56, 108),
                (0, 0, 104, 108),
            ]
        );

        let area: u16 = rectangles
            .iter()
            .map(|&(_, _, width, height)| width * height)
            .sum();

        assert_eq!(area, 160 * 144);
    }

    #[test]
    fn palette_uses_bgp_mapping() {
        assert_eq!(PaletteMode::Raw.color(1, 0x1B), [0xAA, 0xAA, 0xAA],);

        assert_eq!(PaletteMode::Bgp.color(1, 0x1B), [0x55, 0x55, 0x55],);

        assert_eq!(PaletteMode::Green.color(1, 0x1B), [0x30, 0x62, 0x30],);
    }
}
