//! The terminal frontend's state, and how each key changes it.
//!
//! Nothing here talks to the terminal, so the key handling can be tested without one.

use crate::{
    install::{
        Install,
        Places,
    },
    model::{
        View,
        Viewer,
        Zoom,
    },
};
use anyhow::Result;
use ratatui::crossterm::event::{
    KeyCode,
    KeyEvent,
    KeyModifiers,
};

/// Which list the arrow keys move.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The list of pages.
    Pages,
    /// The list of textures on the selected page.
    Textures,
}

/// A prompt or popup that receives the keys instead of the lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Modal {
    /// Nothing is open, and the keys go to the lists.
    None,
    /// Typing a search query.
    Search { query: String },
    /// Choosing among the packs in the folder.
    PackPicker { selected: usize },
    /// Choosing among the installed copies of the game.
    InstallPicker {
        installs: Vec<Install>,
        selected: usize,
    },
}

/// Everything the terminal viewer knows.
pub struct App {
    /// The state shared with the window frontend.
    pub viewer: Viewer,
    /// Which list the arrow keys move.
    pub focus: Focus,
    /// The open prompt or popup, if any.
    pub modal: Modal,
    /// Set when the image must be sent to the terminal again.
    pub dirty: bool,
    /// Set when the user asked to quit.
    pub should_quit: bool,
}

/// The parts of the viewer that decide what the image pane shows.
type Shown = (u64, usize, usize, View, Zoom, crate::render::Background);

impl App {
    /// Starts on the pages list, with nothing open on top.
    pub fn new(viewer: Viewer) -> Self {
        Self {
            viewer,
            focus: Focus::Pages,
            modal: Modal::None,
            dirty: true,
            should_quit: false,
        }
    }

    fn shown(&self) -> Shown {
        let v = &self.viewer;
        (
            v.generation,
            v.page,
            v.texture,
            v.view,
            v.zoom,
            v.background,
        )
    }

