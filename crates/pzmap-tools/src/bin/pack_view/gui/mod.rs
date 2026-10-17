//! The window frontend, built with egui.
//!
//! egui measures sizes in points. A point covers one or more physical pixels, depending on the
//! display's scale factor.

mod textures;
mod zoom;

use crate::{
    Start,
    install::{
        Install,
        Places,
    },
    model::{
        self,
        View,
        Viewer,
        Zoom,
    },
    pack::TextureRef,
    render::{
        self,
        Background,
    },
};
use anyhow::{
    Result,
    anyhow,
};
use eframe::egui::{
    self,
    Color32,
    Event,
    Key,
    RichText,
};
use std::{
    path::Path,
    sync::Arc,
};
use textures::PageTextures;
use zoom::PageZoom;

const TITLE: &str = "pack-view";

/// Instructions for opening a pack by hand. The window shows them whenever nothing is open.
const HOW_TO_OPEN: &str =
    "Open a .pack file or a folder of them from the File menu, or drop one on this window.";

/// Fira Code, the window's font. `assets/fonts/OFL.txt` holds its license.
const FIRA_CODE: &[u8] = include_bytes!("../../../../assets/fonts/FiraCode-Regular.ttf");

/// Opens a window showing `start`, and runs until the window is closed.
///
/// If the path cannot be opened, the window still opens and shows the error. Someone who
/// double-clicked the exe then sees what went wrong.
pub fn run(start: Start, pack_name: Option<String>) -> Result<()> {
    let app = match start {
        Start::Open(path) => PackView::open(&path, pack_name.as_deref()),
        Start::Choose(installs) => PackView::choose(installs, pack_name),
        Start::Missing(missing) => PackView::empty(format!("{missing}. {HOW_TO_OPEN}")),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_app_id(TITLE)
            .with_inner_size([1280.0, 800.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        TITLE,
        options,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow!("{error}"))
}

/// Makes Fira Code the font for all text.
///
/// egui's own fonts are not built in, so Fira Code must have every glyph the window draws. A
/// glyph it lacks is drawn as a question mark.
fn install_fonts(ctx: &egui::Context) {
    const NAME: &str = "fira-code";
    let mut fonts = egui::FontDefinitions::empty();
    fonts.font_data.insert(
        NAME.to_owned(),
        Arc::new(egui::FontData::from_static(FIRA_CODE)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.insert(family, vec![NAME.to_owned()]);
    }
    ctx.set_fonts(fonts);
}

/// Which list the arrow keys move.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Focus {
    Pages,
    Textures,
}

/// What a picker window lists.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum PickerKind {
    /// The packs in the open pack's folder.
    Pack,
    /// The installed copies of the game in `PackView::installs`.
    Install,
}

/// An open picker window and its highlighted row.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Picker {
    kind: PickerKind,
    selected: usize,
}

/// Where the page view drew the page and the selected sprite's outline, in points.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PageGeometry {
    /// The space the page view had.
    pub area: egui::Rect,
    /// The whole page as drawn.
    pub image: egui::Rect,
    /// Physical pixels per page pixel.
    pub scale: f32,
    /// The box the outline is drawn around, outside the sprite's pixels.
    pub outline: Option<egui::Rect>,
}

/// One thing a key press asks for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Command {
    Move(isize),
    First,
    Last,
    FocusList(Focus),
    Show,
    Back,
    FocusSearch,
    ToggleView,
    CycleBackground,
    PickPack,
    CyclePack(isize),
    ZoomBy(i32),
    SetZoom(Zoom),
    StepMatch(isize),
}

/// The window's state: the shared viewer plus what only a window needs.
pub struct PackView {
    viewer: Option<Viewer>,
    /// Why nothing is open, or why the last open failed.
    error: Option<String>,
    focus: Focus,
    query: String,
    /// The pack generation and query that `viewer.matches` holds results for.
    filtered: Option<(u64, String)>,
    picker: Option<Picker>,
    /// The installs found by the last search.
    installs: Vec<Install>,
    /// The `--pack` name, applied to the first install picked.
    pack_name: Option<String>,
    page_zoom: PageZoom,
    /// Where the last frame drew the page view.
    page_geometry: Option<PageGeometry>,
    /// The page, sprite and zoom the page view last scrolled to.
    page_scrolled: Option<(u64, usize, usize, PageZoom)>,
    pages: PageTextures,
    /// Set when the keyboard moved a selection, so its list scrolls to it.
    reveal: bool,
    title: String,
}

