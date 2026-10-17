//! Chooses between the terminal and the window frontend at startup.

/// Which frontend draws the viewer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The terminal frontend.
    Tui,
    /// The window frontend.
    Gui,
}

/// Everything the choice depends on, gathered by [`Startup::detect`] or written out in tests.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Startup {
    /// Set by `--tui` or `--gui`.
    pub forced: Option<Mode>,
    /// Whether this build includes the terminal frontend.
    pub tui_compiled: bool,
    /// Whether this build includes the window frontend.
    pub gui_compiled: bool,
    /// Whether stdin and stdout are both terminals.
    pub is_terminal: bool,
    /// Whether the terminal can show images: it is one known to support the kitty graphics
    /// protocol, or `--force-graphics` was given.
    pub terminal_has_graphics: bool,
    /// On Windows, how many processes share this console. `None` elsewhere.
    pub console_processes: Option<u32>,
}

impl Startup {
    /// Reads the flags given and the environment this process was started in.
    pub fn detect(forced: Option<Mode>, force_graphics: bool) -> Self {
        use std::io::IsTerminal;

        Self {
            forced,
            tui_compiled: cfg!(feature = "tui"),
            gui_compiled: cfg!(feature = "gui"),
            is_terminal: std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
            terminal_has_graphics: force_graphics || terminal_supports_graphics(),
            console_processes: console_process_count(),
        }
    }

    /// Picks the frontend to start.
    ///
    /// A build with only one frontend always uses it. Otherwise `--tui` or `--gui` decides.
    /// Without a flag, the terminal frontend runs only when a shell started the program and the
    /// terminal can show images. Everything else gets a window:
    ///
    /// - On Windows, a console that only this process uses means it was double-clicked in
    ///   Explorer. A console shared with other processes means a shell started it.
    /// - Elsewhere, a shell started it when stdin and stdout are both terminals.
    pub fn choose(&self) -> Mode {
        match (self.tui_compiled, self.gui_compiled) {
            (true, false) => return Mode::Tui,
            (false, true) => return Mode::Gui,
            _ => {}
        }
        if let Some(mode) = self.forced {
            return mode;
        }
        let from_shell = match self.console_processes {
            Some(count) => count > 1,
            None => self.is_terminal,
        };
        if from_shell && self.terminal_has_graphics {
            Mode::Tui
        } else {
            Mode::Gui
        }
    }

    /// Whether to detach from the console before the window opens.
    ///
    /// Windows opens a console window for a double-clicked exe. Detaching closes it, so it does
    /// not stay open behind the viewer's window.
    pub fn should_free_console(&self, mode: Mode) -> bool {
        mode == Mode::Gui && self.console_processes == Some(1)
    }
}

/// Whether this terminal is known to support the kitty graphics protocol.
///
/// It checks environment variables that these terminals set. Asking the terminal directly
/// would make it reply on the same input the viewer reads keys from. `--force-graphics` covers
/// a terminal not recognized here.
pub fn terminal_supports_graphics() -> bool {
    const MARKERS: [&str; 4] = [
        "KITTY_WINDOW_ID",
        "GHOSTTY_RESOURCES_DIR",
        "WEZTERM_PANE",
        "KONSOLE_VERSION",
    ];

    if MARKERS.iter().any(|key| std::env::var_os(key).is_some()) {
        return true;
    }

    ["TERM", "TERM_PROGRAM"].iter().any(|key| {
        std::env::var(key).is_ok_and(|value| {
            let value = value.to_ascii_lowercase();
            ["kitty", "ghostty", "wezterm", "konsole"]
                .iter()
                .any(|name| value.contains(name))
        })
    })
}

/// How many processes are attached to this console, or `None` when not on Windows.
#[cfg(windows)]
fn console_process_count() -> Option<u32> {
    let mut ids = [0u32; 4];
    let capacity = u32::try_from(ids.len()).unwrap_or(0);
    // SAFETY: the buffer is valid for `capacity` process ids.
    let count = unsafe {
        windows_sys::Win32::System::Console::GetConsoleProcessList(ids.as_mut_ptr(), capacity)
    };
    // Zero means the call failed, such as when there is no console at all.
    (count != 0).then_some(count)
}

#[cfg(not(windows))]
fn console_process_count() -> Option<u32> {
    None
}

/// Detaches this process from its console.
///
/// After a double-click, this closes the console window that Windows opened for the exe. That
/// console window is visible for a moment before the viewer's window appears.
#[cfg(windows)]
pub fn free_console() {
    // SAFETY: takes no arguments and only detaches this process from its console.
    unsafe {
        windows_sys::Win32::System::Console::FreeConsole();
    }
}

#[cfg(not(windows))]
pub fn free_console() {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both frontends compiled in, started from a terminal that can show images.
    fn both(forced: Option<Mode>, is_terminal: bool, console: Option<u32>) -> Startup {
        Startup {
            forced,
            tui_compiled: true,
            gui_compiled: true,
            is_terminal,
            terminal_has_graphics: true,
            console_processes: console,
        }
    }

    fn without_graphics(startup: Startup) -> Startup {
        Startup {
            terminal_has_graphics: false,
            ..startup
        }
    }

    #[test]
    fn a_terminal_without_images_gets_the_gui() {
        // Windows Terminal, cmd, macOS Terminal and most Linux terminals.
        assert_eq!(
            without_graphics(both(None, true, Some(2))).choose(),
            Mode::Gui
        );
        assert_eq!(without_graphics(both(None, true, None)).choose(), Mode::Gui);
    }

    #[test]
    fn a_forced_tui_runs_even_without_images() {
        assert_eq!(
            without_graphics(both(Some(Mode::Tui), true, None)).choose(),
            Mode::Tui
        );
    }

    #[test]
    fn a_unix_terminal_gets_the_tui() {
        assert_eq!(both(None, true, None).choose(), Mode::Tui);
    }

    #[test]
    fn unix_without_a_terminal_gets_the_gui() {
        assert_eq!(both(None, false, None).choose(), Mode::Gui);
    }

    #[test]
    fn a_double_clicked_windows_exe_gets_the_gui_and_frees_its_console() {
        let startup = both(None, false, Some(1));
        assert_eq!(startup.choose(), Mode::Gui);
        assert!(startup.should_free_console(Mode::Gui));
    }

    #[test]
    fn a_windows_shell_gets_the_tui() {
        let startup = both(None, true, Some(2));
        assert_eq!(startup.choose(), Mode::Tui);
        assert!(!startup.should_free_console(Mode::Tui));
    }

    #[test]
    fn flags_override_detection() {
        assert_eq!(both(Some(Mode::Gui), true, None).choose(), Mode::Gui);
        assert_eq!(both(Some(Mode::Tui), false, Some(1)).choose(), Mode::Tui);
        // A forced GUI from a shell keeps the shell's console.
        let startup = both(Some(Mode::Gui), true, Some(3));
        assert!(!startup.should_free_console(startup.choose()));
    }

    #[test]
    fn a_single_frontend_build_ignores_flags_and_detection() {
        let tui_only = Startup {
            gui_compiled: false,
            ..both(Some(Mode::Gui), false, Some(1))
        };
        assert_eq!(tui_only.choose(), Mode::Tui);

        let gui_only = Startup {
            tui_compiled: false,
            ..both(Some(Mode::Tui), true, Some(2))
        };
        assert_eq!(gui_only.choose(), Mode::Gui);
    }
}
