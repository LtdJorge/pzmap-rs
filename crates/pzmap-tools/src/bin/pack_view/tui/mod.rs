//! The terminal frontend.
//!
//! Ratatui draws the lists and text. The preview is sent to the terminal as a real image through
//! the kitty graphics protocol, and drawn over a pane that ratatui leaves blank.

mod app;
mod kitty;
mod raster;
mod ui;

use crate::{
    Start,
    install::Install,
    model::{
        View,
        Viewer,
    },
    render,
};
use anyhow::{
    Context,
    Result,
    bail,
};
use app::{
    App,
    Modal,
};
use raster::PageCache;
use ratatui::{
    DefaultTerminal,
    backend::Backend,
    crossterm::{
        cursor::MoveTo,
        event::{
            self,
            Event,
            KeyCode,
            KeyEventKind,
            KeyModifiers,
        },
        execute,
    },
    layout::Rect,
};
use std::{
    io::{
        Write,
        stdout,
    },
    time::{
        Duration,
        Instant,
    },
};

/// Runs the terminal viewer until the user quits.
///
/// If several installs were found, it first lists them so the user can pick one.
pub fn run(start: Start, pack_name: Option<&str>, force_graphics: bool) -> Result<()> {
    let installs = match start {
        Start::Open(ref path) => {
            // Open the pack before taking over the terminal, so an error for a bad path prints
            // on a normal screen.
            let viewer = Viewer::open(path, pack_name)?;
            return run_viewer(viewer, force_graphics);
        }
        Start::Choose(installs) => installs,
        Start::Missing(missing) => {
            bail!("{missing}. Or pass a .pack file or a folder of them as PATH.")
        }
    };
    check_graphics(force_graphics)?;

    let mut terminal = ratatui::try_init().context("taking over the terminal")?;
    let outcome = choose_install(&mut terminal, &installs).and_then(|chosen| match chosen {
        Some(install) => {
            let viewer = Viewer::open(&install.texturepacks(), pack_name)?;
            event_loop(terminal, App::new(viewer))
        }
        None => Ok(()),
    });
    let _ = cleanup();
    ratatui::try_restore().context("restoring the terminal")?;
    outcome
}

fn run_viewer(viewer: Viewer, force_graphics: bool) -> Result<()> {
    check_graphics(force_graphics)?;
    let app = App::new(viewer);
    let terminal = ratatui::try_init().context("taking over the terminal")?;
    let outcome = event_loop(terminal, app);
    // Restore the terminal before returning an error, so the message prints on a normal screen.
    let _ = cleanup();
    ratatui::try_restore().context("restoring the terminal")?;
    outcome
}

fn check_graphics(force_graphics: bool) -> Result<()> {
    if !force_graphics && !crate::mode::terminal_supports_graphics() {
        bail!(
            "this does not look like a terminal that supports the kitty graphics protocol.\n\
             pack-view draws real pixels and has no text fallback, so run it under kitty, \
             Ghostty, WezTerm, or Konsole.\n\
             Pass --force-graphics to try anyway{}.",
            if cfg!(feature = "gui") {
                ", or --gui to open a window instead"
            } else {
                ""
            }
        );
    }
    Ok(())
}

/// Lists `installs` until the user picks one with enter, or leaves with escape or `q`.
fn choose_install<'a>(
    terminal: &mut DefaultTerminal,
    installs: &'a [Install],
) -> Result<Option<&'a Install>> {
    let last = installs.len().saturating_sub(1);
    let mut selected = 0;
    loop {
        terminal
            .draw(|frame| ui::draw_install_picker(frame, frame.area(), installs, selected))
            .context("drawing the install list")?;
        let Event::Key(key) = event::read().context("reading input")? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1).min(last),
            KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
            KeyCode::Home | KeyCode::Char('g') => selected = 0,
            KeyCode::End | KeyCode::Char('G') => selected = last,
            KeyCode::Enter => return Ok(installs.get(selected)),
            KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(None);
            }
            _ => {}
        }
    }
}

/// Removes the image from the screen and frees the terminal's copy of its pixels.
fn cleanup() -> Result<()> {
    let mut out = stdout();
    kitty::delete_image(&mut out)?;
    out.flush()?;
    Ok(())
}

fn event_loop(mut terminal: DefaultTerminal, mut app: App) -> Result<()> {
    let mut cache = PageCache::new();
    let mut generation = app.viewer.generation;
    // Where the image was last placed. Comparing it with the pane catches a resize that ratatui
    // handled without reporting an event.
    let mut placed: Option<Rect> = None;

    loop {
        if generation != app.viewer.generation {
            // A different pack is open, so every cached page belongs to the wrong file.
            cache.clear();
            generation = app.viewer.generation;
        }

        let mut pane = Rect::default();
        terminal
            .draw(|frame| pane = ui::draw(frame, &app))
            .context("drawing the interface")?;

        // The terminal draws images on top of text. While a popup covers the image pane, the
        // image is removed so that it does not hide the popup.
        if overlaps_image(&app) {
            if placed.is_some() {
                let mut out = stdout();
                let _ = kitty::delete_placements(&mut out);
                let _ = out.flush();
                // Clearing this forces a redraw once the popup closes.
                placed = None;
            }
        } else if app.dirty || placed != Some(pane) {
            let previous_status = app.viewer.status.clone();

            if let Err(error) = show(&mut cache, &app, pane, cell_size(&mut terminal)) {
                app.viewer.status = Some(format!("{error:#}"));
                let mut out = stdout();
                let _ = kitty::delete_placements(&mut out);
                let _ = out.flush();
            }
            app.dirty = false;
            placed = Some(pane);

            // Showing the image can put an error in the status line. Draw again only if the
            // status changed, since the image pane itself never needs it.
            if app.viewer.status != previous_status {
                terminal
                    .draw(|frame| {
                        let _ = ui::draw(frame, &app);
                    })
                    .context("drawing the interface")?;
            }
        }

        pump_events(&mut app)?;
        if app.should_quit {
            return Ok(());
        }
    }
}