impl PackView {
    /// Opens `path`, or keeps the error to show in the window.
    pub fn open(path: &Path, pack_name: Option<&str>) -> Self {
        match Viewer::open(path, pack_name) {
            Ok(viewer) => Self::new(Some(viewer), None),
            Err(error) => Self::empty(format!("{error:#}. {HOW_TO_OPEN}")),
        }
    }

    /// Opens with nothing loaded, showing `message`.
    pub fn empty(message: String) -> Self {
        Self::new(None, Some(message))
    }

    /// Opens with nothing loaded and the install picker listing `installs`.
    pub fn choose(installs: Vec<Install>, pack_name: Option<String>) -> Self {
        let mut view = Self::new(None, None);
        view.installs = installs;
        view.pack_name = pack_name;
        view.picker = Some(Picker {
            kind: PickerKind::Install,
            selected: 0,
        });
        view
    }

    fn new(viewer: Option<Viewer>, error: Option<String>) -> Self {
        Self {
            viewer,
            error,
            focus: Focus::Textures,
            query: String::new(),
            filtered: None,
            picker: None,
            installs: Vec::new(),
            pack_name: None,
            page_zoom: PageZoom::Fit,
            page_geometry: None,
            page_scrolled: None,
            pages: PageTextures::default(),
            reveal: false,
            title: String::new(),
        }
    }

    /// Searches for installs again and lists them, or says where it looked.
    fn pick_install(&mut self) {
        match Places::detect().find() {
            Ok(installs) => {
                let selected = self
                    .viewer
                    .as_ref()
                    .and_then(|v| {
                        installs
                            .iter()
                            .position(|i| v.pack.path.starts_with(&i.dir))
                    })
                    .unwrap_or(0);
                self.installs = installs;
                self.picker = Some(Picker {
                    kind: PickerKind::Install,
                    selected,
                });
            }
            Err(missing) => self.error = Some(format!("{missing}. {HOW_TO_OPEN}")),
        }
    }

    /// Opens the install at `index` of the last search.
    fn open_install(&mut self, index: usize) {
        if let Some(path) = self.installs.get(index).map(Install::texturepacks) {
            let pack_name = self.pack_name.take();
            self.open_named(&path, pack_name.as_deref());
        }
    }

    /// Replaces the open pack with `path`, keeping the current one if that fails.
    fn open_path(&mut self, path: &Path) {
        self.open_named(path, None);
    }

    fn open_named(&mut self, path: &Path, pack_name: Option<&str>) {
        match Viewer::open(path, pack_name) {
            Ok(viewer) => {
                self.viewer = Some(viewer);
                self.error = None;
                self.query.clear();
                self.filtered = None;
                self.picker = None;
                self.focus = Focus::Textures;
                // A new viewer starts its generation count from zero, so the cache could mistake
                // the new pack's pages for the old one's.
                self.pages.clear();
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }

    /// Draws one frame.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.take_dropped_file(&ctx);
        for command in self.read_keys(&ctx) {
            self.apply(command, &ctx);
        }
        self.refresh_filter();
        self.update_title(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("lists")
            .default_size(320.0)
            .show(ui, |ui| self.lists(ui));
        egui::Panel::right("info")
            .default_size(260.0)
            .show(ui, |ui| self.info(ui));
        egui::CentralPanel::default().show(ui, |ui| self.preview(ui));
        self.picker_window(&ctx);
        self.reveal = false;
    }

    fn take_dropped_file(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .first()
                .map(|file| file.path().to_path_buf())
        });
        if let Some(path) = dropped {
            self.open_path(&path);
        }
    }

