use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

use gb_core::{SCREEN_HEIGHT, SCREEN_WIDTH};
use rookery_common::{Rgb, rom::Rom};
use rookery_test::acid2::{DEFAULT_MAX_CYCLES, run_dmg_acid2};

const PIXEL_COUNT: usize = SCREEN_WIDTH * SCREEN_HEIGHT;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/public")
        .join(name)
}

fn load_reference(path: &Path) -> Vec<Rgb> {
    let file = File::open(path).unwrap_or_else(|e| panic!("opening {}: {e}", path.display()));

    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);

    let mut reader = decoder.read_info().expect("reference has a valid header");
    let size = reader
        .output_buffer_size()
        .expect("reference size doesn't overflow");

    let mut buffer = vec![0; size];
    let info = reader
        .next_frame(&mut buffer)
        .expect("reference can be decoded");
    assert_eq!(
        (info.width, info.height),
        (SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32),
        "reference image has incorrect dimensions"
    );
    assert_eq!(
        info.bit_depth,
        png::BitDepth::Eight,
        "reference must decode to 8-bit channels"
    );

    let bytes = &buffer[..info.buffer_size()];

    let pixels = match info.color_type {
        png::ColorType::Rgb => bytes
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| Rgb {
                r: p[0],
                g: p[1],
                b: p[2],
            })
            .collect::<Vec<_>>(),
        png::ColorType::Rgba => bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                assert_eq!(p[3], 255, "unexpected transparent pixel");

                Rgb {
                    r: p[0],
                    g: p[1],
                    b: p[2],
                }
            })
            .collect(),
        png::ColorType::Grayscale => bytes.iter().map(|&v| Rgb { r: v, g: v, b: v }).collect(),
        png::ColorType::GrayscaleAlpha => bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                assert_eq!(p[1], 255, "unexpected transparent pixel");
                Rgb {
                    r: p[0],
                    g: p[0],
                    b: p[0],
                }
            })
            .collect(),
        png::ColorType::Indexed => panic!("indexed RGB was not expanded"),
    };

    assert_eq!(pixels.len(), PIXEL_COUNT);

    pixels
}

fn write_ppm(path: &Path, pixels: &[Rgb]) -> std::io::Result<()> {
    assert_eq!(pixels.len(), PIXEL_COUNT);

    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    write!(writer, "P6\n{SCREEN_WIDTH} {SCREEN_HEIGHT}\n255\n").unwrap();
    for pixel in pixels {
        writer.write_all(&[pixel.r, pixel.g, pixel.b])?;
    }

    writer.flush()
}

#[test]
fn dmg_acid2_matches_reference() {
    let rom_path = fixture_path("dmg-acid2.gb");
    let reference_path = fixture_path("reference-dmg.png");

    assert!(
        rom_path.is_file(),
        "missing dmg-acid2 fixture: {}",
        rom_path.display()
    );

    let rom = Rom::load(&rom_path).unwrap();
    let report = run_dmg_acid2(rom, DEFAULT_MAX_CYCLES).unwrap();

    let expected = load_reference(&reference_path);
    let actual = &report.pixels;

    assert_eq!(actual.len(), PIXEL_COUNT);

    let mut mismatches = 0;
    let mut first_mismatch = None;

    let mut diff = vec![
        Rgb {
            r: 255,
            g: 255,
            b: 255
        };
        PIXEL_COUNT
    ];

    for (index, (expected_pixel, actual_pixel)) in expected.iter().zip(actual.iter()).enumerate() {
        if expected_pixel == actual_pixel {
            continue;
        }

        mismatches += 1;

        if first_mismatch.is_none() {
            first_mismatch = Some((
                index % SCREEN_WIDTH,
                index / SCREEN_WIDTH,
                *expected_pixel,
                *actual_pixel,
            ));
        }

        diff[index] = Rgb {
            r: 255,
            g: 0,
            b: 255,
        };
    }

    if mismatches > 0 {
        let artifact_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/rookery-test/dmg-acid2");
        fs::create_dir_all(&artifact_dir).expect("artifact dir can be created");

        let actual_path = artifact_dir.join("actual.ppm");
        let diff_path = artifact_dir.join("diff.ppm");

        write_ppm(&actual_path, actual).unwrap();
        write_ppm(&diff_path, &diff).unwrap();

        let (x, y, expected_pixel, actual_pixel) = first_mismatch.expect("nonzero mismatches");
        panic!(
            "dmg-acid2 FAILED\n\
            {mismatches}/{PIXEL_COUNT} pixels differ\n\
            First difference: ({x}, {y})\n\
            Expected: {expected_pixel:?}\n\
            Actual: {actual_pixel:?}"
        );
    }

    eprintln!("dmg-acid2 PASSED");
}