    /// Handles one key. Returns `Ok(())` even for keys that do nothing.
    pub fn on_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            self.should_quit = true;
            return Ok(());
        }

        let before = self.shown();
        match &self.modal {
            Modal::None => self.on_key_normal(key),
            Modal::Search { .. } => self.on_key_search(key),
            Modal::PackPicker { .. } => self.on_key_picker(key),
            Modal::InstallPicker { .. } => self.on_key_install_picker(key),
        }
        if self.shown() != before {
            self.dirty = true;
        }
        Ok(())
    }

    /// Steps to the previous or next pack in the directory, wrapping.
    pub fn cycle_pack(&mut self, delta: isize) {
        let before = self.shown();
        if self.viewer.cycle_pack(delta) {
            self.focus = Focus::Pages;
        }
        if self.shown() != before {
            self.dirty = true;
        }
    }

    fn on_key_normal(&mut self, key: KeyEvent) {
        let list_len = match self.focus {
            Focus::Pages => self.viewer.pack.page_count(),
            Focus::Textures => self.viewer.texture_count(),
        };

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Pages => Focus::Textures,
                    Focus::Textures => Focus::Pages,
                };
            }
            KeyCode::Down | KeyCode::Char('j') => self.step(1, list_len),
            KeyCode::Up | KeyCode::Char('k') => self.step(-1, list_len),
            KeyCode::PageDown => self.step(10, list_len),
            KeyCode::PageUp => self.step(-10, list_len),
            KeyCode::Home | KeyCode::Char('g') => self.set_selection(0),
            KeyCode::End | KeyCode::Char('G') => self.set_selection(list_len.saturating_sub(1)),
            KeyCode::Enter => {
                self.focus = Focus::Textures;
                self.viewer.view = View::Texture;
                // Send the image again, even if the texture view was already showing.
                self.dirty = true;
            }
            KeyCode::Esc if self.viewer.view == View::Texture => {
                self.viewer.view = View::Page;
            }
            KeyCode::Char(' ') => self.viewer.toggle_view(),
            KeyCode::Char('b') => self.viewer.cycle_background(),
            KeyCode::Char('/') => {
                self.modal = Modal::Search {
                    query: self.viewer.last_query.clone(),
                };
            }
            KeyCode::Char('n') => self.step_match(1),
            KeyCode::Char('N') => self.step_match(-1),
            KeyCode::Char('p') if !self.viewer.siblings.is_empty() => {
                self.modal = Modal::PackPicker {
                    selected: self.viewer.current_sibling,
                };
            }
            KeyCode::Char('i') => self.open_install_picker(),
            KeyCode::Char('+' | '=') => self.viewer.zoom_by(1),
            KeyCode::Char('-' | '_') => self.viewer.zoom_by(-1),
            KeyCode::Char('0') => {
                self.viewer.zoom = Zoom::Fit;
                self.dirty = true;
            }
            KeyCode::Char('1') => {
                self.viewer.zoom = Zoom::Fixed(1);
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn on_key_search(&mut self, key: KeyEvent) {
        let Modal::Search { query } = &mut self.modal else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.modal = Modal::None,
            KeyCode::Char(c) => {
                query.push(c);
            }
            KeyCode::Backspace => {
                query.pop();
            }
            KeyCode::Enter => {
                let query = query.clone();
                self.modal = Modal::None;
                if self.viewer.run_search(&query) {
                    self.focus = Focus::Textures;
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    fn on_key_picker(&mut self, key: KeyEvent) {
        let Modal::PackPicker { selected } = &mut self.modal else {
            return;
        };
        let last = self.viewer.siblings.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'p') => self.modal = Modal::None,
            KeyCode::Down | KeyCode::Char('j') => *selected = (*selected + 1).min(last),
            KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
            KeyCode::Home | KeyCode::Char('g') => *selected = 0,
            KeyCode::End | KeyCode::Char('G') => *selected = last,
            KeyCode::Enter => {
                let target = *selected;
                self.modal = Modal::None;
                if self.viewer.switch_pack(target) {
                    self.focus = Focus::Pages;
                }
            }
            _ => {}
        }
    }

    /// Searches for installs again and lists them, or says where it looked.
    fn open_install_picker(&mut self) {
        match Places::detect().find() {
            Ok(installs) => {
                let path = &self.viewer.pack.path;
                let selected = installs
                    .iter()
                    .position(|install| path.starts_with(&install.dir))
                    .unwrap_or(0);
                self.modal = Modal::InstallPicker { installs, selected };
            }
            Err(missing) => self.viewer.status = Some(missing.to_string()),
        }
    }

    fn on_key_install_picker(&mut self, key: KeyEvent) {
        let Modal::InstallPicker { installs, selected } = &mut self.modal else {
            return;
        };
        let last = installs.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'i') => self.modal = Modal::None,
            KeyCode::Down | KeyCode::Char('j') => *selected = (*selected + 1).min(last),
            KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
            KeyCode::Home | KeyCode::Char('g') => *selected = 0,
            KeyCode::End | KeyCode::Char('G') => *selected = last,
            KeyCode::Enter => {
                let chosen = installs.get(*selected).map(Install::texturepacks);
                self.modal = Modal::None;
                if let Some(path) = chosen {
                    self.open(&path);
                }
            }
            _ => {}
        }
    }

    /// Replaces the open pack with `path`, keeping the current one if that fails.
    pub fn open(&mut self, path: &std::path::Path) {
        match Viewer::open(path, None) {
            Ok(mut viewer) => {
                // A new viewer starts its generation count from zero. Without this, the page
                // cache could keep showing pages from the previous pack.
                viewer.generation = self.viewer.generation.wrapping_add(1);
                self.viewer = viewer;
                self.focus = Focus::Pages;
                self.dirty = true;
            }
            Err(error) => {
                self.viewer.status = Some(format!("cannot open {}: {error:#}", path.display()));
            }
        }
    }

    /// Moves the focused list by `delta`, clamped to its ends.
    fn step(&mut self, delta: isize, len: usize) {
        if len == 0 {
            return;
        }
        let current = match self.focus {
            Focus::Pages => self.viewer.page,
            Focus::Textures => self.viewer.texture,
        };
        let next = current
            .saturating_add_signed(delta)
            .min(len.saturating_sub(1));
        self.set_selection(next);
    }

    fn set_selection(&mut self, index: usize) {
        match self.focus {
            Focus::Pages => self.viewer.select_page(index),
            Focus::Textures => self.viewer.select_texture(index),
        };
    }

    fn step_match(&mut self, delta: isize) {
        if self.viewer.step_match(delta) {
            self.focus = Focus::Textures;
            self.dirty = true;
        }
    }
}