    /// Takes the key presses this viewer handles out of the frame's input, so the search box
    /// does not also receive them.
    fn read_keys(&mut self, ctx: &egui::Context) -> Vec<Command> {
        let search_focused = ctx.memory(|memory| memory.has_focus(search_id()));
        let typing = ctx.egui_wants_keyboard_input();
        let picking = self.picker.is_some();
        let mut commands = Vec::new();

        ctx.input_mut(|input| {
            input.events.retain(|event| {
                let command = match event {
                    Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.is_none() || modifiers.shift_only() => {
                        key_command(*key, typing, search_focused, picking)
                    }
                    Event::Text(text) if !typing && !picking => text_command(text),
                    _ => None,
                };
                if let Some(command) = command {
                    commands.push(command);
                    false
                } else {
                    true
                }
            });
        });
        commands
    }

    fn apply(&mut self, command: Command, ctx: &egui::Context) {
        if command == Command::FocusSearch {
            ctx.memory_mut(|memory| memory.request_focus(search_id()));
            return;
        }
        if let Some(picker) = self.picker {
            self.apply_to_picker(command, picker);
            return;
        }
        let Some(viewer) = &mut self.viewer else {
            return;
        };
        viewer.status = None;
        self.error = None;
        let step = match command {
            Command::Move(delta) => Some(delta),
            Command::First => Some(isize::MIN),
            Command::Last => Some(isize::MAX),
            _ => None,
        };
        if let Some(delta) = step {
            self.move_selection(delta);
            return;
        }
        match command {
            Command::FocusList(focus) => self.focus = focus,
            Command::Show => viewer.view = View::Texture,
            Command::Back => viewer.view = View::Page,
            Command::ToggleView => viewer.toggle_view(),
            Command::CycleBackground => viewer.cycle_background(),
            Command::PickPack => {
                if !viewer.siblings.is_empty() {
                    self.picker = Some(Picker {
                        kind: PickerKind::Pack,
                        selected: viewer.current_sibling,
                    });
                }
            }
            Command::CyclePack(delta) => {
                viewer.cycle_pack(delta);
            }
            // In the page view, zoom scales the page rather than switching to the sprite.
            Command::ZoomBy(delta) if viewer.view == View::Page => {
                let current = self.page_geometry.map_or(1.0, |g| g.scale);
                self.page_zoom = PageZoom::step(current, delta);
            }
            Command::SetZoom(zoom) if viewer.view == View::Page => {
                self.page_zoom = match zoom {
                    Zoom::Fit => PageZoom::Fit,
                    Zoom::Fixed(factor) => PageZoom::fixed(factor),
                };
            }
            Command::ZoomBy(delta) => viewer.zoom_by(delta),
            Command::SetZoom(zoom) => {
                viewer.zoom = zoom;
                viewer.view = View::Texture;
            }
            Command::StepMatch(delta) => {
                if viewer.step_match(delta) {
                    self.focus = Focus::Textures;
                    self.reveal = true;
                }
            }
            Command::Move(_) | Command::First | Command::Last | Command::FocusSearch => {}
        }
    }

    fn apply_to_picker(&mut self, command: Command, picker: Picker) {
        let rows = match picker.kind {
            PickerKind::Pack => self.viewer.as_ref().map_or(0, |v| v.siblings.len()),
            PickerKind::Install => self.installs.len(),
        };
        let last = rows.saturating_sub(1);
        let select = |selected| Some(Picker { selected, ..picker });
        match command {
            Command::Move(delta) => {
                self.picker = select(picker.selected.saturating_add_signed(delta).min(last));
            }
            Command::First => self.picker = select(0),
            Command::Last => self.picker = select(last),
            Command::Show => self.choose_row(picker.kind, picker.selected),
            Command::Back | Command::PickPack => self.picker = None,
            _ => {}
        }
    }

    /// Closes the picker and opens its row `index`.
    fn choose_row(&mut self, kind: PickerKind, index: usize) {
        self.picker = None;
        match kind {
            PickerKind::Pack => {
                if let Some(viewer) = &mut self.viewer {
                    viewer.switch_pack(index);
                }
            }
            PickerKind::Install => self.open_install(index),
        }
    }

