//! `pack-view`, a viewer for the sprites in Project Zomboid's texture packs.
//!
//! A texture pack is a `.pack` file holding many sprites. The sprites are packed onto a few large
//! PNG images called pages. Each sprite is a named rectangle on one page, and the file format
//! calls that rectangle a texture.
//!
//! The viewer lists the pages and textures and previews the selected one. It draws either in a
//! terminal that supports the kitty graphics protocol or in a window. It never writes to the
//! files it opens.

#[cfg(not(any(feature = "tui", feature = "gui")))]
compile_error!(
    "pack-view needs a frontend: enable the `tui` feature, the `gui` feature, or both (the default)"
);

#[cfg(feature = "gui")]
mod gui;
mod install;
mod mode;
mod model;
mod pack;
mod render;
#[cfg(feature = "tui")]
mod tui;

use crate::mode::{
    Mode,
    Startup,
};
use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "pack-view",
    about = "Browse the sprites inside a Project Zomboid .pack texture atlas.",
    version
)]
struct Args {
    /// A .pack file, or a folder of them. Defaults to the texture packs of the installed game.
    path: Option<PathBuf>,

    /// Name of the pack to open first, when a folder was given. Defaults to the first one.
    #[arg(long, value_name = "NAME")]
    pack: Option<String>,

    /// Draw images even in a terminal that is not known to support them.
    #[arg(long)]
    force_graphics: bool,

    /// Run in the terminal, even when a window could be opened.
    #[arg(long, conflicts_with = "gui")]
    tui: bool,

    /// Open a window, even when started from a terminal.
    #[arg(long)]
    gui: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let forced = match (args.tui, args.gui) {
        (true, _) => Some(Mode::Tui),
        (_, true) => Some(Mode::Gui),
        _ => None,
    };
    let startup = Startup::detect(forced, args.force_graphics);
    let mode = startup.choose();
    let start = match args.path.clone() {
        Some(path) => Start::Open(path),
        None => match install::Places::detect().find() {
            Ok(installs) => match <[install::Install; 1]>::try_from(installs) {
                Ok([only]) => Start::Open(only.texturepacks()),
                Err(installs) => Start::Choose(installs),
            },
            Err(missing) => Start::Missing(missing),
        },
    };

    match mode {
        Mode::Tui => run_tui(&args, start),
        Mode::Gui => {
            let freed = startup.should_free_console(mode);
            if freed {
                mode::free_console();
            }
            match run_gui(&args, start.clone()) {
                Ok(()) => Ok(()),
                Err(error) if startup.tui_compiled && startup.is_terminal && !freed => {
                    eprintln!("pack-view: cannot open a window ({error:#}), using the terminal");
                    run_tui(&args, start)
                }
                Err(error) => {
                    if freed {
                        report_without_console(&error);
                    }
                    Err(error)
                }
            }
        }
    }
}

/// What to show first.
#[derive(Clone, Debug)]
pub enum Start {
    /// A `.pack` file or a folder of them, from the command line or from the only install found.
    Open(PathBuf),
    /// Several installs were found, and the user picks one.
    Choose(Vec<install::Install>),
    /// No path was given and no install was found.
    Missing(install::NotFound),
}

#[cfg(feature = "tui")]
fn run_tui(args: &Args, start: Start) -> Result<()> {
    tui::run(start, args.pack.as_deref(), args.force_graphics)
}

#[cfg(not(feature = "tui"))]
fn run_tui(_args: &Args, _start: Start) -> Result<()> {
    anyhow::bail!("this build of pack-view has no terminal frontend")
}

#[cfg(feature = "gui")]
fn run_gui(args: &Args, start: Start) -> Result<()> {
    gui::run(start, args.pack.clone())
}

#[cfg(not(feature = "gui"))]
fn run_gui(_args: &Args, _start: Start) -> Result<()> {
    anyhow::bail!("this build of pack-view has no window frontend")
}

/// Shows a startup error in a dialog box.
///
/// A double-clicked exe has already closed its console, so an error printed there would never be
/// seen.
#[cfg(feature = "gui")]
fn report_without_console(error: &anyhow::Error) {
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("pack-view")
        .set_description(format!("{error:#}"))
        .show();
}

#[cfg(not(feature = "gui"))]
fn report_without_console(_error: &anyhow::Error) {}
