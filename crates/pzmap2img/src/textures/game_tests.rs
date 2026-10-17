//! Texture tests against a real Project Zomboid install.
//!
//! Each test compares the library's sprites with a reference computed separately, from the page
//! images as `image` decodes them. The tests are ignored by default. To run them:
//!
//! ```text
//! cargo test -p pzmap2img --release --lib textures::game_tests -- --ignored --nocapture
//! ```
//!
//! They need `PZ_INSTALL_DIR` set to a Build 42 install, and panic with that instruction when it
//! isn't set.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use super::{
    LoadedPack,
    PlantsConfig,
    build::anchor,
    build_texture_library,
    plants::plant_mapping,
};
use crate::{
    budget::{
        BuildBudget,
        BuildCtx,
    },
    config::{
        BuildLimits,
        InputTrust,
    },
    string_table::{
        StringTableBuilder,
        TileNames,
    },
    testing::install_dir,
};
use image::GenericImageView as _;
use libpzmap::texturepack::{
    Texture,
    TexturePack,
};
use std::collections::HashMap;

fn texture_packs() -> std::path::PathBuf {
    install_dir().join("media/texturepacks")
}

/// Checks every sprite of one pack against `image`'s crop of the page, premultiplied in
/// floating point.
#[test]
#[ignore = "requires an installed copy of the game"]
fn extracted_sprites_match_a_cropped_reference() -> anyhow::Result<()> {
    let pack = LoadedPack::open(
        texture_packs().join("Tiles2x.floor.pack"),
        InputTrust::Trusted,
        &BuildLimits::default(),
    )?;

    let names = StringTableBuilder::<TileNames>::new();
    let parsed = TexturePack::parse(pack.bytes()).map_err(|e| anyhow::anyhow!("{e}"))?;
    for page in &parsed.pages {
        for texture in &page.textures {
            names.intern(texture.name)?;
        }
    }
    let reader = names.into_reader();

    // No vegetation config, so no sprites are combined.
    let (library, stats) = build_texture_library(
        &reader,
        std::slice::from_ref(&pack),
        None,
        BuildCtx::new(&BuildLimits::default(), &BuildBudget::unlimited()),
    )?;
    println!(
        "{} sprites, {:.1} MB, {} pages",
        stats.textures,
        stats.blob_bytes as f64 / 1e6,
        stats.pages_decoded
    );

    let mut compared = 0_usize;
    let mut retrimmed = 0_usize;

    for page in &parsed.pages {
        let reference = image::load_from_memory(page.png)?.to_rgba8();

        for texture in &page.textures {
            let Some(key) = reader.resolve_string(texture.name) else {
                continue;
            };
            let Some(sprite) = library.get(key) else {
                continue;
            };

            let cropped = reference.view(
                texture.x as u32,
                texture.y as u32,
                texture.width as u32,
                texture.height as u32,
            );

            let (anchor_x, anchor_y) = anchor(texture);
            let shift_x = (sprite.offset_x - anchor_x) as u32;
            let shift_y = (sprite.offset_y - anchor_y) as u32;
            if shift_x != 0 || shift_y != 0 || sprite.width != texture.width as u32 {
                retrimmed += 1;
            }

            for y in 0..sprite.height {
                for x in 0..sprite.width {
                    let src = cropped.get_pixel(x + shift_x, y + shift_y).0;
                    let got = &sprite.pixels[((y * sprite.width + x) * 4) as usize..][..4];

                    let alpha = f64::from(src[3]) / 255.0;
                    for channel in 0..3 {
                        let want = (f64::from(src[channel]) * alpha).round() as i32;
                        let diff = (i32::from(got[channel]) - want).abs();
                        assert!(
                            diff <= 1,
                            "{} px({x},{y}) ch{channel}: got {} want {want}",
                            texture.name,
                            got[channel]
                        );
                    }
                    assert_eq!(got[3], src[3], "{} alpha at ({x},{y})", texture.name);
                }
            }

            compared += 1;
        }
    }

    println!("compared {compared} sprites, {retrimmed} needed a tighter trim than the pack's");
    assert!(
        compared > 400,
        "expected the whole floor pack, got {compared}"
    );
    Ok(())
}

const CW: i32 = 384;
const CH: i32 = 512;

