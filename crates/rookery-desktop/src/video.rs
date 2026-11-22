use std::{
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use gb_core::{CLOCK_HZ, CLOCKS_PER_FRAME, GameBoy};
use rookery_common::{Machine, Rgb};
use sdl3::{
    event::{Event, WindowEvent},
    keyboard::Keycode,
    pixels::{Color, PixelFormat},
    render::{FRect, ScaleMode, WindowCanvas},
};

use crate::{tile_map_viewer::TileMapViewer, tile_viewer::TileViewer};

const WIDTH: u32 = gb_core::SCREEN_WIDTH as u32;
const HEIGHT: u32 = gb_core::SCREEN_HEIGHT as u32;
const INITIAL_SCALE: u32 = 4;

const DMG_GREEN: [[u8; 3]; 4] = [
    [0x9B, 0xBC, 0x0F],
    [0x8B, 0xAC, 0x0F],
    [0x30, 0x62, 0x30],
    [0x0F, 0x38, 0x0F],
];

pub fn play(gb: &mut GameBoy, dmg_green: bool) -> Result<()> {
    let sdl = sdl3::init().map_err(|e| anyhow!("initializing SDL: {e}"))?;

    let video = sdl
        .video()
        .map_err(|e| anyhow!("initializing SDL video: {e}"))?;

    let window = video
        .window("Rookery", WIDTH * INITIAL_SCALE, HEIGHT * INITIAL_SCALE)
        .position_centered()
        .resizable()
        .build()
        .map_err(|e| anyhow!("creating window: {e}"))?;

    let mut canvas = window.into_canvas();
    let texture_creator = canvas.texture_creator();

    let game_window_id = canvas.window().id();
    let mut tile_viewer = TileViewer::new(&video, dmg_green)?;
    let mut tile_map_viewer = TileMapViewer::new(&video, dmg_green)?;

    let mut texture = texture_creator
        .create_texture_streaming(PixelFormat::RGB24, WIDTH, HEIGHT)
        .map_err(|e| anyhow!("creating texture: {e}"))?;
    texture.set_scale_mode(ScaleMode::Nearest);

    let mut events = sdl
        .event_pump()
        .map_err(|e| anyhow!("creating event pump: {e}"))?;

    let frame_duration = Duration::from_secs_f64(CLOCKS_PER_FRAME as f64 / CLOCK_HZ as f64);

    let mut next_deadline = Instant::now();
    let mut next_frame_cycle = gb.virtual_clock();

    'running: loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. } => break 'running,

                Event::Window {
                    window_id,
                    win_event: WindowEvent::CloseRequested,
                    ..
                } if window_id == game_window_id => break 'running,

                Event::Window {
                    window_id,
                    win_event: WindowEvent::CloseRequested,
                    ..
                } if window_id == tile_viewer.window_id() => tile_viewer.hide(),

                Event::Window {
                    window_id,
                    win_event: WindowEvent::CloseRequested,
                    ..
                } if window_id == tile_map_viewer.window_id() => tile_viewer.hide(),

                Event::KeyDown {
                    keycode: Some(Keycode::F2),
                    repeat: false,
                    ..
                } => tile_viewer.toggle(),

                Event::KeyDown {
                    keycode: Some(Keycode::F3),
                    repeat: false,
                    ..
                } => tile_map_viewer.toggle(),

                Event::KeyDown {
                    window_id,
                    keycode: Some(Keycode::P),
                    repeat: false,
                    ..
                } if window_id == tile_viewer.window_id() => tile_viewer.cycle_palette(),

                Event::KeyDown {
                    window_id,
                    keycode: Some(Keycode::P),
                    repeat: false,
                    ..
                } if window_id == tile_map_viewer.window_id() => tile_map_viewer.cycle_palette(),

                Event::KeyDown {
                    window_id,
                    keycode: Some(Keycode::Escape),
                    ..
                } if window_id == tile_viewer.window_id() => tile_viewer.hide(),

                Event::KeyDown {
                    window_id,
                    keycode: Some(Keycode::Escape),
                    ..
                } if window_id == tile_map_viewer.window_id() => tile_viewer.hide(),

                Event::KeyDown {
                    window_id,
                    keycode: Some(Keycode::Escape),
                    ..
                } if window_id == game_window_id => break 'running,

                Event::MouseMotion {
                    window_id, x, y, ..
                } if window_id == tile_viewer.window_id() => tile_viewer.mouse_moved(x, y),

                Event::MouseMotion {
                    window_id, x, y, ..
                } if window_id == tile_map_viewer.window_id() => tile_map_viewer.mouse_moved(x, y),

                Event::Window {
                    window_id,
                    win_event: WindowEvent::MouseLeave,
                    ..
                } if window_id == tile_viewer.window_id() => tile_viewer.mouse_left(),

                Event::Window {
                    window_id,
                    win_event: WindowEvent::MouseLeave,
                    ..
                } if window_id == tile_map_viewer.window_id() => tile_map_viewer.mouse_left(),

                _ => {}
            }
        }

        next_frame_cycle = next_frame_cycle
            .checked_add(CLOCKS_PER_FRAME)
            .context("frame cycle counter overflow")?;
        let mut frame_ready = gb.take_frame_ready();

        while gb.virtual_clock() < next_frame_cycle {
            gb.step_instruction()?;
            frame_ready |= gb.take_frame_ready();
        }

        if tile_viewer.is_visible() {
            tile_viewer.render(gb.debug_vram(), gb.debug_bgp())?;
        }

        if tile_map_viewer.is_visible() {
            tile_map_viewer.render(
                gb.debug_vram(),
                gb.debug_lcdc(),
                gb.debug_bgp(),
                gb.debug_scx(),
                gb.debug_scy(),
            )?;
        }

        texture
            .with_lock(None, |pixels, pitch| {
                let source = gb.framebuffer().pixels();

                for (y, row) in source
                    .as_chunks::<{ WIDTH as usize }>()
                    .0
                    .iter()
                    .enumerate()
                {
                    let start = y * pitch;

                    let destination = &mut pixels[start..start + WIDTH as usize * 3];

                    for (dst, pixel) in destination
                        .as_chunks_mut::<3>()
                        .0
                        .iter_mut()
                        .zip(row.iter())
                    {
                        dst.copy_from_slice(&display_color(pixel, dmg_green));
                    }
                }
            })
            .map_err(|e| anyhow!("updating texture: {e}"))?;

        if frame_ready {
            present(&mut canvas, &texture)?;
        }

        next_deadline += frame_duration;

        let now = Instant::now();
        if next_deadline >= now {
            thread::sleep(next_deadline - now);
        } else {
            next_deadline = now;
        }
    }

    Ok(())
}

