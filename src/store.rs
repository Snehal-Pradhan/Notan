use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_NOTE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_DEPTH: usize = 16;
const IGNORED_DIRS: &[&str] = &[
    ".git", ".trash", "target", "node_modules", "dist", "build", "vendor", ".next", ".cache",
];

static NEXT_UID: AtomicU64 = AtomicU64::new(1);

fn next_uid() -> u64 {
    NEXT_UID.fetch_add(1, Ordering::Relaxed)
}

/// Cheap identity of the on-disk file, used to detect external edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    pub len: u64,
    pub modified: Option<SystemTime>,
}

impl Fingerprint {
    pub fn of(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        Some(Self {
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveStatus {
    Saved,
    /// The file changed on disk since the last load/save; nothing was written.
    Conflict,
}

#[derive(Clone, Debug)]
pub struct Note {
    pub uid: u64,
    pub id: String,
    pub path: PathBuf,
    pub body: String,
    pub modified: SystemTime,
    pub disk: Option<Fingerprint>,
}

impl Note {
    pub fn title(&self) -> String {
        title_of(&self.body)
    }
}

pub struct Scan {
    pub notes: Vec<(PathBuf, Fingerprint)>,
    pub dirs: usize,
    pub files: usize,
}

#[derive(Debug)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn open_in(dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        Ok(Self { dir })
    }

    pub fn load(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        self.walk_notes(&self.dir, 0, &mut notes);
        notes.sort_by(|a, b| b.modified.cmp(&a.modified));
        notes
    }

    fn walk_notes(&self, dir: &Path, depth: usize, notes: &mut Vec<Note>) {
        if depth > MAX_DEPTH {
            return;
        }
        let Ok(read_dir) = fs::read_dir(dir) else {
            return;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with('.') || IGNORED_DIRS.contains(&name) {
                continue;
            }
            if path.is_dir() {
                self.walk_notes(&path, depth + 1, notes);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.len() > MAX_NOTE_BYTES {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let body = String::from_utf8_lossy(&bytes).into_owned();
            let id = self.rel_id(&path);
            if id.is_empty() {
                continue;
            }
            let disk = Fingerprint::of(&path);
            notes.push(Note {
                uid: next_uid(),
                id,
                path,
                body,
                modified: meta.modified().unwrap_or(UNIX_EPOCH),
                disk,
            });
        }
    }

    /// Metadata-only snapshot for cheap change detection.
    pub fn scan(&self) -> Scan {
        let mut scan = Scan {
            notes: Vec::new(),
            dirs: 0,
            files: 0,
        };
        self.scan_dir(&self.dir, &mut scan);
        scan
    }

    #[allow(clippy::only_used_in_recursion)]
    fn scan_dir(&self, dir: &Path, scan: &mut Scan) {
        let Ok(read_dir) = fs::read_dir(dir) else {
            return;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if ignored(name) {
                continue;
            }
            if path.is_dir() {
                scan.dirs += 1;
                self.scan_dir(&path, scan);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(fingerprint) = Fingerprint::of(&path) {
                    if fingerprint.len <= MAX_NOTE_BYTES {
                        scan.notes.push((path, fingerprint));
                    }
                }
            } else {
                scan.files += 1;
            }
        }
    }

    /// Every non-hidden directory in the workspace.
    pub fn dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        self.collect_dirs(&self.dir, &mut dirs);
        dirs
    }

    #[allow(clippy::only_used_in_recursion)]
    fn collect_dirs(&self, dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(read_dir) = fs::read_dir(dir) else {
            return;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !path.is_dir() || ignored(name) {
                continue;
            }
            out.push(path.clone());
            self.collect_dirs(&path, out);
        }
    }

    /// Files that are not Markdown. They appear in the tree but cannot open.
    pub fn files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        self.collect_files(&self.dir, &mut files);
        files
    }

    #[allow(clippy::only_used_in_recursion)]
    fn collect_files(&self, dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(read_dir) = fs::read_dir(dir) else {
            return;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if ignored(name) {
                continue;
            }
            if path.is_dir() {
                self.collect_files(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) != Some("md") {
                out.push(path);
            }
        }
    }

    /// Path relative to the workspace root, without the `.md` suffix.
    pub fn rel_id(&self, path: &Path) -> String {
        let rel = path.strip_prefix(&self.dir).unwrap_or(path);
        let text = rel.to_string_lossy().replace('\\', "/");
        text.strip_suffix(".md").unwrap_or(&text).to_owned()
    }

    #[cfg(test)]
    pub fn create(&self) -> io::Result<Note> {
        self.create_in(&self.dir)
    }

    /// Create an empty note inside `dir` (must live under the workspace).
    pub fn create_in(&self, dir: &Path) -> io::Result<Note> {
        let dir = if dir.starts_with(&self.dir) {
            dir
        } else {
            &self.dir
        };
        fs::create_dir_all(dir)?;
        let mut path = dir.join("untitled.md");
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("untitled-{n}.md"));
            n += 1;
        }
        fs::write(&path, "")?;
        let id = self.rel_id(&path);
        Ok(Note {
            uid: next_uid(),
            id,
            disk: Fingerprint::of(&path),
            path,
            body: String::new(),
            modified: SystemTime::now(),
        })
    }

    /// Create an empty note with a user-chosen name inside `dir`.
    pub fn create_named_in(&self, dir: &Path, name: &str) -> io::Result<Note> {
        let dir = if dir.starts_with(&self.dir) {
            dir
        } else {
            &self.dir
        };
        fs::create_dir_all(dir)?;
        let slug = slugify(name);
        let slug = if slug.is_empty() { "untitled".to_owned() } else { slug };
        let mut path = dir.join(format!("{slug}.md"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{slug}-{n}.md"));
            n += 1;
        }
        fs::write(&path, "")?;
        Ok(Note {
            uid: next_uid(),
            id: self.rel_id(&path),
            disk: Fingerprint::of(&path),
            path,
            body: String::new(),
            modified: SystemTime::now(),
        })
    }

    /// Rename a file or folder in place. Files keep their extension.
    pub fn rename_path(&self, path: &Path, name: &str) -> io::Result<PathBuf> {
        if !path.starts_with(&self.dir) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "outside workspace",
            ));
        }
        let slug = slugify(name);
        if slug.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty name"));
        }
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent"))?;
        let (target, mut n) = if path.is_dir() {
            (parent.join(&slug), 2)
        } else {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_default();
            (parent.join(format!("{slug}{ext}")), 2)
        };
        if target == path {
            return Ok(path.to_path_buf());
        }
        let mut target = target;
        while target.exists() {
            if path.is_dir() {
                target = parent.join(format!("{slug}-{n}"));
            } else {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| format!(".{e}"))
                    .unwrap_or_default();
                target = parent.join(format!("{slug}-{n}{ext}"));
            }
            n += 1;
        }
        fs::rename(path, &target)?;
        Ok(target)
    }

    /// Create a folder under `parent`. Returns the new path.
    pub fn create_dir(&self, parent: &Path, name: &str) -> io::Result<PathBuf> {
        let parent = if parent.starts_with(&self.dir) {
            parent
        } else {
            &self.dir
        };
        let slug = slugify(name);
        let slug = if slug.is_empty() { "folder".to_owned() } else { slug };
        let mut path = parent.join(&slug);
        let mut n = 2;
        while path.exists() {
            path = parent.join(format!("{slug}-{n}"));
            n += 1;
        }
        fs::create_dir_all(&path)?;
        Ok(path)
    }

    /// Persist a note. Renames the backing file when the title yields a free,
    /// meaningful slug. Updates `note.id`/`note.path` when a rename happens.
    ///
    /// With `force == false` the write is skipped when the file changed on disk
    /// since the note was loaded or last saved. `force` overwrites that version.
    pub fn save(&self, note: &mut Note, force: bool) -> io::Result<SaveStatus> {
        if !force {
            if let Some(expected) = note.disk {
                if Fingerprint::of(&note.path) != Some(expected) {
                    return Ok(SaveStatus::Conflict);
                }
            }
        }

        let slug = slugify(&note.title());
        let parent = note.path.parent().unwrap_or(&self.dir);
        let target = parent.join(format!("{slug}.md"));
        let stem = note
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let placeholder = stem == "untitled" || stem.starts_with("untitled-");

        if placeholder
            && target != note.path
            && !target.exists()
            && fs::rename(&note.path, &target).is_ok()
        {
            note.path = target;
            note.id = self.rel_id(&note.path);
        }

        atomic_write(&note.path, note.body.as_bytes())?;
        note.modified = SystemTime::now();
        note.disk = Fingerprint::of(&note.path);
        Ok(SaveStatus::Saved)
    }

    /// Move a note into `<workspace>/.trash/`.
    pub fn trash(&self, note: &Note) -> io::Result<PathBuf> {
        self.trash_path(&note.path)
    }

    /// Move any file or folder into `<workspace>/.trash/`.
    pub fn trash_path(&self, path: &Path) -> io::Result<PathBuf> {
        let trash = self.dir.join(".trash");
        fs::create_dir_all(&trash)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("item");
        let (stem, ext) = match path.extension().and_then(|e| e.to_str()) {
            Some(ext) => (name.trim_end_matches(&format!(".{ext}")), format!(".{ext}")),
            None => (name, String::new()),
        };
        let mut target = trash.join(format!("{stem}-{stamp}{ext}"));
        let mut n = 2;
        while target.exists() {
            target = trash.join(format!("{stem}-{stamp}-{n}{ext}"));
            n += 1;
        }
        fs::rename(path, &target)?;
        Ok(target)
    }

    /// Move a file or folder into `to_dir`, keeping its name. Adds a suffix on
    /// collision. Refuses to move a folder into itself or its descendants.
    pub fn move_path(&self, from: &Path, to_dir: &Path) -> io::Result<PathBuf> {
        if !from.starts_with(&self.dir) || !to_dir.starts_with(&self.dir) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "outside workspace",
            ));
        }
        if !from.exists() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "missing source"));
        }
        if from.is_dir() && to_dir.starts_with(from) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot move a folder into itself",
            ));
        }
        let name = from
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad name"))?;
        let mut target = to_dir.join(name);
        let (stem, ext) = split_name(name);
        let mut n = 2;
        while target.exists() {
            target = to_dir.join(format!("{stem}-{n}{ext}"));
            n += 1;
        }
        fs::rename(from, &target)?;
        Ok(target)
    }

    /// Move a trashed file or folder back into the workspace root.
    pub fn restore(&self, trashed: &Path) -> io::Result<Option<PathBuf>> {
        if !trashed.exists() {
            return Ok(None);
        }
        let Some(name) = trashed.file_name().and_then(|n| n.to_str()) else {
            return Ok(None);
        };
        let (stem, ext) = split_name(name);
        let stem = strip_stamp(&stem);
        let mut target = self.dir.join(format!("{stem}{ext}"));
        let mut n = 2;
        while target.exists() {
            target = self.dir.join(format!("{stem}-{n}{ext}"));
            n += 1;
        }
        fs::rename(trashed, &target)?;
        Ok(Some(target))
    }

    /// Trash entries, newest first.
    pub fn trash_entries(&self) -> Vec<PathBuf> {
        let Ok(read_dir) = fs::read_dir(self.dir.join(".trash")) else {
            return Vec::new();
        };
        let mut entries: Vec<PathBuf> = read_dir
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| !path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
            .collect();
        entries.sort_by_key(|path| {
            std::cmp::Reverse(
                fs::metadata(path)
                    .and_then(|meta| meta.modified())
                    .unwrap_or(UNIX_EPOCH),
            )
        });
        entries
    }

    /// Permanently delete everything in the trash. Returns the number of files.
    pub fn empty_trash(&self) -> io::Result<usize> {
        let trash = self.dir.join(".trash");
        let Ok(read_dir) = fs::read_dir(&trash) else {
            return Ok(0);
        };
        let mut removed = 0;
        for entry in read_dir.flatten() {
            let path = entry.path();
            let ok = if path.is_dir() {
                fs::remove_dir_all(&path).is_ok()
            } else {
                fs::remove_file(&path).is_ok()
            };
            if ok {
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn trash_count(&self) -> usize {
        let Ok(read_dir) = fs::read_dir(self.dir.join(".trash")) else {
            return 0;
        };
        read_dir.flatten().count()
    }
}
fn strip_stamp(stem: &str) -> &str {
    match stem.rsplit_once('-') {
        Some((base, suffix))
            if !base.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) =>
        {
            base
        }
        _ => stem,
    }
}

fn ignored(name: &str) -> bool {
    name.starts_with('.') || IGNORED_DIRS.contains(&name)
}

fn split_name(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_owned(), format!(".{ext}")),
        _ => (name.to_owned(), String::new()),
    }
}

