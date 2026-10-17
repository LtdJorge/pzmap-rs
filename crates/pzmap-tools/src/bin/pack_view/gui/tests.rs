//! Headless tests of the window, run through egui's accessibility tree without a GPU.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;
use egui_kittest::{
    Harness,
    kittest::Queryable,
};
use std::{
    io::Cursor,
    path::PathBuf,
};

/// Writes a two-page `.pack` to a temporary file and returns its path. Page 0 holds
/// `grass_01` and `wall_01`, page 1 holds `grass_02` and `roof_01`.
fn fixture(test: &str) -> PathBuf {
    let pages: [(&str, [&str; 2]); 2] = [
        ("page0", ["grass_01", "wall_01"]),
        ("page1", ["grass_02", "roof_01"]),
    ];

    let mut png = Vec::new();
    image::RgbaImage::from_pixel(64, 64, image::Rgba([200, 50, 50, 255]))
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();

    let mut bytes = Vec::new();
    let u32_le = |bytes: &mut Vec<u8>, value: usize| {
        bytes.extend_from_slice(&u32::try_from(value).unwrap().to_le_bytes());
    };
    let string = |bytes: &mut Vec<u8>, text: &str| {
        u32_le(bytes, text.len());
        bytes.extend_from_slice(text.as_bytes());
    };

    bytes.extend_from_slice(b"PZPK");
    u32_le(&mut bytes, 1);
    u32_le(&mut bytes, pages.len());
    for (page, textures) in pages {
        string(&mut bytes, page);
        u32_le(&mut bytes, textures.len());
        u32_le(&mut bytes, 1);
        for (index, name) in textures.iter().enumerate() {
            string(&mut bytes, name);
            // x, y, width, height, offset x, offset y, original width, original height.
            for field in [index * 32, 0, 32, 32, 0, 0, 32, 32] {
                u32_le(&mut bytes, field);
            }
        }
        u32_le(&mut bytes, png.len());
        bytes.extend_from_slice(&png);
    }

    let path = std::env::temp_dir().join(format!("pack-view-{}-{test}.pack", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    path
}

fn harness(path: &Path) -> Harness<'static, PackView> {
    harness_for(PackView::open(path, None))
}

/// Shows `app` with the window's font installed.
///
/// The harness draws its first frames before the font can be installed, and egui has no font of
/// its own here. So the harness starts on an empty window, and `app` replaces it once the font
/// is in place. Otherwise windows such as the picker keep the sizes they had without any text.
fn harness_for(app: PackView) -> Harness<'static, PackView> {
    let mut harness = Harness::builder()
        .with_size([1000.0, 700.0])
        .build_ui_state(
            |ui, app: &mut PackView| app.show(ui),
            PackView::empty(String::new()),
        );
    install_fonts(&harness.ctx);
    harness.run();
    *harness.state_mut() = app;
    harness.run();
    harness
}

fn shows(harness: &Harness<'_, PackView>, label: &str) -> bool {
    harness.query_all_by_label(label).next().is_some()
}

#[test]
fn searching_filters_the_texture_list() {
    let path = fixture("search");
    let mut harness = harness(&path);

    assert!(shows(&harness, "grass_01"));
    assert!(shows(&harness, "wall_01"));
    assert!(!shows(&harness, "grass_02"));

    // `/` focuses the search box, and typing narrows the list to matches from every page.
    harness.event(Event::Text("/".to_owned()));
    harness.run();
    harness.event(Event::Text("grass".to_owned()));
    harness.run();

    assert_eq!(harness.state().query, "grass");
    assert!(shows(&harness, "grass_01"));
    assert!(shows(&harness, "grass_02"));
    assert!(!shows(&harness, "wall_01"));
    assert!(!shows(&harness, "roof_01"));

    std::fs::remove_file(&path).unwrap();
}