    /// Moves the focused list by `delta`, clamped to its ends.
    fn move_selection(&mut self, delta: isize) {
        let Some(viewer) = &mut self.viewer else {
            return;
        };
        self.reveal = true;
        match self.focus {
            Focus::Pages => {
                let last = viewer.pack.page_count().saturating_sub(1);
                let next = viewer.page.saturating_add_signed(delta).min(last);
                if viewer.select_page(next) {
                    viewer.view = View::Page;
                }
            }
            Focus::Textures if self.query.is_empty() => {
                let last = viewer.texture_count().saturating_sub(1);
                let next = viewer.texture.saturating_add_signed(delta).min(last);
                viewer.select_texture(next);
                viewer.view = View::Texture;
            }
            Focus::Textures => {
                let last = viewer.matches.len().saturating_sub(1);
                let next = match current_match(viewer) {
                    Some(at) => at.saturating_add_signed(delta).min(last),
                    None => 0,
                };
                if let Some(found) = viewer.matches.get(next).copied() {
                    viewer.match_cursor = next;
                    viewer.select(found);
                }
            }
        }
    }

    /// Recomputes the search results when the query or the pack changed.
    fn refresh_filter(&mut self) {
        let Some(viewer) = &mut self.viewer else {
            return;
        };
        let wanted = (viewer.generation, self.query.clone());
        if self.filtered.as_ref() == Some(&wanted) {
            return;
        }
        viewer.last_query.clone_from(&self.query);
        viewer.matches = viewer.pack.search(&self.query);
        viewer.match_cursor = current_match(viewer).unwrap_or(0);
        self.filtered = Some(wanted);
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = self.viewer.as_ref().map_or_else(
            || TITLE.to_owned(),
            |v| format!("{} - {TITLE}", v.pack.name),
        );
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open pack…").clicked() {
                    ui.close();
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Texture pack", &["pack"])
                        .pick_file()
                    {
                        self.open_path(&path);
                    }
                }
                if ui.button("Open folder…").clicked() {
                    ui.close();
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.open_path(&path);
                    }
                }
                let has_siblings = self.viewer.as_ref().is_some_and(|v| !v.siblings.is_empty());
                if ui
                    .add_enabled(has_siblings, egui::Button::new("Switch pack…  (p)"))
                    .clicked()
                {
                    ui.close();
                    self.picker = self.viewer.as_ref().map(|v| Picker {
                        kind: PickerKind::Pack,
                        selected: v.current_sibling,
                    });
                }
                if ui.button("Choose install…").clicked() {
                    ui.close();
                    self.pick_install();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            if let Some(viewer) = &self.viewer {
                ui.separator();
                ui.label(RichText::new(&viewer.pack.name).strong());
                ui.label(format!(
                    "{} pages, {} textures",
                    viewer.pack.page_count(),
                    viewer.pack.texture_count()
                ));
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let message = self
            .error
            .as_deref()
            .or_else(|| self.viewer.as_ref().and_then(|v| v.status.as_deref()));
        match message {
            Some(message) => {
                ui.label(RichText::new(message).color(ui.visuals().warn_fg_color));
            }
            None => {
                ui.weak(
                    "/ search · ↑↓ move · ←→ pages/textures · space page/sprite · +/- zoom · \
                     b background · p pack · drop a .pack here to open it",
                );
            }
        }
    }

    fn lists(&mut self, ui: &mut egui::Ui) {
        let search = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .id(search_id())
                .hint_text("Search textures  ( / )")
                .desired_width(f32::INFINITY),
        );
        if search.changed() {
            self.refresh_filter();
        }
        // Enter jumps to the first match, as it does in the terminal.
        if search.lost_focus()
            && ui.input(|input| input.key_pressed(Key::Enter))
            && let Some(viewer) = &mut self.viewer
            && !self.query.is_empty()
            && viewer.run_search(&self.query)
        {
            self.focus = Focus::Textures;
            self.reveal = true;
        }

        let Some(viewer) = &mut self.viewer else {
            ui.add_space(8.0);
            ui.weak("No pack open.");
            return;
        };

        let accent = ui.visuals().selection.bg_fill;
        let heading = |text: String, focused: bool| {
            let text = RichText::new(text).strong();
            if focused { text.color(accent) } else { text }
        };

        ui.add_space(4.0);
        ui.label(heading(
            format!("Pages ({})", viewer.pack.page_count()),
            self.focus == Focus::Pages,
        ));
        let pages_height = (ui.available_height() * 0.35).max(80.0);
        let names: Vec<&str> = viewer.pack.pages().iter().map(|p| p.name).collect();
        let clicked = list(
            ui,
            "pages",
            pages_height,
            names.len(),
            Some(viewer.page),
            self.reveal && self.focus == Focus::Pages,
            |index| {
                let name = names.get(index).copied().unwrap_or_default();
                format!("{index:>4}  {name}")
            },
        );
        if let Some(index) = clicked {
            viewer.select_page(index);
            viewer.view = View::Page;
            self.focus = Focus::Pages;
        }

        ui.add_space(4.0);
        if self.query.is_empty() {
            ui.label(heading(
                format!("Textures ({})", viewer.texture_count()),
                self.focus == Focus::Textures,
            ));
            let textures: Vec<&str> = viewer
                .current_page()
                .map(|page| page.textures.iter().map(|t| t.name).collect())
                .unwrap_or_default();
            let clicked = list(
                ui,
                "textures",
                ui.available_height(),
                textures.len(),
                viewer.selected_texture().map(|at| at.texture),
                self.reveal && self.focus == Focus::Textures,
                |index| textures.get(index).copied().unwrap_or_default().to_owned(),
            );
            if let Some(index) = clicked {
                viewer.select_texture(index);
                viewer.view = View::Texture;
                self.focus = Focus::Textures;
            }
        } else {
            ui.label(heading(
                format!("Matches ({})", viewer.matches.len()),
                self.focus == Focus::Textures,
            ));
            let pack = &viewer.pack;
            let matches = &viewer.matches;
            let clicked = list(
                ui,
                "matches",
                ui.available_height(),
                matches.len(),
                current_match(viewer),
                self.reveal && self.focus == Focus::Textures,
                |index| {
                    matches
                        .get(index)
                        .and_then(|at| pack.texture(*at))
                        .map(|t| t.name.to_owned())
                        .unwrap_or_default()
                },
            );
            if let Some(index) = clicked
                && let Some(found) = viewer.matches.get(index).copied()
            {
                viewer.match_cursor = index;
                viewer.select(found);
                self.focus = Focus::Textures;
            }
        }
    }

    fn info(&self, ui: &mut egui::Ui) {
        let Some(viewer) = &self.viewer else {
            return;
        };
        ui.heading("Sprite");
        egui::Grid::new("info")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                let mut row = |name: &str, value: String| {
                    ui.weak(name);
                    ui.label(value);
                    ui.end_row();
                };
                if let Some(page) = viewer.current_page() {
                    row("Page", format!("{} ({})", page.name, viewer.page));
                    if let Some((w, h)) = self.pages.size_of(viewer.page) {
                        row("Page size", format!("{w}x{h}"));
                    }
                }
                match viewer.selected_texture_data() {
                    Some(texture) => {
                        row("Texture", texture.name.to_owned());
                        let (rect, _) = model::describe_rect(texture);
                        row("Rect", rect);
                        row(
                            "Offset",
                            format!("+{}+{}", texture.offset_x, texture.offset_y),
                        );
                        row(
                            "Original size",
                            format!("{}x{}", texture.original_width, texture.original_height),
                        );
                    }
                    None => row("Texture", "none on this page".to_owned()),
                }
            });
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let Some(viewer) = &mut self.viewer else {
            ui.centered_and_justified(|ui| {
                ui.label(self.error.as_deref().unwrap_or("No pack open."));
            });
            return;
        };

        ui.horizontal(|ui| {
            ui.selectable_value(&mut viewer.view, View::Page, "Page");
            ui.selectable_value(&mut viewer.view, View::Texture, "Sprite");
            ui.separator();
            let on_page = viewer.view == View::Page;
            let page_scale = self.page_geometry.map_or(1.0, |g| g.scale);
            let zoom = match (on_page, self.page_zoom, viewer.zoom) {
                (true, PageZoom::Fit, _) => format!("fit {}", zoom::describe(page_scale)),
                (true, PageZoom::Step(_), _) => zoom::describe(page_scale),
                (false, _, Zoom::Fit) => "fit".to_owned(),
                (false, _, Zoom::Fixed(factor)) => format!("{factor}x"),
            };
            let mut delta = 0;
            if ui.button("−").clicked() {
                delta = -1;
            }
            ui.label(zoom);
            if ui.button("+").clicked() {
                delta = 1;
            }
            if delta != 0 {
                if on_page {
                    self.page_zoom = PageZoom::step(page_scale, delta);
                } else {
                    viewer.zoom_by(delta);
                }
            }
            if ui.button("Fit").clicked() {
                if on_page {
                    self.page_zoom = PageZoom::Fit;
                } else {
                    viewer.zoom = Zoom::Fit;
                }
            }
            ui.separator();
            for background in [Background::Checker, Background::Black, Background::Magenta] {
                ui.selectable_value(&mut viewer.background, background, background.label());
            }
        });
        ui.separator();

        let area = ui.available_rect_before_wrap();
        let ctx = ui.ctx().clone();
        let texture = match self.pages.get(&ctx, viewer) {
            Ok(texture) => texture,
            Err(error) => {
                ui.allocate_rect(area, egui::Sense::hover());
                warn(ui, area, &error);
                return;
            }
        };
        let checker = self.pages.checker(&ctx);
        let ppp = ctx.pixels_per_point();
        let [page_w, page_h] = texture.size();
        let page = (to_u32(page_w), to_u32(page_h));
        let pane = (to_pixels(area.width(), ppp), to_pixels(area.height(), ppp));

        if viewer.view == View::Page {
            let scale = self.page_zoom.scale(page, pane);
            // Round to whole physical pixels, so every page pixel is drawn the same size.
            let pixels = egui::vec2(
                (page.0 as f32 * scale).floor().max(1.0),
                (page.1 as f32 * scale).floor().max(1.0),
            );
            let size = pixels / ppp;
            let outline = viewer
                .selected_texture_data()
                .and_then(|t| render::clamp_rect(t, page.0, page.1).ok())
                .map(|rect| {
                    let per_pixel = (pixels.x / page.0 as f32, pixels.y / page.1 as f32);
                    render::outline_box(rect, per_pixel)
                });
            let place = |image: egui::Rect| PageGeometry {
                area,
                image,
                scale,
                outline: outline.map(|[left, top, right, bottom]| {
                    egui::Rect::from_min_max(
                        image.min + egui::vec2(left, top) / ppp,
                        image.min + egui::vec2(right, bottom) / ppp,
                    )
                }),
            };
            let paint = |painter: &egui::Painter, geometry: &PageGeometry| {
                paint_background(painter, geometry.image, viewer.background, &checker, ppp);
                painter.image(
                    texture.id(),
                    geometry.image,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                if let Some(outline) = geometry.outline {
                    paint_outline(painter, outline, ppp);
                }
            };

            let fits = pixels.x <= pane.0 as f32 && pixels.y <= pane.1 as f32;
            let geometry = if fits {
                ui.allocate_rect(area, egui::Sense::hover());
                let geometry = place(egui::Rect::from_min_size(
                    snap(area.center() - size / 2.0, ppp),
                    size,
                ));
                paint(&ui.painter_at(area), &geometry);
                geometry
            } else {
                // Scroll to the sprite whenever it or the zoom changes.
                let key = (
                    viewer.generation,
                    viewer.page,
                    viewer.texture,
                    self.page_zoom,
                );
                let scroll = self.page_scrolled != Some(key);
                self.page_scrolled = Some(key);
                ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                    egui::ScrollArea::both()
                        .id_salt("page")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                            let geometry =
                                place(egui::Rect::from_min_size(snap(rect.min, ppp), size));
                            paint(ui.painter(), &geometry);
                            if scroll && let Some(outline) = geometry.outline {
                                ui.scroll_to_rect(outline, Some(egui::Align::Center));
                            }
                            geometry
                        })
                        .inner
                })
                .inner
            };
            self.page_geometry = Some(geometry);
            return;
        }

        ui.allocate_rect(area, egui::Sense::hover());
        let placement = match render::place(
            viewer.view,
            viewer.zoom,
            viewer.selected_texture_data(),
            page,
            pane,
        ) {
            Ok(placement) => placement,
            Err(error) => {
                warn(ui, area, &format!("{error:#}"));
                return;
            }
        };
        if placement.width == 0 || placement.height == 0 {
            return;
        }

        // Round to whole physical pixels, so every pixel of a magnified sprite is drawn the same
        // size.
        let size = egui::vec2(placement.width as f32 / ppp, placement.height as f32 / ppp);
        let target = egui::Rect::from_min_size(snap(area.center() - size / 2.0, ppp), size);

        let painter = ui.painter_at(area);
        paint_background(&painter, target, viewer.background, &checker, ppp);
        let rect = placement.rect;
        let uv = egui::Rect::from_min_max(
            egui::pos2(rect.x as f32 / page_w as f32, rect.y as f32 / page_h as f32),
            egui::pos2(
                (rect.x + rect.w) as f32 / page_w as f32,
                (rect.y + rect.h) as f32 / page_h as f32,
            ),
        );
        painter.image(texture.id(), target, uv, Color32::WHITE);
    }

    fn picker_window(&mut self, ctx: &egui::Context) {
        let Some(picker) = self.picker else {
            return;
        };
        let (title, rows): (&str, Vec<String>) = match picker.kind {
            PickerKind::Pack => {
                let Some(viewer) = &self.viewer else {
                    self.picker = None;
                    return;
                };
                let names = viewer.siblings.iter().map(|p| model::display_name(p));
                ("Open pack", names.collect())
            }
            PickerKind::Install => {
                let rows = self
                    .installs
                    .iter()
                    .map(|install| format!("{}  ({})", install.dir.display(), install.source));
                ("Choose a Project Zomboid install", rows.collect())
            }
        };
        let mut chosen = None;
        let response = egui::Modal::new(egui::Id::unique("picker")).show(ctx, |ui| {
            ui.heading(title);
            chosen = list(
                ui,
                "picker-list",
                400.0,
                rows.len(),
                Some(picker.selected),
                true,
                |index| rows.get(index).cloned().unwrap_or_default(),
            );
        });
        if let Some(index) = chosen {
            self.choose_row(picker.kind, index);
        } else if response.should_close() {
            self.picker = None;
        }
    }
}