/// Write to a sibling temp file, flush it to disk, then rename over the target.
/// The parent directory is flushed too so the rename survives a crash.
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("note.md");
    let tmp = parent.join(format!(".{name}.{}.tmp", std::process::id()));

    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();

    if let Err(err) = result {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }

    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Last workflow folder, remembered under Application Support.
pub fn remember_workspace(path: &Path) {
    let Some(config) = config_path() else {
        return;
    };
    if let Some(dir) = config.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(config, path.to_string_lossy().as_bytes());
}

pub fn last_workspace() -> Option<PathBuf> {
    let text = fs::read_to_string(config_path()?).ok()?;
    let path = PathBuf::from(text.trim());
    path.is_dir().then_some(path)
}

fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(
        home.join("Library")
            .join("Application Support")
            .join("notan")
            .join("workspace"),
    )
}

pub fn default_workspace() -> PathBuf {
    notes_dir()
}

pub struct Session {
    pub workspace: PathBuf,
    pub tabs: Vec<String>,
    pub active: Option<String>,
    pub expanded: Vec<PathBuf>,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

pub fn load_session() -> Option<Session> {
    let text = fs::read_to_string(session_path()?).ok()?;
    let mut workspace = None;
    let mut tabs = Vec::new();
    let mut active = None;
    let mut expanded = Vec::new();
    let mut width = None;
    let mut height = None;

    for line in text.lines() {
        let mut parts = line.split('\t');
        match parts.next()? {
            "workspace" => workspace = parts.next().map(PathBuf::from),
            "tab" => {
                if let Some(id) = parts.next() {
                    tabs.push(id.to_owned());
                }
            }
            "active" => active = parts.next().map(str::to_owned),
            "open" => {
                if let Some(path) = parts.next() {
                    expanded.push(PathBuf::from(path));
                }
            }
            "size" => {
                width = parts.next().and_then(|value| value.parse().ok());
                height = parts.next().and_then(|value| value.parse().ok());
            }
            _ => {}
        }
    }

    Some(Session {
        workspace: workspace?,
        tabs,
        active,
        expanded,
        width,
        height,
    })
}

pub fn save_session(session: &Session) {
    let Some(path) = session_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut out = String::new();
    out.push_str(&format!("workspace\t{}\n", session.workspace.display()));
    for tab in &session.tabs {
        out.push_str(&format!("tab\t{tab}\n"));
    }
    if let Some(active) = &session.active {
        out.push_str(&format!("active\t{active}\n"));
    }
    for path in &session.expanded {
        out.push_str(&format!("open\t{}\n", path.display()));
    }
    if let (Some(width), Some(height)) = (session.width, session.height) {
        out.push_str(&format!("size\t{width}\t{height}\n"));
    }
    let _ = fs::write(path, out);
}

fn session_path() -> Option<PathBuf> {
    config_path().map(|path| path.with_file_name("session"))
}

fn notes_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let documents = home.join("Documents");
    if documents.is_dir() {
        documents.join("Notan")
    } else {
        home.join(".notan")
    }
}