/// Combines the expected vegetation sprite in floating point from `image`'s crops. Returns it
/// with the box around its visible pixels.
fn reference_composite(
    sources: &[(Texture<'_>, &image::RgbaImage)],
) -> Option<(Vec<f64>, i32, i32, i32, i32)> {
    let mut canvas = vec![0.0_f64; (CW * CH * 4) as usize];

    for (texture, page) in sources {
        let (ax, ay) = anchor(texture);
        let (dx, dy) = (CW / 2 + ax, CH + ay);

        for row in 0..texture.height {
            let y = dy + row;
            if !(0..CH).contains(&y) {
                continue;
            }
            for col in 0..texture.width {
                let x = dx + col;
                if !(0..CW).contains(&x) {
                    continue;
                }

                let src = page
                    .get_pixel((texture.x + col) as u32, (texture.y + row) as u32)
                    .0;
                let sa = f64::from(src[3]) / 255.0;
                let d = ((y * CW + x) * 4) as usize;

                for channel in 0..3 {
                    let s = f64::from(src[channel]) / 255.0 * sa;
                    canvas[d + channel] = s + canvas[d + channel] * (1.0 - sa);
                }
                canvas[d + 3] = sa + canvas[d + 3] * (1.0 - sa);
            }
        }
    }

    let (mut x0, mut y0, mut x1, mut y1) = (CW, CH, 0, 0);
    for y in 0..CH {
        for x in 0..CW {
            if canvas[((y * CW + x) * 4 + 3) as usize] > 0.0 {
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
            }
        }
    }

    (x1 > x0 && y1 > y0).then_some((canvas, x0, y0, x1, y1))
}

/// Checks the combined vegetation sprites against a reference combined in floating point.
#[test]
#[ignore = "requires an installed copy of the game"]
fn composites_match_a_float_reference() -> anyhow::Result<()> {
    let config = PlantsConfig::default();
    let mapping = plant_mapping(&config);

    // Only the vegetation names are added, so the library holds only combined sprites.
    let names = StringTableBuilder::<TileNames>::new();
    for (key, _) in &mapping {
        names.intern(key)?;
    }
    let reader = names.into_reader();

    let pack = LoadedPack::open(
        texture_packs().join("Tiles2x.pack"),
        InputTrust::Trusted,
        &BuildLimits::default(),
    )?;
    let (library, stats) = build_texture_library(
        &reader,
        std::slice::from_ref(&pack),
        Some(&config),
        BuildCtx::new(&BuildLimits::default(), &BuildBudget::unlimited()),
    )?;

    println!(
        "{} composites, {} missing, {:.2} MB",
        stats.composited,
        stats.missing,
        stats.blob_bytes as f64 / 1e6
    );
    assert_eq!(stats.composited, 98, "every mapping key should compose");
    assert_eq!(stats.missing, 0, "no mapping key should be left blank");

    let parsed = TexturePack::parse(pack.bytes()).map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut located = HashMap::new();
    for (page_ix, page) in parsed.pages.iter().enumerate() {
        for texture in &page.textures {
            located.insert(texture.name, (page_ix, *texture));
        }
    }

    let mut decoded: HashMap<usize, image::RgbaImage> = HashMap::new();

    let mut checked = 0_usize;
    let mut worst = 0_i32;

    for (key, sources) in &mapping {
        for name in sources {
            let (page_ix, _) = located[name.as_str()];
            if let std::collections::hash_map::Entry::Vacant(slot) = decoded.entry(page_ix) {
                slot.insert(image::load_from_memory(parsed.pages[page_ix].png)?.to_rgba8());
            }
        }

        let refs: Vec<(Texture<'_>, &image::RgbaImage)> = sources
            .iter()
            .map(|name| {
                let (page_ix, texture) = located[name.as_str()];
                (texture, &decoded[&page_ix])
            })
            .collect();

        let key_id = reader.resolve_string(key).unwrap();
        let sprite = library.get(key_id).unwrap_or_else(|| panic!("{key} blank"));

        let (canvas, x0, y0, x1, y1) =
            reference_composite(&refs).unwrap_or_else(|| panic!("{key} reference blank"));

        assert_eq!(sprite.width as i32, x1 - x0, "{key}: width");
        assert_eq!(sprite.height as i32, y1 - y0, "{key}: height");
        assert_eq!(sprite.offset_x, x0 - CW / 2, "{key}: offset_x");
        assert_eq!(sprite.offset_y, y0 - CH, "{key}: offset_y");

        for y in 0..sprite.height as i32 {
            for x in 0..sprite.width as i32 {
                let r = (((y + y0) * CW + (x + x0)) * 4) as usize;
                let g = ((y * sprite.width as i32 + x) * 4) as usize;

                for channel in 0..4 {
                    let want = (canvas[r + channel] * 255.0).round() as i32;
                    let got = i32::from(sprite.pixels[g + channel]);
                    let diff = (got - want).abs();
                    worst = worst.max(diff);
                    assert!(
                        diff <= 2,
                        "{key} px({x},{y}) ch{channel}: got {got} want {want}"
                    );
                }
            }
        }

        checked += 1;
    }

    println!("checked {checked} composites, worst channel error {worst}");
    assert_eq!(checked, 98);
    Ok(())
}
