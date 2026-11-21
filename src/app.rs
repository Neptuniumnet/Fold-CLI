//!
//! `App` owns everything that changes while FOLD runs: which folder we're in,
//! where the cursor is, which dialog is open, and so on. `on_key` is the one
//! entry point for keyboard input. Drawing lives in `ui.rs` and only *reads*
//! this state.

use crate::dir::{self, Entry};
use crate::fsops::{self, Op, PastePlan};
use crate::input::{self, Action};
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static UNDO_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub text: String,
    pub level: Level,
}

/// A single-line text box with an editable cursor.
#[derive(Clone, Debug, Default)]
pub struct Input {
    chars: Vec<char>,
    pos: usize,
}

impl Input {
    pub fn new(text: &str) -> Input {
        let chars: Vec<char> = text.chars().collect();
        let pos = chars.len();
        Input { chars, pos }
    }
    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }
    pub fn before_cursor(&self) -> String {
        self.chars[..self.pos].iter().collect()
    }
    /// Returns true if the text changed.
    pub fn handle_key(&mut self, key: &KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.chars.insert(self.pos, c);
                self.pos += 1;
                true
            }
            KeyCode::Backspace if self.pos > 0 => {
                self.pos -= 1;
                self.chars.remove(self.pos);
                true
            }
            KeyCode::Delete if self.pos < self.chars.len() => {
                self.chars.remove(self.pos);
                true
            }
            KeyCode::Left => {
                self.pos = self.pos.saturating_sub(1);
                false
            }
            KeyCode::Right => {
                self.pos = (self.pos + 1).min(self.chars.len());
                false
            }
            KeyCode::Home => {
                self.pos = 0;
                false
            }
            KeyCode::End => {
                self.pos = self.chars.len();
                false
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Clipboard {
    pub op: Op,
    pub items: Vec<PathBuf>,
}

pub enum Confirm {
    Delete(Vec<PathBuf>),
    Overwrite { op: Op, plan: PastePlan },
    Undo(String),
}

#[derive(Clone, Debug)]
enum UndoStep {
    RemoveCreated(PathBuf),
    MoveBack { from: PathBuf, to: PathBuf },
    RestoreBackup { backup: PathBuf, original: PathBuf },
}

#[derive(Clone, Debug)]
struct UndoRecord {
    label: String,
    steps: Vec<UndoStep>,
}

#[derive(Clone, Debug)]
pub struct ContentMatch {
    pub path: PathBuf,
    pub line: usize,
    pub text: String,
}

pub enum Mode {
    Normal,
    Filter(Input),
    Rename {
        input: Input,
        target: PathBuf,
        error: Option<String>,
    },
    Confirm(Confirm),
    Message {
        title: String,
        lines: Vec<String>,
    },
    Help,
    Settings {
        selected: usize,
    },
    Bookmarks {
        selected: usize,
    },
    ContentSearch(Input),
    SearchResults {
        query: String,
        matches: Vec<ContentMatch>,
        selected: usize,
    },
    TextViewer {
        title: String,
        lines: Vec<String>,
        offset: usize,
    },
    ImageViewer {
        title: String,
        image: image::DynamicImage,
    },
}

pub struct App {
    pub cwd: PathBuf,
    pub entries: Vec<Entry>,
    /// Indices into `entries` that pass the hidden/filter rules.
    pub visible: Vec<usize>,
    /// Position inside `visible`.
    pub cursor: usize,
    /// First visible row on screen (kept in sync by the UI).
    pub offset: usize,
    /// Rows per screen, for PageUp / PageDown (kept in sync by the UI).
    pub page: usize,
    pub show_hidden: bool,
    pub filter: String,
    pub selected: HashSet<PathBuf>,
    pub clipboard: Option<Clipboard>,
    pub mode: Mode,
    pub status: Option<Status>,
    pub should_quit: bool,
    pub theme: Theme,
    pub theme_choice: Option<usize>,
    saved_theme_choice: Option<usize>,
    pub bookmarks: Vec<PathBuf>,
    pub show_icons: bool,
    last_undo: Option<UndoRecord>,
    /// False = plain ASCII fallback for terminals without box drawing.
    pub unicode: bool,
}

fn bookmark_file() -> Option<PathBuf> {
    Some(config_dir()?.join("fold").join("bookmarks.json"))
}

fn settings_file() -> Option<PathBuf> {
    Some(config_dir()?.join("fold").join("settings.json"))
}

fn config_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })?;
    Some(config)
}

fn load_settings() -> (Option<usize>, bool) {
    let Some(file) = settings_file() else {
        return (None, true);
    };
    let Ok(bytes) = fs::read(file) else {
        return (None, true);
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return (None, true);
    };
    let theme = value
        .get("theme")
        .and_then(serde_json::Value::as_u64)
        .map(|index| index as usize)
        .filter(|index| *index <= 8);
    let icons = value
        .get("show_icons")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    (theme, icons)
}

