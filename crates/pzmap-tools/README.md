# pzmap-tools

Tools for Project Zomboid game files, built on `libpzmap`.

## pack-view

pack-view lets you look through the sprites in the game's texture packs. A texture pack is a `.pack` file in the game's `media/texturepacks` folder. Each one holds many sprites, arranged on a few large images called pages.

pack-view lists the pages in a pack and the sprites on each page. It shows a preview of the sprite you select, with its position on the page, its size, and its original size before the empty edges were trimmed. You can search sprites by name, zoom in, and change the background drawn behind transparent pixels. pack-view only reads the game files. It never changes them.

pack-view can run in a window or inside a terminal. In a terminal it draws real pixels, which needs a terminal that supports the kitty graphics protocol: kitty, Ghostty, WezTerm or Konsole.

### Starting pack-view

On Windows, double-click `pack-view.exe`. A console window flashes for a moment, then the pack-view window opens. You can also drag a `.pack` file or a folder onto `pack-view.exe` to open it.

From a terminal, run:

```
pack-view [PATH] [--pack NAME] [--tui | --gui] [--force-graphics]
```

`PATH` is a `.pack` file or a folder of them. Leave it out to open the texture packs of the installed game.

In the window, you can also open a pack or a folder from the File menu, or drop one onto the window.

### Finding the game

When you don't give a path, pack-view looks for Project Zomboid in your Steam libraries. It finds Steam in the usual places:

- On Windows, it asks the registry where Steam is installed, and also tries `C:\Program Files (x86)\Steam`.
- On Linux, it tries `~/.steam/steam`, `~/.local/share/Steam` and the Flatpak version of Steam.
- On macOS, it tries `~/Library/Application Support/Steam`.

It also checks every extra Steam library you have added, for example on another drive.

If the game is somewhere else, set the `PZ_INSTALL_DIR` environment variable to the game folder, the one that contains the `media` folder. pack-view checks it first. You can also just pass the folder's path as `PATH`.

If pack-view finds one copy of the game, it opens it. If it finds several, it asks which one to open each time. If it finds none, it says where it looked.

### Options

- `--pack NAME` picks which pack to open first, when you open a folder.
- `--tui` runs in the terminal.
- `--gui` opens a window.
- `--force-graphics` draws in a terminal that pack-view does not recognize as supporting kitty graphics.

### Window or terminal

Without `--tui` or `--gui`, pack-view runs in the terminal only when you start it from a terminal that can show its images: kitty, Ghostty, WezTerm or Konsole. Everywhere else it opens a window, including when you double-click it and when you start it from Command Prompt, PowerShell, Windows Terminal or macOS Terminal.

To use the terminal in a terminal pack-view doesn't recognize, pass `--tui --force-graphics`.

If the window cannot open, for example over SSH with no display, pack-view runs in the terminal instead when it can.

### Keys

These keys work in both the window and the terminal:

- `/` searches sprite names.
- `n` and `N` go to the next and previous search result.
- Up and down move through the list.
- `space` switches between the whole page and the selected sprite.
- `+` and `-` zoom in and out.
- `0` fits the sprite to the view, and `1` shows it at its real size.
- `b` changes the background between a checkerboard, black and magenta.
- `p` lists the other packs in the same folder, and `[` and `]` step through them.

On the page, the selected sprite has a yellow outline.

In the window:

- Left and right switch between the pages list and the sprites list.
- The zoom keys also zoom the whole page, which starts scaled to fit the window.
- File, Choose install lists the installed copies of the game again.

In the terminal:

- `tab` switches between the pages list and the sprites list.
- `enter` shows the selected sprite.
- `i` lists the installed copies of the game.
- `q` quits.

### Building

pack-view has two Cargo features, one for each way of running:

- `gui` builds the window, with eframe and native file dialogs.
- `tui` builds the terminal version, with ratatui.

Both are on by default. To build only one, use `--no-default-features --features tui` or `--no-default-features --features gui`. The terminal-only build is much smaller: about 1.5 MB stripped, against about 12 MB with the window.

## Third-party assets

The window uses the Fira Code font, included unmodified in `assets/fonts/FiraCode-Regular.ttf`. It is licensed under the SIL Open Font License 1.1, in `assets/fonts/OFL.txt`. The terminal version does not use it.

## License

The code is MIT OR Apache-2.0. The bundled font is OFL-1.1.

The licenses of the crates and the font compiled into pack-view are in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

Project Zomboid is a trademark of The Indie Stone. This project is not affiliated with or endorsed
by The Indie Stone, and it does not include any game files.