fn present(canvas: &mut WindowCanvas, texture: &sdl3::render::Texture<'_>) -> Result<()> {
    let (output_width, output_height) = canvas
        .output_size()
        .map_err(|e| anyhow!("querying output size: {e}"))?;
    if output_width == 0 || output_height == 0 {
        return Ok(());
    }

    let destination = destination_rect(output_width, output_height);

    canvas.set_draw_color(Color::RGB(0, 0, 0));
    canvas.clear();

    canvas
        .copy(texture, None, Some(destination))
        .map_err(|e| anyhow!("drawing texture: {e}"))?;
    canvas.present();
    Ok(())
}

fn display_color(pixel: &Rgb, dmg_green: bool) -> [u8; 3] {
    let original = [pixel.r, pixel.g, pixel.b];

    if !dmg_green {
        return original;
    }

    match pixel.r {
        0xFF => DMG_GREEN[0],
        0xAA => DMG_GREEN[1],
        0x55 => DMG_GREEN[2],
        0x00 => DMG_GREEN[3],
        _ => original,
    }
}

fn destination_rect(width: u32, height: u32) -> FRect {
    let integer_scale = (width / WIDTH).min(height / HEIGHT);

    let scale = if integer_scale > 0 {
        integer_scale as f32
    } else {
        (width as f32 / WIDTH as f32).min(height as f32 / HEIGHT as f32)
    };

    let display_width = WIDTH as f32 * scale;
    let display_height = HEIGHT as f32 * scale;

    let x = (width as f32 - display_width) / 2.0;
    let y = (height as f32 / display_height) / 2.0;

    FRect::new(x, y, display_width, display_height)
}
