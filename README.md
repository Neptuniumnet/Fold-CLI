<div align="center">

<pre align="center">
███████╗ ██████╗ ██╗     ██████╗
██╔════╝██╔═══██╗██║     ██╔══██╗
█████╗  ██║   ██║██║     ██║  ██║
██╔══╝  ██║   ██║██║     ██║  ██║
██║     ╚██████╔╝███████╗██████╔╝
╚═╝      ╚═════╝ ╚══════╝╚═════╝
</pre>

### Fast Organized Links & Directories

A keyboard-first terminal file manager built for speed, simplicity, and efficient filesystem navigation.

<img src="https://img.shields.io/badge/interface-cli-262622?style=for-the-badge&logo=gnometerminal&logoColor=262622&logoSize=auto&color=e2e2d3" />
<img src="https://img.shields.io/badge/platform-linux-262622?style=for-the-badge&logo=linux&logoColor=262622&logoSize=auto&color=e2e2d3" />
<img src="https://img.shields.io/badge/version-0.1.1-262622?style=for-the-badge&logo=github&logoColor=262622&logoSize=auto&color=e2e2d3" />
<img src="https://img.shields.io/badge/built_with-rust-262622?style=for-the-badge&logo=rust&logoColor=262622&logoSize=auto&color=e2e2d3" />

</div>

---

## About

**FOLD** is a command-line file manager designed to make navigating and organizing files effortless.
One calm pane, no mouse required, vim-style keys, and a confirmation before anything destructive.

Built with a terminal-first philosophy, FOLD aims to provide a simple, efficient, and distraction-free
file management experience. The UI is inspired by the
[Text theme from Spicetify](https://github.com/spicetify/spicetify-themes/tree/master/text).

## Features

- **Fast navigation** — move with the arrow keys or `j` / `k`, jump with `g` / `G`, and page through long folders.
- **File operations** — copy, move, rename, and delete, with a confirmation dialog and undo for the last operation.
- **Search and filter** — filter the current folder by name (`/`) or search text inside files (`f`).
- **Bookmarks and settings** — save favourite folders and your chosen theme/icons; both persist between runs.
- **Clipboard paths** — copy selected paths (`y`) or the current folder path (`Y`).
- **Themes** — Catppuccin Mocha, Osaka Jane, One Dark, Gruvbox, Matugen / System, Spotify, Tokyo Night, Rose Pine, and Dracula.
- **Image previews** — full-resolution images in Kitty, and a built-in Braille preview in other terminals.
- **Optional icons** — with readable ASCII fallbacks.
- **Matugen integration** — derive a Material You palette from a wallpaper with `--matugen`.

## Previews

<p align="center">
  <img src="github/assets/mocha.png" width="49%" alt="Catppuccin Mocha theme" />
  <img src="github/assets/osakajane.png" width="49%" alt="Osaka Jane theme" />
</p>
<p align="center">
  <img src="github/assets/gruvbox.png" width="49%" alt="Gruvbox theme" />
  <img src="github/assets/matugen.png" width="49%" alt="Matugen theme" />
</p>

> [!IMPORTANT]
> The look of FOLD may change between releases; some previews are from an earlier version.

## Installation

### From source

Install Rust 1.75 or newer, then run:

```sh
git clone https://github.com/Neptunium-Laboratory/Fold-CLI.git
cd Fold-CLI
cargo build --release
```

The executable is `target/release/foldcli`. Install it to `~/.cargo/bin` with:

```sh
cargo install --path .
```

After installation, launch it with `foldcli`. See [CreatefromSource.md](CreatefromSource.md) for a
step-by-step guide and runtime dependencies.

### Arch Linux

An AUR package recipe is included in [`AUR/`](AUR/README.md). It targets the `v0.1.1` tag and installs
the `foldcli` binary, documentation, and GPL-3 license.

## Usage

Open the current directory or choose a starting directory:

```sh
foldcli
foldcli /home/you/Documents
foldcli --path /home/you/Documents
```

When running from the repository, Cargo's `--` passes arguments to FOLD:

```sh
cargo run -- --path /home/you/Documents
```

`cargo run --path ...` is a Cargo option form and does not pass `--path` to FOLD; use
`cargo run -- --path ...`.

## Keys

| Key | Action |
|---|---|
| `↑` / `↓` or `k` / `j` | Move through files |
| `PgUp` / `PgDn` | Scroll a page |
| `g` / `G` | Jump to the top / bottom |
| `Enter` | Open a folder, preview a file, or launch it with the system app |
| `→` or `l` | Enter a folder (never launches files) |
| `Backspace`, `←`, or `h` | Go to the parent folder |
| `Space` | Select an item |
| `a` | Select all / none |
| `c` / `m` | Mark selected items to copy / move |
| `p` | Paste into the current folder |
| `r` | Rename the item under the cursor |
| `d` or `Delete` | Delete (asks for confirmation) |
| `u` | Undo the last copy, move, rename, or delete |
| `y` | Copy selected item paths to the clipboard |
| `Y` | Copy the current folder path |
| `f` | Search text inside files under the current folder |
| `/` | Filter by filename |
| `b` / `B` | Bookmark this folder / open bookmarks |
| `.` | Show or hide hidden files |
| `s` | Settings; choose a theme or toggle icons |
| `Esc` | Close the current panel or clear a pending selection/filter |
| `?` / `q` | Help / quit |

Click the FOLD logo in settings lol :). `Ctrl+C` exits and restores the terminal.

## Themes and options

Choose a theme in Settings. `Matugen / System` follows your terminal palette. To derive a palette
from a wallpaper using Matugen, install Matugen and run:

```sh
foldcli --matugen ~/Pictures/wallpaper.jpg
```

A Spicetify-style `color.ini` can be loaded with `--theme PATH` or `FOLD_THEME=PATH`. An example is
in [`examples/spicetify-text-like.ini`](examples/spicetify-text-like.ini). Run `foldcli --help` for all
command options.

## Configuration

Settings are saved under `$XDG_CONFIG_HOME/fold/settings.json` (or `~/.config/fold/settings.json`).
Bookmarks are saved alongside them in `bookmarks.json`.

Path copying uses `wl-copy`, `xclip`, or `xsel` (whichever is installed). Opening files uses
`xdg-open`, so `xdg-utils` should be installed.

## Documentation

- [Build and install from source](CreatefromSource.md)
- [AUR packaging guide](AUR/README.md)
- [Version history](Updates)

## License

FOLD is free software: you can redistribute it and/or modify it under the terms of the
[GNU General Public License, version 3 or later](LICENSE). It is distributed in the hope that it
will be useful, but **without any warranty**; see the license for details.

---

<div align="center">

**FOLD — Built for the terminal.**

Made by Neptunium

</div>
