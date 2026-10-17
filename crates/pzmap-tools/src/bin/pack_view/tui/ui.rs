//! Draws the text part of the terminal screen: two lists, a header and a status line.
//!
//! The image pane is left as blank cells. Ratatui only repaints cells that changed, and these
//! never change, so it never draws over the image the terminal shows there.

use super::app::{
    App,
    Focus,
    Modal,
};
use crate::{
    install::Install,
    model::{
        View,
        Zoom,
    },
};
use ratatui::{
    Frame,
    layout::{
        Alignment,
        Constraint,
        Layout,
        Rect,
    },
    style::{
        Color,
        Modifier,
        Style,
    },
    text::{
        Line,
        Span,
    },
    widgets::{
        Block,
        BorderType,
        Clear,
        List,
        ListItem,
        ListState,
        Paragraph,
    },
};

/// Width of the list sidebar in cells. It fits the longest texture names in the base game.
const SIDEBAR_WIDTH: u16 = 34;

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;

/// Draws one frame and returns the interior of the image pane.
pub fn draw(frame: &mut Frame<'_>, app: &App) -> Rect {
    let [header, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let [sidebar, image] =
        Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(10)]).areas(body);

    draw_header(frame, header, app);
    draw_sidebar(frame, sidebar, app);
    let pane = draw_image_frame(frame, image, app);
    draw_status(frame, status, app);

    match &app.modal {
        Modal::PackPicker { selected } => draw_pack_picker(frame, frame.area(), app, *selected),
        Modal::InstallPicker { installs, selected } => {
            draw_install_picker(frame, frame.area(), installs, *selected)
        }
        Modal::None | Modal::Search { .. } => {}
    }

    pane
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let line = Line::from(vec![
        Span::styled(
            format!(" {} ", app.viewer.pack.name),
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  {} pages, {} textures",
            app.viewer.pack.page_count(),
            app.viewer.pack.texture_count()
        )),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_sidebar(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let [pages_area, textures_area] =
        Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(area);

    let pages: Vec<ListItem<'_>> = app
        .viewer
        .pack
        .pages()
        .iter()
        .enumerate()
        .map(|(index, page)| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{index:>4} "), Style::default().fg(DIM)),
                Span::raw(truncate(page.name, usize::from(SIDEBAR_WIDTH) - 9)),
            ]))
        })
        .collect();

    render_list(
        frame,
        pages_area,
        pages,
        &format!(" Pages ({}) ", app.viewer.pack.page_count()),
        app.viewer.page,
        app.focus == Focus::Pages,
    );

    let textures: Vec<ListItem<'_>> = app
        .viewer
        .pack
        .page(app.viewer.page)
        .map(|page| {
            page.textures
                .iter()
                .map(|texture| {
                    ListItem::new(Span::raw(truncate(
                        texture.name,
                        usize::from(SIDEBAR_WIDTH) - 4,
                    )))
                })
                .collect()
        })
        .unwrap_or_default();

    render_list(
        frame,
        textures_area,
        textures,
        &format!(" Textures ({}) ", app.viewer.texture_count()),
        app.viewer.texture,
        app.focus == Focus::Textures,
    );
}

fn render_list(
    frame: &mut Frame<'_>,
    area: Rect,
    items: Vec<ListItem<'_>>,
    title: &str,
    selected: usize,
    focused: bool,
) {
    let border = if focused {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(DIM)
    };
    let block = Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(border)
        .title(Span::styled(
            title.to_owned(),
            if focused {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DIM)
            },
        ));

    let highlight = if focused {
        Style::default().fg(Color::Black).bg(ACCENT)
    } else {
        Style::default().add_modifier(Modifier::REVERSED)
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(highlight)
        .highlight_symbol("▌");

    let mut state = ListState::default();
    state.select(Some(selected));
    frame.render_stateful_widget(list, area, &mut state);
}

/// Draws the border around the image pane and returns its interior.
fn draw_image_frame(frame: &mut Frame<'_>, area: Rect, app: &App) -> Rect {
    let title = match app.viewer.view {
        View::Page => app
            .viewer
            .pack
            .page(app.viewer.page)
            .map_or_else(|| " page ".to_owned(), |p| format!(" {} ", p.name)),
        View::Texture => app
            .viewer
            .selected_texture()
            .and_then(|at| app.viewer.pack.texture(at))
            .map_or_else(|| " no texture ".to_owned(), |t| format!(" {} ", t.name)),
    };

    let block = Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            match app.viewer.view {
                View::Page => " page ",
                View::Texture => " texture ",
            },
            Style::default().fg(DIM),
        )))
        .title_alignment(Alignment::Left);

    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

