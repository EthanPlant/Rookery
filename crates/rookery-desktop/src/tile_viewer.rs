use anyhow::{Result, anyhow, ensure};
use sdl3::{
    VideoSubsystem,
    pixels::{Color, PixelFormat},
    render::{FPoint, FRect, ScaleMode, WindowCanvas},
};

const TILE_SIZE: usize = 8;
const TILE_BYTES: usize = 16;
const TILE_COUNT: usize = 384;

const COLUMNS: usize = 16;
const ROWS: usize = 24;

const ATLAS_WIDTH: usize = COLUMNS * TILE_SIZE;
const ATLAS_HEIGHT: usize = ROWS * TILE_SIZE;

const SCALE: usize = 3;
const DISPLAY_WIDTH: usize = ATLAS_WIDTH * SCALE;
const DISPLAY_HEIGHT: usize = ATLAS_HEIGHT * SCALE;

const PANEL_HEIGHT: usize = 40;

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

    fn label(self) -> &'static str {
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

fn decode_atlas(vram: &[u8; 0x2000], bgp: u8, palette: PaletteMode, output: &mut [u8]) {
    assert_eq!(output.len(), ATLAS_WIDTH * ATLAS_HEIGHT * 3,);

    for tile in 0..TILE_COUNT {
        let tile_x = (tile % COLUMNS) * TILE_SIZE;
        let tile_y = (tile / COLUMNS) * TILE_SIZE;

        for y in 0..TILE_SIZE {
            let source = tile * TILE_BYTES + y * 2;

            let low = vram[source];
            let high = vram[source + 1];

            for x in 0..TILE_SIZE {
                let bit = 7 - x;

                let color_id = ((low >> bit) & 1) | (((high >> bit) & 1) << 1);

                let color = palette.color(color_id, bgp);

                let output_x = tile_x + x;
                let output_y = tile_y + y;

                let offset = (output_y * ATLAS_WIDTH + output_x) * 3;

                output[offset..offset + 3].copy_from_slice(&color);
            }
        }
    }
}

pub struct TileViewer {
    canvas: WindowCanvas,
    pixels: Vec<u8>,
    palette: PaletteMode,
    hovered: Option<usize>,
    visible: bool,
}

impl TileViewer {
    /// Construct the viewer initially hidden.
    pub fn new(video: &VideoSubsystem, dmg_green: bool) -> Result<Self> {
        let window = video
            .window(
                "Rookery - VRAM Tile Viewer",
                DISPLAY_WIDTH as u32,
                (DISPLAY_HEIGHT + PANEL_HEIGHT) as u32,
            )
            .hidden()
            .build()
            .map_err(|e| anyhow!("{e}"))?;

        Ok(Self {
            canvas: window.into_canvas(),
            pixels: vec![0; ATLAS_WIDTH * ATLAS_HEIGHT * 3],
            palette: if dmg_green {
                PaletteMode::Green
            } else {
                PaletteMode::Bgp
            },
            hovered: None,
            visible: false,
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
        self.hovered =
            if x >= 0.0 && y >= 0.0 && x < DISPLAY_WIDTH as f32 && y < DISPLAY_HEIGHT as f32 {
                let column = x as usize / (TILE_SIZE * SCALE);
                let row = y as usize / (TILE_SIZE * SCALE);

                Some(row * COLUMNS + column)
            } else {
                None
            };
    }

    pub fn mouse_left(&mut self) {
        self.hovered = None;
    }

    pub fn render(&mut self, vram: &[u8; 0x2000], bgp: u8) -> Result<()> {
        if !self.visible {
            return Ok(());
        }

        decode_atlas(vram, bgp, self.palette, &mut self.pixels);

        let creator = self.canvas.texture_creator();

        let mut texture = creator
            .create_texture_streaming(PixelFormat::RGB24, ATLAS_WIDTH as u32, ATLAS_HEIGHT as u32)
            .map_err(|e| anyhow!("{e}"))?;

        texture.set_scale_mode(ScaleMode::Nearest);

        texture
            .update(None, &self.pixels, ATLAS_WIDTH * 3)
            .map_err(|e| anyhow!("{e}"))?;

        self.canvas.set_draw_color(Color::RGB(25, 25, 25));
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

        // Highlight the tile being inspected.
        if let Some(tile) = self.hovered {
            let column = tile % COLUMNS;
            let row = tile / COLUMNS;

            self.canvas.set_draw_color(Color::RGB(255, 0, 255));

            self.canvas.draw_rect(FRect::new(
                (column * TILE_SIZE * SCALE) as f32,
                (row * TILE_SIZE * SCALE) as f32,
                (TILE_SIZE * SCALE) as f32,
                (TILE_SIZE * SCALE) as f32,
            ))?;
        }

        self.render_inspector(vram)?;

        ensure!(
            self.canvas.present(),
            "presenting VRAM tile viewer: {}",
            sdl3::get_error(),
        );

        Ok(())
    }

    fn render_inspector(&mut self, vram: &[u8; 0x2000]) -> Result<()> {
        self.canvas.set_draw_color(Color::RGB(230, 230, 230));

        let heading = format!("F2: hide  P: palette [{}]", self.palette.label(),);

        self.canvas
            .draw_debug_text(&heading, FPoint::new(2.0, (DISPLAY_HEIGHT + 2) as f32))?;

        if let Some(tile) = self.hovered {
            let offset = tile * TILE_BYTES;
            let address = 0x8000 + offset;

            let details = format!("Tile {:03} (${address:04X})", tile,);

            self.canvas
                .draw_debug_text(&details, FPoint::new(2.0, (DISPLAY_HEIGHT + 13) as f32))?;

            let bytes = vram[offset..offset + TILE_BYTES]
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(" ");

            self.canvas
                .draw_debug_text(&bytes, FPoint::new(2.0, (DISPLAY_HEIGHT + 24) as f32))?;
        } else {
            self.canvas.draw_debug_text(
                "Hover over a tile to inspect its data.",
                FPoint::new(2.0, (DISPLAY_HEIGHT + 13) as f32),
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_both_bitplanes() {
        let mut vram = [0; 0x2000];
        let mut output = vec![0; ATLAS_WIDTH * ATLAS_HEIGHT * 3];

        vram[0] = 0b1000_0000;
        vram[1] = 0b0100_0000;

        decode_atlas(&vram, 0xE4, PaletteMode::Raw, &mut output);

        assert_eq!(&output[0..3], &[0xAA, 0xAA, 0xAA]);
        assert_eq!(&output[3..6], &[0x55, 0x55, 0x55]);
        assert_eq!(&output[6..9], &[0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn uses_physical_vram_tile_order() {
        let mut vram = [0; 0x2000];
        let mut output = vec![0; ATLAS_WIDTH * ATLAS_HEIGHT * 3];

        // First pixel of tile 383, at $97F0.
        vram[383 * TILE_BYTES] = 0x80;

        decode_atlas(&vram, 0xE4, PaletteMode::Raw, &mut output);

        let x = 15 * TILE_SIZE;
        let y = 23 * TILE_SIZE;
        let offset = (y * ATLAS_WIDTH + x) * 3;

        assert_eq!(&output[offset..offset + 3], &[0xAA, 0xAA, 0xAA],);
    }

    #[test]
    fn palette_modes_are_distinct() {
        assert_eq!(PaletteMode::Raw.color(1, 0x1B), [0xAA, 0xAA, 0xAA],);

        assert_eq!(PaletteMode::Bgp.color(1, 0x1B), [0x55, 0x55, 0x55],);

        assert_eq!(PaletteMode::Green.color(1, 0x1B), [0x30, 0x62, 0x30],);
    }
}