impl eframe::App for PackView {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

fn search_id() -> egui::Id {
    egui::Id::unique("texture-search")
}

/// Where the selected texture sits in the search results, if it is one of them.
fn current_match(viewer: &Viewer) -> Option<usize> {
    let selected: Option<TextureRef> = viewer.selected_texture();
    viewer.matches.iter().position(|at| Some(*at) == selected)
}

/// Maps a key press to a command. Arrow keys also reach the lists while the search box has
/// focus, so results can be browsed without leaving it.
fn key_command(key: Key, typing: bool, search_focused: bool, picking: bool) -> Option<Command> {
    let browsing = !typing || search_focused || picking;
    match key {
        Key::ArrowDown if browsing => Some(Command::Move(1)),
        Key::ArrowUp if browsing => Some(Command::Move(-1)),
        Key::PageDown if browsing => Some(Command::Move(10)),
        Key::PageUp if browsing => Some(Command::Move(-10)),
        Key::Home if !typing => Some(Command::First),
        Key::End if !typing => Some(Command::Last),
        Key::ArrowLeft if !typing && !picking => Some(Command::FocusList(Focus::Pages)),
        Key::ArrowRight if !typing && !picking => Some(Command::FocusList(Focus::Textures)),
        Key::Enter if !typing => Some(Command::Show),
        Key::Escape if !typing => Some(Command::Back),
        _ => None,
    }
}

/// Maps a typed character to a command. It matches characters rather than keys, so `/` and `+`
/// work on any keyboard layout.
fn text_command(text: &str) -> Option<Command> {
    Some(match text {
        "/" => Command::FocusSearch,
        " " => Command::ToggleView,
        "b" => Command::CycleBackground,
        "p" => Command::PickPack,
        "[" => Command::CyclePack(-1),
        "]" => Command::CyclePack(1),
        "+" | "=" => Command::ZoomBy(1),
        "-" | "_" => Command::ZoomBy(-1),
        "0" => Command::SetZoom(Zoom::Fit),
        "1" => Command::SetZoom(Zoom::Fixed(1)),
        "n" => Command::StepMatch(1),
        "N" => Command::StepMatch(-1),
        _ => return None,
    })
}

/// Draws a scrolling list of `rows` selectable rows, laying out only the visible ones. Returns
/// the row that was clicked, if any.
fn list(
    ui: &mut egui::Ui,
    id: &str,
    max_height: f32,
    rows: usize,
    selected: Option<usize>,
    reveal: bool,
    label: impl Fn(usize) -> String,
) -> Option<usize> {
    let row_height = ui.spacing().interact_size.y;
    let mut area = egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(max_height)
        .auto_shrink([false, true]);
    if reveal && let Some(selected) = selected {
        // Work out the scroll offset that centers the selection. Rows outside the view are never
        // laid out, so there is no widget to scroll to.
        let stride = row_height + ui.spacing().item_spacing.y;
        let offset = selected as f32 * stride - (max_height - row_height) / 2.0;
        area = area.vertical_scroll_offset(offset.max(0.0));
    }

    let mut clicked = None;
    area.show_rows(ui, row_height, rows, |ui, range| {
        for index in range {
            let button = egui::Button::selectable(selected == Some(index), label(index))
                .min_size(egui::vec2(ui.available_width(), row_height));
            if ui.add(button).clicked() {
                clicked = Some(index);
            }
        }
    });
    clicked
}

/// Rounds a position to the nearest physical pixel.
fn snap(point: egui::Pos2, pixels_per_point: f32) -> egui::Pos2 {
    egui::pos2(
        (point.x * pixels_per_point).round() / pixels_per_point,
        (point.y * pixels_per_point).round() / pixels_per_point,
    )
}

fn warn(ui: &egui::Ui, area: egui::Rect, message: &str) {
    ui.painter().text(
        area.center(),
        egui::Align2::CENTER_CENTER,
        message,
        egui::FontId::proportional(14.0),
        ui.visuals().warn_fg_color,
    );
}

/// Fills `target` with what transparent pixels are drawn over.
fn paint_background(
    painter: &egui::Painter,
    target: egui::Rect,
    background: Background,
    checker: &egui::TextureHandle,
    pixels_per_point: f32,
) {
    match background {
        Background::Checker => {
            // The checker texture is two 8-pixel squares across, so it repeats every 16.
            let repeats = target.size() * pixels_per_point / textures::CHECKER_SIZE as f32;
            painter.image(
                checker.id(),
                target,
                egui::Rect::from_min_size(egui::Pos2::ZERO, repeats),
                Color32::WHITE,
            );
        }
        other => {
            let [r, g, b] = other.sample(0, 0);
            painter.rect_filled(target, 0.0, Color32::from_rgb(r, g, b));
        }
    }
}

/// Draws the selection outline just outside `outline`: two physical pixels of the outline
/// color, then one of a dark edge.
fn paint_outline(painter: &egui::Painter, outline: egui::Rect, pixels_per_point: f32) {
    let pixel = 1.0 / pixels_per_point;
    let [r, g, b] = render::OUTLINE_EDGE;
    painter.rect_stroke(
        outline.expand(2.0 * pixel),
        0.0,
        egui::Stroke::new(pixel, Color32::from_rgb(r, g, b)),
        egui::StrokeKind::Outside,
    );
    let [r, g, b] = render::OUTLINE;
    painter.rect_stroke(
        outline,
        0.0,
        egui::Stroke::new(2.0 * pixel, Color32::from_rgb(r, g, b)),
        egui::StrokeKind::Outside,
    );
}

/// Converts a length in points to whole physical pixels, rounding down.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to u32's range first"
)]
fn to_pixels(points: f32, pixels_per_point: f32) -> u32 {
    let pixels = (points * pixels_per_point).floor();
    if pixels.is_nan() || pixels <= 0.0 {
        0
    } else if pixels >= u32::MAX as f32 {
        u32::MAX
    } else {
        pixels as u32
    }
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