fn draw_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if let Modal::Search { query } = &app.modal {
        let line = Line::from(vec![
            Span::styled(
                "/",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(query.clone()),
            Span::styled("█", Style::default().fg(ACCENT)),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    if let Some(status) = &app.viewer.status {
        let line = Line::from(Span::styled(
            format!(" {status}"),
            Style::default().fg(Color::Yellow),
        ));
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    let mut spans = vec![Span::raw(" ")];

    if let Some(texture) = app.viewer.selected_texture_data() {
        let (rect, untrimmed) = crate::model::describe_rect(texture);
        spans.push(Span::styled(rect, Style::default().fg(Color::White)));
        // The untrimmed box is the sprite before the packer trimmed its transparent margins. It
        // is shown only when it differs from the trimmed rectangle.
        if let Some(untrimmed) = untrimmed {
            spans.push(Span::styled(
                format!("  {untrimmed}"),
                Style::default().fg(DIM),
            ));
        }
    } else {
        spans.push(Span::styled(
            "no textures on this page",
            Style::default().fg(DIM),
        ));
    }

    spans.push(Span::styled("  •  ", Style::default().fg(DIM)));
    spans.push(Span::styled(
        match app.viewer.zoom {
            Zoom::Fit => "fit".to_owned(),
            Zoom::Fixed(factor) => format!("{factor}x"),
        },
        Style::default().fg(DIM),
    ));
    spans.push(Span::styled(
        format!("  •  bg {}", app.viewer.background.label()),
        Style::default().fg(DIM),
    ));
    spans.push(Span::styled(
        "  •  tab pane · enter zoom · / search · b bg · p pack · i install · q quit",
        Style::default().fg(DIM),
    ));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_pack_picker(frame: &mut Frame<'_>, area: Rect, app: &App, selected: usize) {
    let height = u16::try_from(app.viewer.siblings.len().min(20) + 2).unwrap_or(12);
    let width = 52.min(area.width.saturating_sub(4));
    let popup = centered(area, width, height);

    let items: Vec<ListItem<'_>> = app
        .viewer
        .siblings
        .iter()
        .map(|path| {
            let name = crate::model::display_name(path);
            ListItem::new(Span::raw(name))
        })
        .collect();

    frame.render_widget(Clear, popup);
    render_list(frame, popup, items, " Open pack ", selected, true);
}

/// Lists the installed copies of the game, with where each was found.
pub fn draw_install_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    installs: &[Install],
    selected: usize,
) {
    let rows: Vec<String> = installs
        .iter()
        .map(|install| format!("{}  ({})", install.dir.display(), install.source))
        .collect();
    let longest = rows
        .iter()
        .map(|row| row.chars().count())
        .max()
        .unwrap_or(0);
    let width = u16::try_from(longest + 6)
        .unwrap_or(u16::MAX)
        .max(40)
        .min(area.width.saturating_sub(4));
    let height = u16::try_from(installs.len().min(20) + 2).unwrap_or(12);
    let popup = centered(area, width, height);
    let items: Vec<ListItem<'_>> = rows.into_iter().map(ListItem::new).collect();

    frame.render_widget(Clear, popup);
    render_list(
        frame,
        popup,
        items,
        " Choose a Project Zomboid install ",
        selected,
        true,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [_, middle, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height.min(area.height)),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, center, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(width.min(area.width)),
        Constraint::Fill(1),
    ])
    .areas(middle);
    center
}

/// Shortens a name to `width` cells, keeping the end. Texture names such as
/// `walls_exterior_house_01_42` differ from their neighbors mostly in the last few characters.
fn truncate(name: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if name.chars().count() <= width {
        return name.to_owned();
    }
    let keep = width.saturating_sub(1);
    let start = name.chars().count().saturating_sub(keep);
    let tail: String = name.chars().skip(start).collect();
    format!("…{tail}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn a_short_name_is_unchanged() {
        assert_eq!(truncate("walls_01", 20), "walls_01");
        assert_eq!(truncate("exactly_ten", 11), "exactly_ten");
    }

    #[test]
    fn a_long_name_keeps_its_tail() {
        // The ellipsis costs one cell, so ten cells show the last nine characters.
        assert_eq!(
            truncate("walls_exterior_house_01_42", 10),
            "\u{2026}use_01_42"
        );
    }

    #[test]
    fn truncation_fits_the_requested_width() {
        for width in 1..40 {
            let out = truncate("vegetation_trees_01_0_extra_long_name", width);
            assert!(out.chars().count() <= width, "width {width} gave {out:?}");
        }
    }

    #[test]
    fn a_zero_width_gives_nothing() {
        assert_eq!(truncate("anything", 0), "");
    }
}