/// Whether something is drawn on top of the image pane. Only the pickers do that. The search
/// prompt is on the status line, which the image never covers.
fn overlaps_image(app: &App) -> bool {
    matches!(
        app.modal,
        Modal::PackPicker { .. } | Modal::InstallPicker { .. }
    )
}

/// Waits for input, then handles every event already queued.
///
/// When an arrow key is held down, this draws the list once where it stops instead of once per
/// repeated key press.
fn pump_events(app: &mut App) -> Result<()> {
    if !event::poll(Duration::from_millis(250)).context("polling for input")? {
        return Ok(());
    }

    // Stop after a short time, so a held key cannot keep the loop from ever drawing.
    let deadline = Instant::now() + Duration::from_millis(8);
    loop {
        match event::read().context("reading input")? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                // Any key clears an old status message, except while typing a search.
                if app.viewer.status.is_some() && !matches!(app.modal, Modal::Search { .. }) {
                    app.viewer.status = None;
                }
                match key.code {
                    KeyCode::Char('[') if matches!(app.modal, Modal::None) => app.cycle_pack(-1),
                    KeyCode::Char(']') if matches!(app.modal, Modal::None) => app.cycle_pack(1),
                    _ => app.on_key(key)?,
                }
            }
            Event::Resize(_, _) => app.dirty = true,
            _ => {}
        }

        if app.should_quit || Instant::now() >= deadline {
            break;
        }
        if !event::poll(Duration::ZERO).context("polling for input")? {
            break;
        }
    }
    Ok(())
}

/// Returns the size of one terminal cell in pixels.
///
/// The terminal reports its size in pixels, but many terminals report zero. Then this assumes a
/// cell 8 pixels wide and 16 tall.
fn cell_size(terminal: &mut DefaultTerminal) -> (u32, u32) {
    const FALLBACK: (u32, u32) = (8, 16);

    let Ok(size) = terminal.backend_mut().window_size() else {
        return FALLBACK;
    };
    let columns = u32::from(size.columns_rows.width);
    let rows = u32::from(size.columns_rows.height);
    let width = u32::from(size.pixels.width);
    let height = u32::from(size.pixels.height);
    if columns == 0 || rows == 0 || width == 0 || height == 0 {
        return FALLBACK;
    }
    ((width / columns).max(1), (height / rows).max(1))
}

/// Decodes and scales the current selection, and sends it to the terminal.
fn show(cache: &mut PageCache, app: &App, pane: Rect, cell: (u32, u32)) -> Result<()> {
    if pane.width == 0 || pane.height == 0 {
        return Ok(());
    }

    let page_index = app.viewer.page;
    let png = app
        .viewer
        .pack
        .page(page_index)
        .map(|page| page.png)
        .with_context(|| format!("pack has no page {page_index}"))?;
    let source = cache.get_or_decode(page_index, png)?;
    let (page_w, page_h) = (source.width(), source.height());

    // The pane in pixels, minus one cell of slack. A rounding error could otherwise put the last
    // row of pixels into the border cell and scroll the terminal.
    let max_w = u32::from(pane.width).saturating_sub(1) * cell.0;
    let max_h = u32::from(pane.height).saturating_sub(1) * cell.1;
    if max_w == 0 || max_h == 0 {
        return Ok(());
    }

    let placement = render::place(
        app.viewer.view,
        app.viewer.zoom,
        app.viewer.selected_texture_data(),
        (page_w, page_h),
        (max_w, max_h),
    )?;
    if placement.width == 0 || placement.height == 0 {
        return Ok(());
    }

    let mut rendered = raster::render(
        source,
        placement.rect,
        placement.width,
        placement.height,
        placement.nearest,
        app.viewer.background,
    )?;
    if app.viewer.view == View::Page
        && let Some(texture) = app.viewer.selected_texture_data()
        && let Ok(rect) = render::clamp_rect(texture, page_w, page_h)
    {
        let scale = (
            placement.width as f32 / page_w as f32,
            placement.height as f32 / page_h as f32,
        );
        raster::outline(&mut rendered, render::outline_box(rect, scale));
    }

    let frame = kitty::Frame {
        pixels: &rendered.rgb,
        width: rendered.width,
        height: rendered.height,
    };

    // Center the image in the pane, in whole cells.
    let used_cols = u16::try_from(rendered.width.div_ceil(cell.0)).unwrap_or(pane.width);
    let used_rows = u16::try_from(rendered.height.div_ceil(cell.1)).unwrap_or(pane.height);
    let x = pane.x + pane.width.saturating_sub(used_cols) / 2;
    let y = pane.y + pane.height.saturating_sub(used_rows) / 2;

    let mut out = stdout();
    kitty::delete_placements(&mut out)?;
    execute!(out, MoveTo(x, y)).context("positioning the cursor")?;
    kitty::transmit_and_place(&mut out, &frame).context("sending the image")?;
    out.flush().context("flushing the image")?;
    Ok(())
}