fn load_bookmarks() -> Vec<PathBuf> {
    let Some(file) = bookmark_file() else {
        return Vec::new();
    };
    fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
        .unwrap_or_default()
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

fn save_bookmarks(bookmarks: &[PathBuf]) -> Result<(), String> {
    let file = bookmark_file().ok_or("HOME or XDG_CONFIG_HOME is not set")?;
    let parent = file.parent().ok_or("cannot determine config directory")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let paths: Vec<String> = bookmarks
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let data = serde_json::to_vec_pretty(&paths).map_err(|error| error.to_string())?;
    fs::write(file, data).map_err(|error| error.to_string())
}

fn save_settings(theme: Option<usize>, show_icons: bool) -> Result<(), String> {
    let file = settings_file().ok_or("HOME or XDG_CONFIG_HOME is not set")?;
    let parent = file.parent().ok_or("cannot determine config directory")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let data = serde_json::to_vec_pretty(&serde_json::json!({
        "theme": theme,
        "show_icons": show_icons,
    }))
    .map_err(|error| error.to_string())?;
    fs::write(file, data).map_err(|error| error.to_string())
}

fn theme_for_choice(index: usize) -> Theme {
    if index == 4 {
        Theme::terminal()
    } else {
        crate::theme::preset(index)
    }
}

fn undo_backup_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    loop {
        let id = UNDO_ID.fetch_add(1, Ordering::Relaxed);
        let backup = parent.join(format!(".foldcli-undo-{}-{id}", std::process::id()));
        if fs::symlink_metadata(&backup).is_err() {
            return backup;
        }
    }
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn search_contents(root: &Path, query: &str, show_hidden: bool) -> Vec<ContentMatch> {
    const MAX_RESULTS: usize = 250;
    const MAX_FILES: usize = 20_000;
    const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
    const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
    let needle = query.to_lowercase();
    let mut pending = vec![root.to_path_buf()];
    let mut matches = Vec::new();
    let mut scanned = 0usize;
    let mut total_bytes = 0u64;
    while let Some(dir_path) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir_path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if !show_hidden && name.to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(path);
                continue;
            }
            scanned += 1;
            if scanned > MAX_FILES {
                return matches;
            }
            if !kind.is_file() {
                continue;
            }
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            let size = metadata.len();
            if size > MAX_FILE_SIZE || !is_text_file(&path) {
                continue;
            }
            total_bytes = total_bytes.saturating_add(size);
            if total_bytes > MAX_TOTAL_BYTES {
                return matches;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for (index, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(&needle) {
                    matches.push(ContentMatch {
                        path: path.clone(),
                        line: index + 1,
                        text: line.to_string(),
                    });
                    if matches.len() >= MAX_RESULTS {
                        return matches;
                    }
                }
            }
        }
    }
    matches
}