#[test]
fn keys_move_the_selection_and_cycle_the_background() {
    let path = fixture("keys");
    let mut harness = harness(&path);

    harness.key_press(Key::ArrowDown);
    harness.run();
    harness.event(Event::Text("b".to_owned()));
    harness.run();

    let viewer = harness.state().viewer.as_ref().unwrap();
    assert_eq!(viewer.texture, 1);
    assert_eq!(viewer.view, View::Texture);
    assert_eq!(viewer.background, Background::Black);
    // The selection is shown in the info panel.
    assert!(shows(&harness, "32,0 32x32"));

    std::fs::remove_file(&path).unwrap();
}

#[test]
fn a_missing_path_opens_an_empty_window_that_explains_itself() {
    let mut harness = harness(Path::new("/nonexistent/pack-view-test"));
    harness.run();
    assert!(harness.state().viewer.is_none());
    assert!(
        harness
            .query_all_by_label_contains("File menu")
            .next()
            .is_some()
    );
}

#[test]
fn fit_mode_magnifies_a_small_page_by_a_whole_factor() {
    let path = fixture("fit");
    let harness = harness(&path);

    let geometry = harness.state().page_geometry.unwrap();
    let scale = geometry.scale;
    assert_eq!(scale.fract(), 0.0, "scale {scale} is not a whole factor");
    assert!(scale >= 1.0);
    // The 64-pixel page fills the shorter side as far as a whole factor allows.
    let side = geometry.area.width().min(geometry.area.height());
    assert!(64.0 * scale <= side);
    assert!(64.0 * (scale + 1.0) > side || scale == zoom::MAX_FIT as f32);
    assert_eq!(
        geometry.image.size(),
        egui::vec2(64.0 * scale, 64.0 * scale)
    );
    assert!(geometry.area.contains_rect(geometry.image));

    std::fs::remove_file(&path).unwrap();
}

#[test]
fn the_outline_follows_the_selected_sprite_at_every_zoom() {
    let path = fixture("outline");
    let mut harness = harness(&path);
    // `wall_01` sits at 32,0 and is 32x32.
    harness.state_mut().viewer.as_mut().unwrap().texture = 1;

    for factor in [1, 2] {
        harness.state_mut().page_zoom = PageZoom::fixed(factor);
        harness.run();
        let geometry = harness.state().page_geometry.unwrap();
        let scale = factor as f32;
        assert_eq!(geometry.scale, scale);
        let expected = egui::Rect::from_min_size(
            geometry.image.min + egui::vec2(32.0 * scale, 0.0),
            egui::vec2(32.0 * scale, 32.0 * scale),
        );
        assert_eq!(geometry.outline, Some(expected), "at {factor}x");
    }

    std::fs::remove_file(&path).unwrap();
}

#[test]
fn several_installs_open_a_chooser_and_picking_one_opens_it() {
    let root = std::env::temp_dir().join(format!("pack-view-{}-installs", std::process::id()));
    let installs: Vec<Install> = ["first", "second"]
        .iter()
        .map(|name| {
            let dir = root.join(name);
            let packs = dir.join("media/texturepacks");
            std::fs::create_dir_all(&packs).unwrap();
            let pack = fixture(&format!("install-{name}"));
            std::fs::rename(&pack, packs.join(format!("{name}.pack"))).unwrap();
            Install {
                dir,
                source: crate::install::Source::SteamLibrary,
            }
        })
        .collect();

    let mut harness = harness_for(PackView::choose(installs, None));
    assert!(harness.state().viewer.is_none());
    assert!(shows(&harness, "Choose a Project Zomboid install"));

    let second = format!("{}  (Steam library)", root.join("second").display());
    harness.get_by_label(&second).click();
    harness.run();

    let viewer = harness.state().viewer.as_ref().unwrap();
    assert_eq!(viewer.pack.name, "second.pack");
    assert!(harness.state().picker.is_none());

    std::fs::remove_dir_all(&root).unwrap();
}