/// First heading or first non-empty line, stripped of common Markdown markers.
pub fn title_of(body: &str) -> String {
    let mut in_meta = true;
    for line in body.lines().take(60) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "---" {
            continue;
        }
        if in_meta {
            let lower = trimmed.to_lowercase();
            if lower.starts_with("type:") || lower.starts_with("color:") {
                continue;
            }
            in_meta = false;
        }
        if let Some(heading) = trimmed.strip_prefix('#') {
            let heading = clean_title(heading.trim_start_matches('#'));
            if !heading.is_empty() {
                return clip(&heading, 120);
            }
            continue;
        }
        let cleaned = clean_title(
            trimmed
                .trim_start_matches(['>', '-', '*', '+'])
                .trim(),
        );
        if cleaned.is_empty() {
            continue;
        }
        return clip(&cleaned, 120);
    }
    "untitled".to_owned()
}

fn clean_title(text: &str) -> String {
    text.trim()
        .replace(['*', '`', '~'], "")
        .trim()
        .to_owned()
}

fn clip(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;

    for c in title.chars() {
        let mapped = if c.is_ascii_alphanumeric() {
            Some(c.to_ascii_lowercase())
        } else {
            match c {
                'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => {
                    Some('a')
                }
                'ç' | 'Ç' => Some('c'),
                'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => Some('e'),
                'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => Some('i'),
                'ñ' | 'Ñ' => Some('n'),
                'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => {
                    Some('o')
                }
                'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => Some('u'),
                'ý' | 'ÿ' | 'Ý' | 'Ÿ' => Some('y'),
                'ß' => Some('s'),
                _ => None,
            }
        };

        if let Some(ch) = mapped {
            out.push(ch);
            last_dash = false;
        } else if c.is_whitespace() || matches!(c, '-' | '_' | '.' | '/' | '\\' | ':') {
            if !out.is_empty() && !last_dash {
                out.push('-');
                last_dash = true;
            }
        } else if !c.is_ascii() {
            // Keep filenames ASCII so macOS normalization never changes identity.
            if !out.is_empty() && !last_dash {
                out.push('-');
                last_dash = true;
            }
        }

        if out.len() >= 48 {
            break;
        }
    }

    let trimmed = out.trim_matches('-').to_owned();
    if trimmed.is_empty() {
        "untitled".to_owned()
    } else {
        trimmed
    }
}