fn copy_to_system_clipboard(text: &str) -> Result<(), String> {
    let commands: [(&str, &[&str]); 3] = [
        ("wl-copy", &["--type", "text/plain;charset=utf-8"]),
        ("xclip", &["-selection", "clipboard", "-i"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    for (program, args) in commands {
        let child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else { continue };
        let write_result = child
            .stdin
            .take()
            .ok_or_else(|| "clipboard process has no input".to_string())
            .and_then(|mut stdin| stdin.write_all(text.as_bytes()).map_err(|e| e.to_string()));
        let status = child.wait().map_err(|e| e.to_string())?;
        if write_result.is_ok() && status.success() {
            return Ok(());
        }
    }

    // OSC 52 is supported by many modern terminals and also works over SSH.
    let encoded = base64(text.as_bytes());
    let mut out = std::io::stdout().lock();
    write!(out, "\x1b]52;c;{encoded}\x07").map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())
}

fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        result.push(TABLE[(a >> 2) as usize] as char);
        result.push(TABLE[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        result.push(if chunk.len() > 1 {
            TABLE[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            TABLE[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    result
}

fn count(n: usize) -> String {
    if n == 1 {
        "1 item".into()
    } else {
        format!("{n} items")
    }
}

fn is_text_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "txt"
            | "md"
            | "markdown"
            | "rst"
            | "adoc"
            | "org"
            | "log"
            | "ini"
            | "toml"
            | "yaml"
            | "yml"
            | "json"
            | "xml"
            | "csv"
            | "tsv"
            | "html"
            | "css"
            | "js"
            | "ts"
            | "jsx"
            | "tsx"
            | "rs"
            | "py"
            | "sh"
            | "bash"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "go"
            | "java"
            | "kt"
            | "swift"
            | "sql"
            | "conf"
            | "cfg"
            | "env"
            | "gitignore"
            | "makefile"
    ) {
        return true;
    }
    if path.extension().is_none() {
        if let Ok(bytes) = fs::read(path) {
            return bytes.len() <= 4 * 1024 * 1024
                && std::str::from_utf8(&bytes).is_ok()
                && !bytes.contains(&0);
        }
    }
    false
}

fn is_image_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff"
    )
}

impl App {
    pub fn new(start: &Path, theme: Theme, unicode: bool) -> Result<App, String> {
        let cwd = fs::canonicalize(start)
            .map_err(|e| format!("cannot open '{}': {e}", start.display()))?;
        let listing =
            dir::list_dir(&cwd).map_err(|e| format!("cannot read '{}': {e}", cwd.display()))?;
        let (saved_theme_choice, show_icons) = load_settings();
        let theme = saved_theme_choice.map(theme_for_choice).unwrap_or(theme);
        let mut app = App {
            cwd,
            entries: listing.entries,
            visible: Vec::new(),
            cursor: 0,
            offset: 0,
            page: 10,
            show_hidden: false,
            filter: String::new(),
            selected: HashSet::new(),
            clipboard: None,
            mode: Mode::Normal,
            status: None,
            should_quit: false,
            theme,
            theme_choice: saved_theme_choice,
            saved_theme_choice,
            bookmarks: load_bookmarks(),
            show_icons,
            last_undo: None,
            unicode,
        };
        app.refresh_visible(None, true);
        if listing.skipped > 0 {
            app.warn(format!("{} entries could not be read", listing.skipped));
        }
        Ok(app)
    }

    // ----- small helpers ---------------------------------------------------

    pub fn info(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            level: Level::Info,
        });
    }
    pub fn warn(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            level: Level::Warn,
        });
    }
    pub fn error(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            level: Level::Error,
        });
    }

    pub fn current_entry(&self) -> Option<&Entry> {
        self.visible.get(self.cursor).map(|&i| &self.entries[i])
    }

    /// Entries that are both marked and currently on screen.
    pub fn marked_count(&self) -> usize {
        self.visible
            .iter()
            .filter(|&&i| self.selected.contains(&self.entries[i].path))
            .count()
    }

    /// What an operation applies to: the marked entries that are on screen,
    /// or just the entry under the cursor if nothing is marked.
    pub fn targets(&self) -> Vec<PathBuf> {
        let marked: Vec<PathBuf> = self
            .visible
            .iter()
            .map(|&i| &self.entries[i])
            .filter(|e| self.selected.contains(&e.path))
            .map(|e| e.path.clone())
            .collect();
        if !marked.is_empty() {
            return marked;
        }
        self.current_entry()
            .map(|e| vec![e.path.clone()])
            .unwrap_or_default()
    }

    /// Recompute which entries are on screen. The cursor stays on `keep` if
    /// that entry is still visible; otherwise it goes to the top
    /// (`fallback_top`) or stays near its old row.
    fn refresh_visible(&mut self, keep: Option<PathBuf>, fallback_top: bool) {
        let old = self.cursor;
        self.visible = dir::visible_indices(&self.entries, self.show_hidden, &self.filter);
        let found = keep.and_then(|p| self.visible.iter().position(|&i| self.entries[i].path == p));
        self.cursor = match found {
            Some(i) => i,
            None if fallback_top => 0,
            None => old.min(self.visible.len().saturating_sub(1)),
        };
    }

    fn current_path(&self) -> Option<PathBuf> {
        self.current_entry().map(|e| e.path.clone())
    }

    /// Re-read the current folder (after a file operation).
    fn reload(&mut self, keep: Option<PathBuf>) {
        let keep = keep.or_else(|| self.current_path());
        match dir::list_dir(&self.cwd) {
            Ok(listing) => {
                self.entries = listing.entries;
                if listing.skipped > 0 {
                    self.warn(format!("{} entries could not be read", listing.skipped));
                }
            }
            Err(e) => {
                // The folder vanished or became unreadable: climb to one that works.
                let lost = self.cwd.clone();
                let recovered = lost
                    .ancestors()
                    .skip(1)
                    .find_map(|a| dir::list_dir(a).ok().map(|l| (a.to_path_buf(), l.entries)));
                match recovered {
                    Some((path, entries)) => {
                        self.cwd = path;
                        self.entries = entries;
                        self.filter.clear();
                        self.selected.clear();
                        self.error(format!(
                            "'{}' is no longer readable ({e}); moved up",
                            fsops::display_name(&lost)
                        ));
                    }
                    None => self.error(format!("Cannot read this folder: {e}")),
                }
            }
        }
        let present: HashSet<&PathBuf> = self.entries.iter().map(|e| &e.path).collect();
        self.selected.retain(|p| present.contains(p));
        self.refresh_visible(keep, false);
    }

    pub fn ensure_cursor_visible(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        self.page = height;
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + height {
            self.offset = self.cursor + 1 - height;
        }
        self.offset = self.offset.min(self.visible.len().saturating_sub(height));
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }

    // ----- navigation ------------------------------------------------------

    /// Enter a folder. On failure we stay where we are and show why.
    pub fn change_dir(&mut self, target: PathBuf, select: Option<PathBuf>) {
        let name = fsops::display_name(&target);
        let canon = match fs::canonicalize(&target) {
            Ok(p) => p,
            Err(e) => {
                self.error(format!("Cannot open '{name}': {e}"));
                return;
            }
        };
        match dir::list_dir(&canon) {
            Ok(listing) => {
                let skipped = listing.skipped;
                self.cwd = canon;
                self.entries = listing.entries;
                self.filter.clear();
                self.selected.clear();
                self.cursor = 0;
                self.offset = 0;
                self.refresh_visible(select, true);
                if skipped > 0 {
                    self.warn(format!("{skipped} entries could not be read"));
                }
            }
            Err(e) => self.error(format!("Cannot open '{name}': {e}")),
        }
    }

    fn go_parent(&mut self) {
        match dir::parent_dir(&self.cwd) {
            Some(parent) => {
                let from = self.cwd.clone();
                self.change_dir(parent, Some(from));
            }
            None => self.info("Already at the root"),
        }
    }

    fn open_current(&mut self, dirs_only: bool) {
        let Some(entry) = self.current_entry().cloned() else {
            return;
        };
        if entry.is_dir {
            self.change_dir(entry.path, None);
        } else if entry.broken_link {
            self.error(format!("'{}' is a broken symbolic link", entry.display));
        } else if !dirs_only {
            if is_text_file(&entry.path) {
                match fs::read_to_string(&entry.path) {
                    Ok(text) => {
                        self.mode = Mode::TextViewer {
                            title: entry.display.clone(),
                            lines: text.lines().map(crate::dir::sanitize).collect(),
                            offset: 0,
                        }
                    }
                    Err(e) => self.error(format!("Cannot read '{}': {e}", entry.display)),
                }
            } else if is_image_file(&entry.path) {
                match image::open(&entry.path) {
                    Ok(image) => {
                        self.mode = Mode::ImageViewer {
                            title: entry.display.clone(),
                            image,
                        }
                    }
                    Err(e) => self.error(format!("Cannot view '{}': {e}", entry.display)),
                }
            } else {
                self.open_file(&entry);
            }
        }
    }

    /// Hand a file to the system's default application (`xdg-open`, or the
    /// program named in `FOLD_OPENER`). FOLD doesn't wait for it.
    fn open_file(&mut self, entry: &Entry) {
        let opener = std::env::var("FOLD_OPENER")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "xdg-open".to_string());
        let spawned = Command::new(&opener)
            .arg(&entry.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                // Reap the child in the background so it can't become a zombie.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                self.info(format!("Opening {}…", entry.display));
            }
            Err(e) => self.error(format!("Could not run '{opener}': {e}")),
        }
    }

    // ----- keyboard --------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        if input::is_interrupt(&key) {
            self.should_quit = true;
            return;
        }
        self.status = None; // messages last until the next key press
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.on_normal(key),
            Mode::Filter(text) => self.on_filter(key, text),
            Mode::Rename {
                input,
                target,
                error,
            } => self.on_rename(key, input, target, error),
            Mode::Confirm(c) => self.on_confirm(key, c),
            Mode::TextViewer {
                title,
                lines,
                mut offset,
            } => {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {}
                    KeyCode::Up | KeyCode::Char('k') => offset = offset.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        offset = (offset + 1).min(lines.len().saturating_sub(1))
                    }
                    KeyCode::PageUp => offset = offset.saturating_sub(10),
                    KeyCode::PageDown => offset = (offset + 10).min(lines.len().saturating_sub(1)),
                    KeyCode::Home => offset = 0,
                    KeyCode::End => offset = lines.len().saturating_sub(1),
                    _ => {}
                }
                if !matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                    self.mode = Mode::TextViewer {
                        title,
                        lines,
                        offset,
                    };
                }
            }
            Mode::ImageViewer { title, image } => {
                if key.code != KeyCode::Esc && key.code != KeyCode::Char('q') {
                    self.mode = Mode::ImageViewer { title, image };
                }
            }
            Mode::Settings { selected } => self.on_settings(key, selected),
            Mode::Bookmarks { selected } => self.on_bookmarks(key, selected),
            Mode::ContentSearch(mut input) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    let query = input.value().trim().to_string();
                    if !query.is_empty() {
                        self.run_content_search(query);
                    }
                }
                _ => {
                    input.handle_key(&key);
                    self.mode = Mode::ContentSearch(input);
                }
            },
            Mode::SearchResults {
                query,
                matches,
                selected,
            } => self.on_search_results(key, query, matches, selected),
            Mode::Message { .. } | Mode::Help => {} // any key closes it
        }
    }

    fn on_normal(&mut self, key: KeyEvent) {
        let Some(action) = input::normal_action(key) else {
            return;
        };
        match action {
            Action::Quit => self.should_quit = true,
            Action::Up => self.move_cursor(-1),
            Action::Down => self.move_cursor(1),
            Action::PageUp => self.move_cursor(-(self.page as isize)),
            Action::PageDown => self.move_cursor(self.page as isize),
            Action::Top => self.cursor = 0,
            Action::Bottom => self.cursor = self.visible.len().saturating_sub(1),
            Action::Open => self.open_current(false),
            Action::Descend => self.open_current(true),
            Action::Parent => self.go_parent(),
            Action::ToggleSelect => {
                if let Some(path) = self.current_path() {
                    if !self.selected.remove(&path) {
                        self.selected.insert(path);
                    }
                    self.move_cursor(1);
                }
            }
            Action::SelectAll => self.toggle_select_all(),
            Action::Copy => self.start_clipboard(Op::Copy),
            Action::Move => self.start_clipboard(Op::Move),
            Action::Rename => self.start_rename(),
            Action::Delete => {
                let targets = self.targets();
                if !targets.is_empty() {
                    self.mode = Mode::Confirm(Confirm::Delete(targets));
                }
            }
            Action::Paste => self.paste(),
            Action::FilterStart => self.mode = Mode::Filter(Input::new(&self.filter)),
            Action::Escape => self.escape(),
            Action::ToggleHidden => {
                self.show_hidden = !self.show_hidden;
                let keep = self.current_path();
                self.refresh_visible(keep, true);
                self.info(if self.show_hidden {
                    "Hidden files: shown"
                } else {
                    "Hidden files: hidden"
                });
            }
            Action::Help => self.mode = Mode::Help,
            Action::Settings => self.mode = Mode::Settings { selected: 0 },
            Action::BookmarkAdd => self.add_bookmark(),
            Action::BookmarksOpen => self.mode = Mode::Bookmarks { selected: 0 },
            Action::CopyAddress => self.copy_addresses(),
            Action::CopyDirectoryAddress => self.copy_directory_address(),
            Action::ContentSearch => self.mode = Mode::ContentSearch(Input::default()),
            Action::Undo => match &self.last_undo {
                Some(record) => self.mode = Mode::Confirm(Confirm::Undo(record.label.clone())),
                None => self.info("Nothing to undo"),
            },
        }
    }

    fn run_content_search(&mut self, query: String) {
        let matches = search_contents(&self.cwd, &query, self.show_hidden);
        self.mode = Mode::SearchResults {
            query,
            matches,
            selected: 0,
        };
    }

    fn on_search_results(
        &mut self,
        key: KeyEvent,
        query: String,
        matches: Vec<ContentMatch>,
        mut selected: usize,
    ) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return,
            KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                selected = (selected + 1).min(matches.len().saturating_sub(1));
            }
            KeyCode::Enter if !matches.is_empty() => {
                let found = &matches[selected];
                match fs::read_to_string(&found.path) {
                    Ok(text) => {
                        self.mode = Mode::TextViewer {
                            title: found.path.file_name().map_or_else(
                                || found.path.display().to_string(),
                                |name| name.to_string_lossy().into_owned(),
                            ),
                            lines: text.lines().map(crate::dir::sanitize).collect(),
                            offset: found.line.saturating_sub(1),
                        };
                    }
                    Err(error) => self.error(format!("Cannot open search result: {error}")),
                }
                return;
            }
            _ => {}
        }
        self.mode = Mode::SearchResults {
            query,
            matches,
            selected,
        };
    }

    fn copy_addresses(&mut self) {
        let targets = self.targets();
        let addresses = if targets.is_empty() {
            vec![self.cwd.clone()]
        } else {
            targets
        };
        let text = addresses
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n");
        match copy_to_system_clipboard(&text) {
            Ok(()) => self.info(if addresses.len() == 1 {
                "Copied path to clipboard"
            } else {
                "Copied paths to clipboard"
            }),
            Err(error) => self.error(format!("Could not copy path: {error}")),
        }
    }

    fn copy_directory_address(&mut self) {
        match copy_to_system_clipboard(&self.cwd.to_string_lossy()) {
            Ok(()) => self.info("Copied current folder path to clipboard"),
            Err(error) => self.error(format!("Could not copy path: {error}")),
        }
    }

    pub fn open_logo_link(&mut self) {
        match Command::new("xdg-open")
            .arg("https://www.myinstants.com/en/instant/m-e-o-w-82698/")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                self.info("Opening FOLD's meow page…");
            }
            Err(error) => self.error(format!("Could not open web browser: {error}")),
        }
    }

    fn add_bookmark(&mut self) {
        if self.bookmarks.contains(&self.cwd) {
            self.info("Folder is already bookmarked");
            return;
        }
        self.bookmarks.push(self.cwd.clone());
        match save_bookmarks(&self.bookmarks) {
            Ok(()) => self.info("Bookmarked current folder"),
            Err(error) => self.warn(format!("Bookmark saved for this run only: {error}")),
        }
    }

    fn on_bookmarks(&mut self, key: KeyEvent, mut selected: usize) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return,
            KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                selected = (selected + 1).min(self.bookmarks.len().saturating_sub(1));
            }
            KeyCode::Enter if !self.bookmarks.is_empty() => {
                self.change_dir(self.bookmarks[selected].clone(), None);
                return;
            }
            KeyCode::Char('d') | KeyCode::Delete if !self.bookmarks.is_empty() => {
                self.bookmarks.remove(selected);
                if let Err(error) = save_bookmarks(&self.bookmarks) {
                    self.warn(format!("Bookmark removed for this run only: {error}"));
                } else {
                    self.info("Bookmark removed");
                }
                selected = selected.min(self.bookmarks.len().saturating_sub(1));
            }
            KeyCode::Char('a') => self.add_bookmark(),
            _ => {}
        }
        self.mode = Mode::Bookmarks { selected };
    }

    fn on_settings(&mut self, key: KeyEvent, mut selected: usize) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return,
            KeyCode::Up | KeyCode::Char('k') => {
                selected = if selected == 9 {
                    6
                } else {
                    selected.saturating_sub(3)
                };
            }
            KeyCode::Down | KeyCode::Char('j') => selected = (selected + 3).min(9),
            KeyCode::Left | KeyCode::Char('h') => {
                selected = if selected == 9 {
                    8
                } else if selected % 3 == 0 {
                    selected + 2
                } else {
                    selected - 1
                };
            }
            KeyCode::Right | KeyCode::Char('l') => {
                selected = if selected == 9 {
                    0
                } else if selected % 3 == 2 {
                    selected - 2
                } else {
                    selected + 1
                };
            }
            KeyCode::Enter => {
                if selected == 9 {
                    self.show_icons = !self.show_icons;
                } else {
                    self.set_theme_choice(selected);
                }
                if let Err(error) = save_settings(self.saved_theme_choice, self.show_icons) {
                    self.warn(format!("Could not save settings: {error}"));
                } else {
                    self.info("Settings saved");
                }
            }
            _ => {}
        }
        self.mode = Mode::Settings { selected };
    }

    fn set_theme_choice(&mut self, selected: usize) {
        self.theme = theme_for_choice(selected);
        self.theme_choice = Some(selected);
        self.saved_theme_choice = Some(selected);
    }

    /// Esc in normal mode clears one thing at a time: filter, then a pending
    /// copy/move, then the selection.
    fn escape(&mut self) {
        if !self.filter.is_empty() {
            self.filter.clear();
            let keep = self.current_path();
            self.refresh_visible(keep, true);
        } else if self.clipboard.take().is_some() {
            self.info("Cancelled");
        } else {
            self.selected.clear();
        }
    }

    fn toggle_select_all(&mut self) {
        let paths: Vec<PathBuf> = self
            .visible
            .iter()
            .map(|&i| self.entries[i].path.clone())
            .collect();
        if paths.iter().all(|p| self.selected.contains(p)) {
            for p in &paths {
                self.selected.remove(p);
            }
        } else {
            self.selected.extend(paths);
        }
    }

    fn on_filter(&mut self, key: KeyEvent, mut text: Input) {
        match key.code {
            KeyCode::Enter => {} // keep the filter, back to the list
            KeyCode::Esc => {
                self.filter.clear();
                let keep = self.current_path();
                self.refresh_visible(keep, true);
            }
            KeyCode::Up => {
                self.move_cursor(-1);
                self.mode = Mode::Filter(text);
            }
            KeyCode::Down => {
                self.move_cursor(1);
                self.mode = Mode::Filter(text);
            }
            _ => {
                if text.handle_key(&key) {
                    self.filter = text.value();
                    self.refresh_visible(None, true);
                }
                self.mode = Mode::Filter(text);
            }
        }
    }

    // ----- copy / move / paste ----------------------------------------------

    fn start_clipboard(&mut self, op: Op) {
        let items = self.targets();
        if items.is_empty() {
            self.info(format!("Nothing to {}", op.label()));
            return;
        }
        let n = items.len();
        self.selected.clear();
        self.clipboard = Some(Clipboard { op, items });
        self.info(format!(
            "{} ready to {}: open the destination folder and press p",
            count(n),
            op.label()
        ));
    }

    fn paste(&mut self) {
        let Some(clip) = self.clipboard.clone() else {
            self.info("Nothing to paste: press c (copy) or m (move) first");
            return;
        };
        let plan = fsops::plan_paste(&clip.items, &self.cwd, clip.op);
        if plan.items.iter().any(|i| i.overwrite) {
            self.mode = Mode::Confirm(Confirm::Overwrite { op: clip.op, plan });
        } else {
            self.run_plan(clip.op, plan);
        }
    }

    fn run_plan(&mut self, op: Op, plan: PastePlan) {
        let mut report = fsops::Report::default();
        let mut steps = Vec::new();
        for item in &plan.items {
            let mut backup = None;
            if item.overwrite {
                let staged = undo_backup_path(&item.dst);
                match fs::rename(&item.dst, &staged) {
                    Ok(()) => {
                        backup = Some(staged.clone());
                        steps.push(UndoStep::RestoreBackup {
                            backup: staged,
                            original: item.dst.clone(),
                        });
                    }
                    Err(error) => {
                        report.errors.push(format!(
                            "{}: could not stage the item being replaced: {error}",
                            fsops::display_name(&item.dst)
                        ));
                        continue;
                    }
                }
            }
            let result = match op {
                Op::Copy => fsops::copy_path(&item.src, &item.dst),
                Op::Move => fsops::move_path(&item.src, &item.dst),
            };
            match result {
                Ok(()) => {
                    report.done += 1;
                    match op {
                        Op::Copy => steps.push(UndoStep::RemoveCreated(item.dst.clone())),
                        Op::Move => steps.push(UndoStep::MoveBack {
                            from: item.dst.clone(),
                            to: item.src.clone(),
                        }),
                    }
                }
                Err(error) => {
                    if path_exists(&item.dst) && (op == Op::Copy || path_exists(&item.src)) {
                        let _ = fsops::delete_path(&item.dst);
                    }
                    if let Some(staged) = backup {
                        if fs::rename(&staged, &item.dst).is_ok() {
                            steps.pop();
                        } else {
                            report.errors.push(format!(
                                "{}: operation failed ({error}); original is preserved at {}",
                                fsops::display_name(&item.dst),
                                staged.display()
                            ));
                            continue;
                        }
                    }
                    report.errors.push(error.to_string());
                }
            }
        }
        if !steps.is_empty() {
            self.install_undo(UndoRecord {
                label: format!("{} {}", op.past().to_ascii_lowercase(), count(report.done)),
                steps,
            });
        }
        let first = plan.items.first().map(|i| i.dst.clone());
        if op == Op::Move {
            self.clipboard = None;
        }
        self.reload(first);
        let mut problems = plan.problems;
        problems.extend(report.errors);
        self.report(op.past(), report.done, problems);
    }

    /// Tell the user how an operation went. Problems get a dialog so none
    /// are missed; plain success is just a status line.
    fn report(&mut self, verb: &str, done: usize, problems: Vec<String>) {
        if problems.is_empty() {
            self.info(format!("{verb} {}", count(done)));
        } else {
            let n = problems.len();
            self.mode = Mode::Message {
                title: format!(
                    "{verb} {done}, {n} problem{}",
                    if n == 1 { "" } else { "s" }
                ),
                lines: problems,
            };
        }
    }

    // ----- rename -----------------------------------------------------------

    fn start_rename(&mut self) {
        let Some(entry) = self.current_entry() else {
            return;
        };
        self.mode = Mode::Rename {
            input: Input::new(&entry.display),
            target: entry.path.clone(),
            error: None,
        };
    }

    fn on_rename(
        &mut self,
        key: KeyEvent,
        mut input: Input,
        target: PathBuf,
        _error: Option<String>,
    ) {
        match key.code {
            KeyCode::Esc => {}
            KeyCode::Enter => {
                let new_name = input.value();
                // A name with invalid UTF-8 can't be typed back exactly; if the
                // text is unchanged, treat it as "no change" instead of renaming
                // the file to its lossy spelling.
                let untouched_lossy = self
                    .entries
                    .iter()
                    .any(|e| e.path == target && e.lossy && e.display == new_name);
                if untouched_lossy {
                    self.info("Name unchanged");
                    return;
                }
                match fsops::rename_path(&target, &new_name) {
                    Ok(new_path) => {
                        self.install_undo(UndoRecord {
                            label: "rename".into(),
                            steps: vec![UndoStep::MoveBack {
                                from: new_path.clone(),
                                to: target.clone(),
                            }],
                        });
                        self.reload(Some(new_path));
                        self.info(format!("Renamed to {}", dir::sanitize(&new_name)));
                    }
                    Err(msg) => {
                        self.mode = Mode::Rename {
                            input,
                            target,
                            error: Some(msg),
                        }
                    }
                }
            }
            _ => {
                input.handle_key(&key);
                self.mode = Mode::Rename {
                    input,
                    target,
                    error: None,
                };
            }
        }
    }

    // ----- confirmation dialogs ---------------------------------------------

    /// `y` confirms. Everything else (n, Esc, Enter, stray keys that aren't
    /// `y`) cancels or is ignored, so an accidental Enter can never delete.
    fn on_confirm(&mut self, key: KeyEvent, confirm: Confirm) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => self.apply_confirm(confirm),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Enter => {
                self.info("Cancelled");
            }
            _ => self.mode = Mode::Confirm(confirm),
        }
    }

    fn apply_confirm(&mut self, confirm: Confirm) {
        match confirm {
            Confirm::Delete(paths) => {
                let mut done = 0;
                let mut errors = Vec::new();
                let mut steps = Vec::new();
                for path in &paths {
                    let backup = undo_backup_path(path);
                    match fs::rename(path, &backup) {
                        Ok(()) => {
                            done += 1;
                            steps.push(UndoStep::RestoreBackup {
                                backup,
                                original: path.clone(),
                            });
                        }
                        Err(error) => errors.push(format!("{}: {error}", path.display())),
                    }
                }
                if !steps.is_empty() {
                    self.install_undo(UndoRecord {
                        label: format!("delete {}", count(done)),
                        steps,
                    });
                }
                self.selected.clear();
                self.reload(None);
                self.report("Deleted (undo available)", done, errors);
            }
            Confirm::Overwrite { op, plan } => self.run_plan(op, plan),
            Confirm::Undo(_) => self.undo_last(),
        }
    }

    fn install_undo(&mut self, record: UndoRecord) {
        self.discard_undo_backups();
        self.last_undo = Some(record);
    }

    fn discard_undo_backups(&mut self) {
        if let Some(record) = self.last_undo.take() {
            for step in record.steps {
                if let UndoStep::RestoreBackup { backup, .. } = step {
                    if path_exists(&backup) {
                        let _ = fsops::delete_path(&backup);
                    }
                }
            }
        }
    }

    pub fn undo_available(&self) -> bool {
        self.last_undo.is_some()
    }

    pub fn cleanup_undo_backups(&mut self) {
        self.discard_undo_backups();
    }

    fn undo_last(&mut self) {
        let Some(mut record) = self.last_undo.take() else {
            self.info("Nothing to undo");
            return;
        };
        while let Some(step) = record.steps.pop() {
            let result = match &step {
                UndoStep::RemoveCreated(path) => {
                    if path_exists(path) {
                        fsops::delete_path(path)
                    } else {
                        Ok(())
                    }
                }
                UndoStep::MoveBack { from, to } => {
                    if path_exists(to) {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::AlreadyExists,
                            format!("{} already exists", to.display()),
                        ))
                    } else {
                        fsops::move_path(from, to)
                    }
                }
                UndoStep::RestoreBackup { backup, original } => {
                    if path_exists(original) {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::AlreadyExists,
                            format!("{} already exists", original.display()),
                        ))
                    } else {
                        fsops::move_path(backup, original)
                    }
                }
            };
            if let Err(error) = result {
                record.steps.push(step);
                self.last_undo = Some(record);
                self.error(format!("Undo stopped: {error}"));
                self.reload(None);
                return;
            }
        }
        let label = record.label;
        self.reload(None);
        self.info(format!("Undid {label}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn app_in(dir: &Path) -> App {
        App::new(dir, Theme::builtin(true), true).unwrap()
    }
    fn press(app: &mut App, c: char) {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    fn code(app: &mut App, c: KeyCode) {
        app.on_key(KeyEvent::new(c, KeyModifiers::NONE));
    }
    fn typed(app: &mut App, s: &str) {
        for c in s.chars() {
            press(app, c);
        }
    }
    fn shown(app: &App) -> Vec<String> {
        app.visible
            .iter()
            .map(|&i| app.entries[i].display.clone())
            .collect()
    }
    fn cursor_name(app: &App) -> String {
        app.current_entry().unwrap().display.clone()
    }

    fn sample() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("docs")).unwrap();
        fs::write(d.path().join("docs/readme.md"), "r").unwrap();
        fs::write(d.path().join("a.txt"), "a").unwrap();
        fs::write(d.path().join("b.txt"), "b").unwrap();
        fs::write(d.path().join(".hidden"), "h").unwrap();
        d
    }

    #[test]
    fn open_folder_then_parent_restores_cursor() {
        let d = sample();
        let mut app = app_in(d.path());
        assert_eq!(shown(&app), ["docs", "a.txt", "b.txt"]);
        assert_eq!(cursor_name(&app), "docs");
        code(&mut app, KeyCode::Enter);
        assert_eq!(app.cwd, fs::canonicalize(d.path().join("docs")).unwrap());
        assert_eq!(shown(&app), ["readme.md"]);
        code(&mut app, KeyCode::Backspace);
        assert_eq!(cursor_name(&app), "docs");
    }

    #[test]
    fn cursor_is_clamped_and_empty_folders_are_safe() {
        let d = sample();
        fs::create_dir(d.path().join("empty")).unwrap();
        let mut app = app_in(d.path());
        code(&mut app, KeyCode::Up);
        assert_eq!(app.cursor, 0);
        for _ in 0..20 {
            code(&mut app, KeyCode::Down);
        }
        assert_eq!(app.cursor, app.visible.len() - 1);
        press(&mut app, 'g');
        assert_eq!(cursor_name(&app), "docs");
        code(&mut app, KeyCode::Enter); // docs
        code(&mut app, KeyCode::Backspace);
        // enter "empty" and poke every key in an empty folder
        while cursor_name(&app) != "empty" {
            code(&mut app, KeyCode::Down);
        }
        code(&mut app, KeyCode::Enter);
        assert!(app.visible.is_empty());
        for c in ['j', 'k', 'd', 'r', 'c', 'm', 'p', ' ', 'a', 'G'] {
            press(&mut app, c);
        }
        code(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn parent_of_root_is_a_message_not_a_crash() {
        let mut app = app_in(Path::new("/"));
        code(&mut app, KeyCode::Backspace);
        assert_eq!(app.cwd, PathBuf::from("/"));
        assert!(app.status.is_some());
    }

    #[test]
    fn failing_to_open_a_folder_shows_an_error_and_stays_put() {
        let d = sample();
        let mut app = app_in(d.path());
        let before = app.cwd.clone();
        app.change_dir(d.path().join("does-not-exist"), None);
        assert_eq!(app.cwd, before);
        assert_eq!(app.status.as_ref().unwrap().level, Level::Error);
    }

    #[test]
    fn hidden_files_toggle() {
        let d = sample();
        let mut app = app_in(d.path());
        assert!(!shown(&app).contains(&".hidden".to_string()));
        press(&mut app, '.');
        assert!(shown(&app).contains(&".hidden".to_string()));
        press(&mut app, '.');
        assert!(!shown(&app).contains(&".hidden".to_string()));
    }

    #[test]
    fn filter_typing_enter_and_escape() {
        let d = sample();
        let mut app = app_in(d.path());
        press(&mut app, '/');
        assert!(matches!(app.mode, Mode::Filter(_)));
        // 'q' and 'd' are plain text while typing, not commands.
        typed(&mut app, "b.t");
        assert_eq!(shown(&app), ["b.txt"]);
        assert!(!app.should_quit);
        code(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.filter, "b.t");
        assert_eq!(shown(&app), ["b.txt"]);
        code(&mut app, KeyCode::Esc); // clears the active filter
        assert_eq!(shown(&app), ["docs", "a.txt", "b.txt"]);

        press(&mut app, '/');
        typed(&mut app, "zzz");
        assert!(shown(&app).is_empty());
        code(&mut app, KeyCode::Esc); // cancel while typing
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(shown(&app).len(), 3);
    }

    #[test]
    fn q_quits_only_outside_text_input_and_dialogs() {
        let d = sample();
        let mut app = app_in(d.path());
        press(&mut app, '/');
        press(&mut app, 'q');
        assert!(!app.should_quit);
        code(&mut app, KeyCode::Esc);

        press(&mut app, 'd');
        assert!(matches!(app.mode, Mode::Confirm(_)));
        press(&mut app, 'q');
        assert!(!app.should_quit);
        assert!(matches!(app.mode, Mode::Confirm(_)));
        press(&mut app, 'n');

        press(&mut app, 'q');
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_always_quits() {
        let d = sample();
        let mut app = app_in(d.path());
        press(&mut app, '/');
        app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn delete_needs_explicit_yes() {
        let d = sample();
        let mut app = app_in(d.path());
        press(&mut app, 'j'); // a.txt
        assert_eq!(cursor_name(&app), "a.txt");

        press(&mut app, 'd');
        assert!(matches!(app.mode, Mode::Confirm(Confirm::Delete(_))));
        assert!(d.path().join("a.txt").exists(), "nothing deleted yet");

        code(&mut app, KeyCode::Enter); // Enter must NOT confirm
        assert!(d.path().join("a.txt").exists());
        assert!(matches!(app.mode, Mode::Normal));

        press(&mut app, 'd');
        press(&mut app, 'n');
        assert!(d.path().join("a.txt").exists());

        press(&mut app, 'd');
        code(&mut app, KeyCode::Esc);
        assert!(d.path().join("a.txt").exists());

        press(&mut app, 'd');
        press(&mut app, 'y');
        assert!(!d.path().join("a.txt").exists());
        assert!(d.path().join("b.txt").exists());
        assert_eq!(shown(&app), ["docs", "b.txt"]);
    }

    #[test]
    fn space_selects_and_delete_applies_to_all_marked() {
        let d = sample();
        let mut app = app_in(d.path());
        press(&mut app, 'j'); // a.txt
        press(&mut app, ' '); // mark a.txt, cursor moves to b.txt
        press(&mut app, ' '); // mark b.txt
        assert_eq!(app.marked_count(), 2);
        press(&mut app, 'd');
        match &app.mode {
            Mode::Confirm(Confirm::Delete(p)) => assert_eq!(p.len(), 2),
            _ => panic!("expected delete confirmation"),
        }
        press(&mut app, 'y');
        assert!(!d.path().join("a.txt").exists() && !d.path().join("b.txt").exists());
        assert!(d.path().join("docs").exists());
    }

    #[test]
    fn hidden_marked_entries_are_not_deleted_by_accident() {
        let d = sample();
        let mut app = app_in(d.path());
        press(&mut app, '.');
        press(&mut app, 'g');
        // mark everything including .hidden
        press(&mut app, 'a');
        press(&mut app, '.'); // hide dotfiles again
        press(&mut app, 'd');
        match &app.mode {
            Mode::Confirm(Confirm::Delete(p)) => {
                assert!(p.iter().all(|p| !p.ends_with(".hidden")));
            }
            _ => panic!("expected delete confirmation"),
        }
    }

    #[test]
    fn copy_paste_asks_before_overwriting() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("src")).unwrap();
        fs::create_dir_all(d.path().join("dest")).unwrap();
        fs::write(d.path().join("src/f.txt"), "new").unwrap();
        fs::write(d.path().join("dest/f.txt"), "old").unwrap();

        let mut app = app_in(&d.path().join("src"));
        press(&mut app, 'c');
        assert!(app.clipboard.is_some());
        app.change_dir(d.path().join("dest"), None);

        press(&mut app, 'p');
        assert!(matches!(app.mode, Mode::Confirm(Confirm::Overwrite { .. })));
        assert_eq!(
            fs::read_to_string(d.path().join("dest/f.txt")).unwrap(),
            "old"
        );

        press(&mut app, 'n');
        assert_eq!(
            fs::read_to_string(d.path().join("dest/f.txt")).unwrap(),
            "old"
        );

        press(&mut app, 'p');
        press(&mut app, 'y');
        assert_eq!(
            fs::read_to_string(d.path().join("dest/f.txt")).unwrap(),
            "new"
        );
        assert!(d.path().join("src/f.txt").exists(), "copy keeps the source");
    }

    #[test]
    fn move_paste_moves_and_clears_clipboard() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("src")).unwrap();
        fs::create_dir_all(d.path().join("dest")).unwrap();
        fs::write(d.path().join("src/f.txt"), "x").unwrap();

        let mut app = app_in(&d.path().join("src"));
        press(&mut app, 'm');
        app.change_dir(d.path().join("dest"), None);
        press(&mut app, 'p');
        assert!(d.path().join("dest/f.txt").exists());
        assert!(!d.path().join("src/f.txt").exists());
        assert!(app.clipboard.is_none());
        assert_eq!(cursor_name(&app), "f.txt");
    }

    #[test]
    fn escape_cancels_pending_copy() {
        let d = sample();
        let mut app = app_in(d.path());
        press(&mut app, 'c');
        assert!(app.clipboard.is_some());
        code(&mut app, KeyCode::Esc);
        assert!(app.clipboard.is_none());
    }

    #[test]
    fn paste_problems_are_shown_in_a_dialog() {
        let d = sample();
        let mut app = app_in(d.path());
        press(&mut app, 'm'); // move "docs" ...
        press(&mut app, 'p'); // ... into the folder it is already in
        assert!(matches!(app.mode, Mode::Message { .. }));
        code(&mut app, KeyCode::Char('x')); // any key closes
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn rename_flow_and_conflict_message() {
        let d = sample();
        let mut app = app_in(d.path());
        press(&mut app, 'j'); // a.txt
        press(&mut app, 'r');
        match &app.mode {
            Mode::Rename { input, .. } => assert_eq!(input.value(), "a.txt"),
            _ => panic!("expected rename mode"),
        }
        // Try to rename onto an existing file: refused, dialog stays open.
        for _ in 0..5 {
            code(&mut app, KeyCode::Backspace);
        }
        typed(&mut app, "b.txt");
        code(&mut app, KeyCode::Enter);
        match &app.mode {
            Mode::Rename { error, .. } => {
                assert!(error.as_ref().unwrap().contains("already exists"))
            }
            _ => panic!("rename dialog should stay open"),
        }
        assert!(d.path().join("a.txt").exists());

        for _ in 0..5 {
            code(&mut app, KeyCode::Backspace);
        }
        typed(&mut app, "c.txt");
        code(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert!(d.path().join("c.txt").exists() && !d.path().join("a.txt").exists());
        assert_eq!(cursor_name(&app), "c.txt");

        press(&mut app, 'r');
        code(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn invalid_utf8_names_do_not_break_anything() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join(OsStr::from_bytes(b"caf\xe9.txt")), "x").unwrap();
        let mut app = app_in(d.path());
        assert_eq!(app.visible.len(), 1);
        assert!(app.current_entry().unwrap().lossy);
        // Renaming without editing the text leaves the file alone.
        press(&mut app, 'r');
        code(&mut app, KeyCode::Enter);
        assert!(d.path().join(OsStr::from_bytes(b"caf\xe9.txt")).exists());
        // Delete flow works on it too.
 
