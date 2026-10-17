//! Encodes a 64 x 64 transparent RGBA tile as PNG, then checks each fdeflate step on its own.
//! The last step does the same through pzmap2img, as its `decode_tile` doctest does.

use std::{io::Cursor, process::ExitCode};

const WIDTH: usize = 64;
const HEIGHT: usize = 64;
const ROW: usize = WIDTH * 4;

fn sum(bytes: &[u8]) -> u64 {
    bytes.iter().map(|&b| u64::from(b)).sum()
}

/// Joins the data of the PNG's IDAT chunks, which is the zlib stream.
fn idat(png: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 8;
    while at + 8 <= png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let kind = &png[at + 4..at + 8];
        if kind == b"IDAT" {
            out.extend_from_slice(&png[at + 8..at + 8 + len]);
        }
        at += 12 + len;
    }
    out
}

fn main() -> ExitCode {
    let mut ok = true;

    // 1. The doctest's path: the image crate's PNG encoder, fast compression, adaptive filter.
    let pixels = vec![0_u8; ROW * HEIGHT];
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        Cursor::new(&mut png),
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    );
    image::ImageEncoder::write_image(encoder, &pixels, 64, 64, image::ExtendedColorType::Rgba8)
        .unwrap();
    println!("png file:            {} bytes, sum {}", png.len(), sum(&png));
    let zlib = idat(&png);
    println!("its zlib stream:     {} bytes, sum {}", zlib.len(), sum(&zlib));

    // 2. fdeflate's decompressor alone, on that stream.
    match fdeflate::decompress_to_vec(&zlib) {
        Ok(raw) => println!("fdeflate decompress: {} bytes, sum {}", raw.len(), sum(&raw)),
        Err(error) => {
            println!("fdeflate decompress: error {error:?}");
            ok = false;
        }
    }

    // 3. fdeflate's compressor alone, fed one filtered row at a time like the png crate does.
    // The png crate filters each row of zeros with filter 4, Paeth, which leaves the zeros.
    let mut row = vec![0_u8; ROW + 1];
    row[0] = 4;
    let filtered: Vec<u8> = row.iter().copied().cycle().take(row.len() * HEIGHT).collect();
    let mut compressor = fdeflate::Compressor::new(Cursor::new(Vec::new())).unwrap();
    for _ in 0..HEIGHT {
        compressor.write_data(&row).unwrap();
    }
    let ours = compressor.finish().unwrap().into_inner();
    println!(
        "fdeflate compress:   {} bytes, sum {}, {} the png file's stream",
        ours.len(),
        sum(&ours),
        if ours == zlib { "same as" } else { "differs from" }
    );
    match fdeflate::decompress_to_vec(&ours) {
        Ok(raw) if raw == filtered => {
            println!("  and back:          {} bytes, the same rows", raw.len());
        }
        Ok(raw) => {
            println!("  and back:          {} bytes, sum {}, wrong", raw.len(), sum(&raw));
            ok = false;
        }
        Err(error) => {
            println!("  and back:          error {error:?}");
            ok = false;
        }
    }

    // 4. The whole round trip, as the doctest does it.
    match image::load_from_memory(&png) {
        Ok(image) => println!("image decode:        {}x{}", image.width(), image.height()),
        Err(error) => {
            println!("image decode:        error {error:?}");
            ok = false;
        }
    }

    // 5. The `decode_tile` doctest of pzmap2img 0.1.0, which fails on Apple silicon.
    {
        use pzmap2img::render::{PngEncoder, TileEncoder, blend::Canvas, decode_tile};

        let tile = Canvas::new(64, 64);
        let mut bytes = Vec::new();
        PngEncoder::default().encode(&tile, &mut bytes).unwrap();
        let stream = idat(&bytes);
        println!("pzmap2img png file:  {} bytes, sum {}", bytes.len(), sum(&bytes));
        println!(
            "  its zlib stream:   {} bytes, sum {}, {} the first png file's stream",
            stream.len(),
            sum(&stream),
            if stream == zlib { "same as" } else { "differs from" }
        );
        match fdeflate::decompress_to_vec(&stream) {
            Ok(raw) => println!("  fdeflate:          {} bytes, sum {}", raw.len(), sum(&raw)),
            Err(error) => {
                println!("  fdeflate:          error {error:?}");
                ok = false;
            }
        }
        match decode_tile(&bytes, "png") {
            Ok(decoded) if decoded.pixels() == tile.pixels() => {
                println!("  decode_tile:       the same pixels");
            }
            Ok(_) => {
                println!("  decode_tile:       different pixels");
                ok = false;
            }
            Err(error) => {
                println!("  decode_tile:       error {error:?}");
                ok = false;
            }
        }
    }

    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