pub fn relative_time(then: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(then)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match secs {
        0..=59 => "now".to_owned(),
        60..=3_599 => format!("{}m", secs / 60),
        3_600..=86_399 => format!("{}h", secs / 3_600),
        86_400..=604_799 => format!("{}d", secs / 86_400),
        604_800..=2_591_999 => format!("{}w", secs / 604_800),
        _ => format!("{}mo", secs / 2_592_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(title_of("# Hello world\n\nbody"), "Hello world");
        assert_eq!(title_of("\n\nplain first line\nmore"), "plain first line");
        assert_eq!(title_of("- a list item"), "a list item");
        assert_eq!(title_of("# **Bold** idea"), "Bold idea");
        assert_eq!(title_of(""), "untitled");
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  spaces   and   more  "), "spaces-and-more");
        assert_eq!(slugify("***"), "untitled");
        assert_eq!(slugify("Café notes"), "cafe-notes");
        assert_eq!(slugify("日本語"), "untitled");
    }

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "notan-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let store = Store { dir: dir.clone() };

        let mut note = store.create().unwrap();
        note.body = "# First note\n\nhello".to_owned();
        assert_eq!(store.save(&mut note, false).unwrap(), SaveStatus::Saved);
        assert_eq!(note.id, "first-note");
        assert!(note.path.exists());

        let loaded = store.load();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title(), "First note");
        assert_eq!(loaded[0].body, "# First note\n\nhello");
        assert!(loaded[0].disk.is_some());

        let trashed = store.trash(&note).unwrap();
        assert!(store.load().is_empty());
        assert!(dir.join(".trash").is_dir());
        assert_eq!(store.trash_count(), 1);

        let restored = store.restore(&trashed).unwrap().expect("restored");
        assert_eq!(restored.file_name().unwrap(), "first-note.md");
        assert_eq!(store.load().len(), 1);
        assert_eq!(store.trash_count(), 0);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_external_edits() {
        let dir = temp_dir("conflict");
        let store = Store { dir: dir.clone() };

        let mut note = store.create().unwrap();
        note.body = "# Local\n".to_owned();
        store.save(&mut note, false).unwrap();

        fs::write(&note.path, "# Written by another app\n").unwrap();

        note.body = "# Local edit\n".to_owned();
        assert_eq!(store.save(&mut note, false).unwrap(), SaveStatus::Conflict);
        assert_eq!(
            fs::read_to_string(&note.path).unwrap(),
            "# Written by another app\n"
        );

        assert_eq!(store.save(&mut note, true).unwrap(), SaveStatus::Saved);
        assert_eq!(fs::read_to_string(&note.path).unwrap(), "# Local edit\n");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn loads_nested_workspace() {
        let dir = temp_dir("nested");
        let store = Store { dir: dir.clone() };

        fs::create_dir_all(dir.join("specs")).unwrap();
        fs::create_dir_all(dir.join("drafts")).unwrap();
        fs::create_dir_all(dir.join("assets")).unwrap();
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        fs::write(dir.join("specs").join("auth.md"), "# Auth spec\n").unwrap();
        fs::write(dir.join("assets").join("logo.png"), b"png").unwrap();
        fs::write(dir.join("node_modules").join("x.md"), "# ignored\n").unwrap();
        fs::write(dir.join("root.md"), "# Root\n").unwrap();

        let notes = store.load();
        assert_eq!(notes.len(), 2);
        let auth = notes.iter().find(|n| n.id == "specs/auth").expect("auth");
        assert_eq!(auth.title(), "Auth spec");

        let dirs = store.dirs();
        assert!(dirs.contains(&dir.join("specs")));
        assert!(dirs.contains(&dir.join("drafts")));
        assert!(dirs.contains(&dir.join("assets")));
        assert!(!dirs.contains(&dir.join("node_modules")));

        let files = store.files();
        assert!(files.contains(&dir.join("assets").join("logo.png")));
        assert!(!files.iter().any(|path| path.ends_with("x.md")));

        let mut created = store.create_in(&dir.join("specs")).unwrap();
        assert_eq!(created.id, "specs/untitled");
        created.body = "# Deep spec\n".to_owned();
        assert_eq!(store.save(&mut created, false).unwrap(), SaveStatus::Saved);
        assert_eq!(created.id, "specs/deep-spec");
        assert_eq!(created.path.parent(), Some(dir.join("specs").as_path()));

        let folder = store.create_dir(&dir, "Plans Q1").unwrap();
        assert!(folder.is_dir());
        assert_eq!(folder.file_name().unwrap(), "plans-q1");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn moves_files_and_folders() {
        let dir = temp_dir("move");
        let store = Store { dir: dir.clone() };
        fs::create_dir_all(dir.join("specs")).unwrap();
        fs::create_dir_all(dir.join("docs")).unwrap();
        fs::write(dir.join("a.md"), "# A\n").unwrap();

        let moved = store.move_path(&dir.join("a.md"), &dir.join("specs")).unwrap();
        assert_eq!(moved, dir.join("specs").join("a.md"));
        assert!(moved.is_file());

        fs::write(dir.join("specs").join("a.md"), "# A2\n").unwrap();
        let second = store.move_path(&dir.join("a.md"), &dir.join("specs"));
        assert!(second.is_err() || second.unwrap() != dir.join("specs").join("a.md"));

        fs::write(dir.join("b.md"), "# B\n").unwrap();
        let duplicate = store.move_path(&dir.join("b.md"), &dir.join("docs")).unwrap();
        assert_eq!(duplicate, dir.join("docs").join("b.md"));

        let inside = store.move_path(&dir.join("docs"), &dir.join("docs"));
        assert!(inside.is_err());

        let folder = store.move_path(&dir.join("docs"), &dir.join("specs")).unwrap();
        assert_eq!(folder, dir.join("specs").join("docs"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn restores_trashed_folders() {
        let dir = temp_dir("restore-folder");
        let store = Store { dir: dir.clone() };
        fs::create_dir_all(dir.join("specs")).unwrap();
        fs::write(dir.join("specs").join("a.md"), "# A\n").unwrap();

        let trashed = store.trash_path(&dir.join("specs")).unwrap();
        assert!(!dir.join("specs").exists());
        assert!(store.trash_entries().contains(&trashed));

        let restored = store.restore(&trashed).unwrap().expect("restored");
        assert_eq!(restored, dir.join("specs"));
        assert!(restored.join("a.md").is_file());
        assert!(store.trash_entries().is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_trash_removes_files() {
        let dir = temp_dir("trash");
        let store = Store { dir: dir.clone() };

        let mut first = store.create().unwrap();
        first.body = "# One\n".to_owned();
        store.save(&mut first, false).unwrap();
        let mut second = store.create().unwrap();
        second.body = "# Two\n".to_owned();
        store.save(&mut second, false).unwrap();

        store.trash(&first).unwrap();
        store.trash(&second).unwrap();
        assert_eq!(store.trash_count(), 2);

        assert_eq!(store.empty_trash().unwrap(), 2);
        assert_eq!(store.trash_count(), 0);
        assert!(store.load().is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "notan-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
