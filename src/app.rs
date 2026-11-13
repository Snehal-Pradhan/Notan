use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FocusDirection, FontFamily, FontId,
    Frame, Id, Key, KeyboardShortcut, Layout, Margin, Modifiers, Order, Rect, RichText, ScrollArea,
    Sense, Stroke, TextEdit, Ui, ViewportCommand,
};

use crate::actions::{Action, ACTIONS};
use crate::markdown;
use crate::store::{self, Note, SaveStatus, Store};
use crate::theme;

const EDITOR_ID: &str = "notan-editor";
const SEARCH_ID: &str = "notan-search";
const PALETTE_ID: &str = "notan-palette";
const INLINE_ID: &str = "notan-inline";
const LOGO_RGBA: &[u8] = include_bytes!("../assets/logo-64.rgba");
const AUTOSAVE_MS: u64 = 450;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    List,
    Editor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Overlay {
    None,
    Palette,
    Help,
    Confirm,
    Trash,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InlineKind {
    NewFile,
    NewFolder,
    RenameFile,
    RenameFolder,
}

#[derive(Clone)]
struct InlineEdit {
    kind: InlineKind,
    /// Parent folder for new entries, or the existing path to rename.
    path: PathBuf,
    text: String,
}

#[derive(Clone, PartialEq, Eq)]
enum Pending {
    DeleteNote,
    DeletePath(PathBuf, bool),
    EmptyTrash,
    ClearNotes,
}

#[derive(Default)]
struct Search {
    active: bool,
    query: String,
}

struct NoteMeta {
    title: String,
    snippet: String,
}

/// Parsed preview for one note at one revision.
struct PreviewCache {
    uid: u64,
    revision: u64,
    blocks: Vec<markdown::Block>,
    words: usize,
    title: String,
    kind: Option<String>,
    kind_color: Option<Color32>,
}

#[derive(Clone)]
struct TreeRow {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
    name: String,
    supported: bool,
    note_id: Option<String>,
}

pub struct Notan {
    store: Option<Store>,
    store_error: Option<String>,
    notes: Vec<Note>,
    selected: Option<String>,
    focus: Focus,
    preview: bool,
    preview_only: bool,
    sidebar: bool,
    zen: bool,
    search: Search,
    dirty: bool,
    conflict: bool,
    conflict_notified: bool,
    save_failures: u32,
    retry_at: Option<Instant>,
    last_trashed: Option<PathBuf>,
    last_edit: Instant,
    last_saved: Option<Instant>,
    overlay: Overlay,
    palette_query: String,
    palette_index: usize,
    palette_scroll: bool,
    pending: Option<Pending>,
    toast: Option<(String, Instant)>,
    copied_at: Option<Instant>,
    trash_entries: Vec<PathBuf>,
    trash_index: usize,
    /// Bumped on every edit or note-list change; invalidates derived data.
    revision: u64,
    last_poll: Instant,
    preview_cache: Option<PreviewCache>,
    note_meta: HashMap<u64, NoteMeta>,
    workspace_dirs: Vec<PathBuf>,
    workspace_files: Vec<PathBuf>,
    tree_rows: Vec<TreeRow>,
    tree_cursor: usize,
    expanded: HashSet<PathBuf>,
    tree_dirty: bool,
    pending_cursor: Option<PathBuf>,
    inline: Option<InlineEdit>,
    drag_path: Option<PathBuf>,
    drop_target: Option<PathBuf>,
    open_tabs: Vec<String>,
    logo: Option<egui::TextureHandle>,
    pending_window: Option<egui::Vec2>,
    last_session_save: Instant,
}

impl Notan {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let dir = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .or_else(store::last_workspace)
            .unwrap_or_else(store::default_workspace);
        let store = Store::open_in(dir);
        if let Ok(store) = &store {
            store::remember_workspace(&store.dir);
        }
        let session = store::load_session().filter(|session| match &store {
            Ok(store) => session.workspace == store.dir,
            Err(_) => false,
        });
        Self::build(&cc.egui_ctx, store, session)
    }

    #[cfg(test)]
    fn from_context(ctx: &egui::Context, store: Result<Store, String>) -> Self {
        Self::build(ctx, store, None)
    }

    fn build(
        ctx: &egui::Context,
        store: Result<Store, String>,
        session: Option<store::Session>,
    ) -> Self {
        theme::install(ctx);
        ctx.send_viewport_cmd(ViewportCommand::SetTheme(egui::SystemTheme::Dark));

        let (store, store_error, notes, workspace_dirs, workspace_files, expanded) = match store {
            Ok(store) => {
                let notes = store.load();
                let dirs = store.dirs();
                let files = store.files();
                let root = store.dir.clone();
                (Some(store), None, notes, dirs, files, HashSet::from([root]))
            }
            Err(err) => (
                None,
                Some(err),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                HashSet::new(),
            ),
        };
        let selected = notes.first().map(|note| note.id.clone());

        let mut app = Self {
            store,
            store_error,
            notes,
            selected: selected.clone(),
            focus: Focus::List,
            preview: true,
            preview_only: false,
            sidebar: true,
            zen: false,
            search: Search::default(),
            dirty: false,
            conflict: false,
            conflict_notified: false,
            save_failures: 0,
            retry_at: None,
            last_trashed: None,
            last_edit: Instant::now(),
            last_saved: None,
            overlay: Overlay::None,
            palette_query: String::new(),
            palette_index: 0,
            palette_scroll: false,
            pending: None,
            toast: None,
            copied_at: None,
            trash_entries: Vec::new(),
            trash_index: 0,
            revision: 0,
            last_poll: Instant::now(),
            preview_cache: None,
            note_meta: HashMap::new(),
            workspace_dirs,
            workspace_files,
            tree_rows: Vec::new(),
            tree_cursor: 0,
            expanded,
            tree_dirty: true,
            pending_cursor: None,
            inline: None,
            drag_path: None,
            drop_target: None,
            open_tabs: selected.iter().cloned().collect(),
            logo: None,
            pending_window: None,
            last_session_save: Instant::now(),
        };

        if let Some(session) = session {
            let tabs: Vec<String> = session
                .tabs
                .into_iter()
                .filter(|id| app.notes.iter().any(|note| &note.id == id))
                .collect();
            if !tabs.is_empty() {
                app.open_tabs = tabs;
            }
            app.selected = session
                .active
                .filter(|id| app.open_tabs.iter().any(|tab| tab == id))
                .or_else(|| app.open_tabs.first().cloned());
            for path in session.expanded {
                if path.is_dir() {
                    app.expanded.insert(path);
                }
            }
            if let (Some(width), Some(height)) = (session.width, session.height) {
                app.pending_window = Some(egui::vec2(width, height));
            }
        }
        app
    }

    fn save_session(&mut self, ctx: &egui::Context) {
        let Some(workspace) = self.workspace_root() else {
            return;
        };
        let expanded = self
            .expanded
            .iter()
            .filter(|path| **path != workspace)
            .cloned()
            .collect();
        let size = ctx.content_rect().size();
        store::save_session(&store::Session {
            workspace,
            tabs: self.open_tabs.clone(),
            active: self.selected.clone(),
            expanded,
            width: Some(size.x),
            height: Some(size.y),
        });
        self.last_session_save = Instant::now();
    }

    // ---------- state helpers ----------

    fn logo_texture(&mut self, ctx: &egui::Context) -> egui::TextureId {
        if self.logo.is_none() {
            self.logo = Some(ctx.load_texture(
                "notan-logo",
                egui::ColorImage::from_rgba_unmultiplied([64, 64], LOGO_RGBA),
                egui::TextureOptions::LINEAR,
            ));
        }
        self.logo.as_ref().expect("logo").id()
    }

    fn selected_index(&self) -> Option<usize> {
        let id = self.selected.as_ref()?;
        self.notes.iter().position(|note| &note.id == id)
    }

    /// Mark the selected note as edited and invalidate derived data.
    fn poll_external(&mut self) {
        if self.inline.is_some()
            || self.dirty
            || self.overlay != Overlay::None
            || self.last_poll.elapsed() < Duration::from_millis(1800)
        {
            return;
        }
        self.last_poll = Instant::now();
        let Some(store) = &self.store else {
            return;
        };
        let scan = store.scan();
        let same_count = scan.notes.len() == self.notes.len()
            && scan.dirs == self.workspace_dirs.len()
            && scan.files == self.workspace_files.len();
        let same_notes = same_count
            && scan.notes.iter().all(|(path, fingerprint)| {
                self.notes
                    .iter()
                    .any(|note| &note.path == path && note.disk == Some(*fingerprint))
            });
        if same_notes {
            return;
        }

        let notes = store.load();
        let dirs = store.dirs();
        let files = store.files();
        self.notes = notes;
        self.workspace_dirs = dirs;
        self.workspace_files = files;
        self.rebuild_meta();
        self.revision = self.revision.wrapping_add(1);
        self.preview_cache = None;
        self.tree_dirty = true;
        let keep = self
            .selected
            .clone()
            .filter(|id| self.notes.iter().any(|note| &note.id == id));
        self.selected = keep;
        self.prune_tabs();
        self.set_toast("refreshed from disk");
    }

    fn touch(&mut self) {
        self.dirty = true;
        self.last_edit = Instant::now();
        self.revision = self.revision.wrapping_add(1);
        self.refresh_selected_meta();
    }

    fn workspace_root(&self) -> Option<PathBuf> {
        self.store.as_ref().map(|store| store.dir.clone())
    }

    fn refresh_dirs(&mut self) {
        if let Some(store) = &self.store {
            self.workspace_dirs = store.dirs();
            self.workspace_files = store.files();
        }
        self.tree_dirty = true;
    }

    /// Rebuild the flattened tree rows when notes, folders, or expansion change.
    fn ensure_tree(&mut self) {
        if !self.tree_dirty {
            return;
        }
        self.tree_rows.clear();
        if let Some(root) = self.workspace_root() {
            self.collect_rows(&root, 0);
        }
        self.tree_dirty = false;

        if let Some(path) = self.pending_cursor.take() {
            if let Some(index) = self.tree_rows.iter().position(|row| row.path == path) {
                self.tree_cursor = index;
            }
        }
        if self.tree_cursor >= self.tree_rows.len() {
            self.tree_cursor = self.tree_rows.len().saturating_sub(1);
        }
    }

    fn collect_rows(&mut self, dir: &std::path::Path, depth: usize) {
        let mut entries: Vec<(bool, PathBuf, String, Option<String>)> = Vec::new();

        for path in &self.workspace_dirs {
            if path.parent() == Some(dir) {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    entries.push((true, path.clone(), name.to_owned(), None));
                }
            }
        }
        for note in &self.notes {
            if note.path.parent() == Some(dir) {
                let name = note
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&note.id)
                    .to_owned();
                entries.push((false, note.path.clone(), name, Some(note.id.clone())));
            }
        }
        for path in &self.workspace_files {
            if path.parent() == Some(dir) {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    entries.push((false, path.clone(), name.to_owned(), None));
                }
            }
        }

        entries.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.2.to_lowercase().cmp(&b.2.to_lowercase()))
        });

        for (is_dir, path, name, note_id) in entries {
            let expanded = self.expanded.contains(&path);
            self.tree_rows.push(TreeRow {
                path: path.clone(),
                depth,
                is_dir,
                name,
                supported: is_dir || note_id.is_some(),
                note_id,
            });
            if is_dir && expanded {
                self.collect_rows(&path, depth + 1);
            }
        }
    }

    fn target_dir(&self) -> Option<PathBuf> {
        self.current_dir().or_else(|| self.workspace_root())
    }

    fn current_dir(&self) -> Option<PathBuf> {
        let root = self.workspace_root()?;
        if self.search.active {
            return Some(root);
        }
        match self.tree_rows.get(self.tree_cursor) {
            Some(row) if row.is_dir => Some(row.path.clone()),
            Some(row) => row
                .path
                .parent()
                .map(std::path::Path::to_path_buf)
                .or(Some(root)),
            None => Some(root),
        }
    }

    fn move_cursor(&mut self, delta: isize, ctx: &egui::Context) {
        self.ensure_tree();
        if self.tree_rows.is_empty() {
            return;
        }
        let last = self.tree_rows.len() - 1;
        self.tree_cursor = (self.tree_cursor as isize + delta).clamp(0, last as isize) as usize;
        let row = self.tree_rows[self.tree_cursor].clone();
        if !row.is_dir {
            if let Some(id) = row.note_id {
                self.select_note(id, ctx);
            }
        }
    }

    fn activate_cursor(&mut self, ctx: &egui::Context) {
        self.ensure_tree();
        let Some(row) = self.tree_rows.get(self.tree_cursor).cloned() else {
            return;
        };
        if row.is_dir {
            if !self.expanded.remove(&row.path) {
                self.expanded.insert(row.path);
            }
            self.tree_dirty = true;
        } else if let Some(id) = row.note_id {
            self.select_note(id, ctx);
            self.focus_editor(ctx);
        }
    }

    fn expand_or_open(&mut self, ctx: &egui::Context) {
        self.ensure_tree();
        let Some(row) = self.tree_rows.get(self.tree_cursor).cloned() else {
            return;
        };
        if row.is_dir {
            if self.expanded.insert(row.path) {
                self.tree_dirty = true;
            }
        } else if let Some(id) = row.note_id {
            self.select_note(id, ctx);
            self.focus_editor(ctx);
        }
    }

    fn collapse_or_parent(&mut self) {
        self.ensure_tree();
        let Some(row) = self.tree_rows.get(self.tree_cursor).cloned() else {
            return;
        };
        if row.is_dir && self.expanded.remove(&row.path) {
            self.tree_dirty = true;
            return;
        }
        if row.depth > 0 {
            for index in (0..self.tree_cursor).rev() {
                if self.tree_rows[index].depth < row.depth {
                    self.tree_cursor = index;
                    break;
                }
            }
        }
    }

    fn prune_tabs(&mut self) {
        self.open_tabs
            .retain(|id| self.notes.iter().any(|note| &note.id == id));
        let active_missing = self
            .selected
            .as_ref()
            .is_some_and(|id| !self.notes.iter().any(|note| &note.id == id));
        if active_missing {
            self.selected = self.open_tabs.last().cloned();
            self.dirty = false;
            self.conflict = false;
            self.conflict_notified = false;
        }
    }

    fn close_tab(&mut self, ctx: &egui::Context) {
        let Some(active) = self.selected.clone() else {
            return;
        };
        if !self.save_current() {
            return;
        }
        let Some(index) = self.open_tabs.iter().position(|id| id == &active) else {
            return;
        };
        self.open_tabs.remove(index);
        self.selected = self
            .open_tabs
            .get(index)
            .or_else(|| self.open_tabs.last())
            .cloned();
        self.dirty = false;
        self.conflict = false;
        self.conflict_notified = false;
        if self.focus == Focus::Editor {
            let id = self.editor_id();
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }

    fn close_tab_id(&mut self, id: String, ctx: &egui::Context) {
        if self.selected.as_deref() == Some(id.as_str()) {
            self.close_tab(ctx);
        } else {
            self.open_tabs.retain(|tab| tab != &id);
        }
    }

    fn switch_tab(&mut self, index: usize, ctx: &egui::Context) {
        if let Some(id) = self.open_tabs.get(index).cloned() {
            self.select_note(id, ctx);
            self.focus = Focus::Editor;
        }
    }

    fn move_entry(&mut self, from: PathBuf, to_dir: PathBuf) {
        if from.parent() == Some(to_dir.as_path()) {
            return;
        }
        let Some(store) = &self.store else {
            return;
        };
        let selected_path = self.selected_index().map(|index| self.notes[index].path.clone());
        match store.move_path(&from, &to_dir) {
            Ok(target) => {
                let notes = store.load();
                let dirs = store.dirs();
                let files = store.files();
                self.notes = notes;
                self.workspace_dirs = dirs;
                self.workspace_files = files;
                self.expanded.insert(to_dir);
                self.pending_cursor = Some(target.clone());
                self.rebuild_meta();
                self.revision = self.revision.wrapping_add(1);
                self.preview_cache = None;
                self.tree_dirty = true;
                if let Some(old) = selected_path {
                    let moved = if old == from {
                        Some(target.clone())
                    } else {
                        old.strip_prefix(&from)
                            .ok()
                            .map(|rest| target.join(rest))
                    };
                    if let Some(path) = moved {
                        self.selected = self
                            .notes
                            .iter()
                            .find(|note| note.path == path)
                            .map(|note| note.id.clone());
                    }
                }
                self.set_toast("moved");
            }
            Err(err) => self.set_toast(format!("cannot move: {err}")),
        }
    }

    fn set_workspace(&mut self, dir: PathBuf, ctx: &egui::Context) {
        if !self.save_current() {
            return;
        }
        match Store::open_in(dir) {
            Ok(store) => {
                store::remember_workspace(&store.dir);
                let notes = store.load();
                let dirs = store.dirs();
                let files = store.files();
                let root = store.dir.clone();
                self.store = Some(store);
                self.store_error = None;
                self.notes = notes;
                self.workspace_dirs = dirs;
                self.workspace_files = files;
                self.expanded = HashSet::from([root]);
                self.tree_dirty = true;
                self.tree_cursor = 0;
                self.selected = self.notes.first().map(|note| note.id.clone());
                self.dirty = false;
                self.conflict = false;
                self.conflict_notified = false;
                self.preview_cache = None;
                self.revision = self.revision.wrapping_add(1);
                self.rebuild_meta();
                self.open_tabs.clear();
                if let Some(id) = &self.selected {
                    self.open_tabs.push(id.clone());
                }
                self.search.active = false;
                self.search.query.clear();
                self.leave_zen();
                self.sidebar = true;
                self.focus_list(ctx);
                self.set_toast("workspace opened");
            }
            Err(err) => self.set_toast(format!("cannot open folder: {err}")),
        }
    }

    fn open_folder(&mut self, ctx: &egui::Context) {
        let script = "POSIX path of (choose folder with prompt \"Open a folder in Notan\")";
        let output = Command::new("osascript").args(["-e", script]).output();
        let Ok(output) = output else {
            self.set_toast("cannot open folder picker");
            return;
        };
        if !output.status.success() {
            return;
        }
        let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if path.is_empty() {
            return;
        }
        let dir = PathBuf::from(path.trim_end_matches('/'));
        self.set_workspace(dir, ctx);
    }

    fn start_new_file(&mut self, dir: PathBuf, ctx: &egui::Context) {
        self.expanded.insert(dir.clone());
        self.inline = Some(InlineEdit {
            kind: InlineKind::NewFile,
            path: dir,
            text: String::new(),
        });
        self.tree_dirty = true;
        self.focus = Focus::List;
        let _ = ctx;
    }

    fn start_new_folder(&mut self, dir: PathBuf, ctx: &egui::Context) {
        self.expanded.insert(dir.clone());
        self.inline = Some(InlineEdit {
            kind: InlineKind::NewFolder,
            path: dir,
            text: String::new(),
        });
        self.tree_dirty = true;
        self.focus = Focus::List;
        let _ = ctx;
    }

    fn start_rename_cursor(&mut self, ctx: &egui::Context) {
        self.ensure_tree();
        let Some(row) = self.tree_rows.get(self.tree_cursor).cloned() else {
            return;
        };
        let text = row
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&row.name)
            .to_owned();
        self.inline = Some(InlineEdit {
            kind: if row.is_dir {
                InlineKind::RenameFolder
            } else {
                InlineKind::RenameFile
            },
            path: row.path,
            text,
        });
        self.focus = Focus::List;
        let _ = ctx;
    }

    fn reload_from_disk(&mut self) {
        if let Some(store) = &self.store {
            self.notes = store.load();
            self.workspace_dirs = store.dirs();
            self.workspace_files = store.files();
        }
        self.rebuild_meta();
        self.revision = self.revision.wrapping_add(1);
        self.preview_cache = None;
        self.tree_dirty = true;
    }

    fn commit_inline(&mut self, ctx: &egui::Context) {
        let Some(edit) = self.inline.take() else {
            return;
        };
        let name = edit.text.trim().to_owned();
        if name.is_empty() {
            return;
        }
        let Some(store) = &self.store else {
            return;
        };

        match edit.kind {
            InlineKind::NewFile => match store.create_named_in(&edit.path, &name) {
                Ok(note) => {
                    let id = note.id.clone();
                    let path = note.path.clone();
                    self.notes.insert(0, note);
                    if !self.open_tabs.iter().any(|tab| tab == &id) {
                        self.open_tabs.push(id.clone());
                    }
                    self.selected = Some(id);
                    self.expanded.insert(edit.path);
                    self.pending_cursor = Some(path);
                    self.revision = self.revision.wrapping_add(1);
                    self.refresh_dirs();
                    self.rebuild_meta();
                    self.set_toast("note created");
                }
                Err(err) => self.set_toast(format!("cannot create note: {err}")),
            },
            InlineKind::NewFolder => match store.create_dir(&edit.path, &name) {
                Ok(path) => {
                    self.expanded.insert(edit.path);
                    self.pending_cursor = Some(path);
                    self.refresh_dirs();
                    self.set_toast("folder created");
                }
                Err(err) => self.set_toast(format!("cannot create folder: {err}")),
            },
            InlineKind::RenameFile | InlineKind::RenameFolder => {
                let old = edit.path.clone();
                let old_id = store.rel_id(&old);
                let selected_before = self.selected.clone();
                let tabs_before = self.open_tabs.clone();
                match store.rename_path(&old, &name) {
                    Ok(target) => {
                        let new_id = store.rel_id(&target);
                        self.reload_from_disk();

                        if let Some(selected) = selected_before {
                            if selected == old_id {
                                self.selected = Some(new_id.clone());
                            } else if let Some(rest) =
                                selected.strip_prefix(&format!("{old_id}/"))
                            {
                                let moved = target.join(rest);
                                self.selected = self
                                    .store
                                    .as_ref()
                                    .map(|store| store.rel_id(&moved))
                                    .filter(|id| self.notes.iter().any(|note| &note.id == id));
                            }
                        }

                        let mut tabs = Vec::new();
                        for tab in tabs_before {
                            let mapped = if tab == old_id {
                                Some(new_id.clone())
                            } else if let Some(rest) =
                                tab.strip_prefix(&format!("{old_id}/"))
                            {
                                let moved = target.join(rest);
                                self.store
                                    .as_ref()
                                    .map(|store| store.rel_id(&moved))
                                    .filter(|id| self.notes.iter().any(|note| &note.id == id))
                            } else if self.notes.iter().any(|note| note.id == tab) {
                                Some(tab)
                            } else {
                                None
                            };
                            if let Some(id) = mapped {
                                if !tabs.contains(&id) {
                                    tabs.push(id);
                                }
                            }
                        }
                        self.open_tabs = tabs;

                        self.pending_cursor = Some(target);
                        self.set_toast("renamed");
                    }
                    Err(err) => self.set_toast(format!("cannot rename: {err}")),
                }
            }
        }

        self.focus_list(ctx);
    }

    fn refresh_selected_meta(&mut self) {
        let Some(index) = self.selected_index() else {
            return;
        };
        let note = &self.notes[index];
        let meta = NoteMeta {
            title: note.title(),
            snippet: snippet(&note.body),
        };
        self.note_meta.insert(note.uid, meta);
    }

    fn rebuild_meta(&mut self) {
        self.note_meta.clear();
        for note in &self.notes {
            self.note_meta.insert(
                note.uid,
                NoteMeta {
                    title: note.title(),
                    snippet: snippet(&note.body),
                },
            );
        }
    }

    fn current_title(&self) -> String {
        let Some(index) = self.selected_index() else {
            return "no note".to_owned();
        };
        let uid = self.notes[index].uid;
        if let Some(cache) = &self.preview_cache {
            if cache.uid == uid && cache.revision == self.revision {
                return cache.title.clone();
            }
        }
        self.notes[index].title()
    }

    fn word_count(&self) -> usize {
        let Some(index) = self.selected_index() else {
            return 0;
        };
        let uid = self.notes[index].uid;
        if let Some(cache) = &self.preview_cache {
            if cache.uid == uid && cache.revision == self.revision {
                return cache.words;
            }
        }
        self.notes[index].body.split_whitespace().count()
    }

    fn visible_ids(&self) -> Vec<String> {
        let query = self.search.query.trim().to_lowercase();
        if query.is_empty() {
            return self.notes.iter().map(|note| note.id.clone()).collect();
        }

        let mut hits: Vec<(u8, SystemTime, &Note)> = Vec::new();
        for note in &self.notes {
            let title = note.title().to_lowercase();
            let score = if title.starts_with(&query) {
                0
            } else if title.contains(&query) {
                1
            } else if note.body.to_lowercase().contains(&query) {
                2
            } else {
                continue;
            };
            hits.push((score, note.modified, note));
        }
        hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
        hits.into_iter().map(|(_, _, note)| note.id.clone()).collect()
    }

    fn set_toast(&mut self, message: impl Into<String>) {
        self.toast = Some((message.into(), Instant::now()));
    }

    fn focus_editor(&mut self, ctx: &egui::Context) {
        self.focus = Focus::Editor;
        self.preview_only = false;
        let id = self.editor_id();
        ctx.memory_mut(|m| m.request_focus(id));
    }

    fn focus_list(&mut self, ctx: &egui::Context) {
        self.focus = Focus::List;
        let id = self.editor_id();
        ctx.memory_mut(|m| m.surrender_focus(id));
        ctx.memory_mut(|m| m.surrender_focus(Id::new(SEARCH_ID)));
    }

    fn editor_id(&self) -> Id {
        self.selected_index()
            .map(|index| Id::new(("notan-editor", self.notes[index].uid)))
            .unwrap_or_else(|| Id::new(EDITOR_ID))
    }

    fn close_search(&mut self, ctx: &egui::Context) {
        self.search.active = false;
        self.search.query.clear();
        self.focus_list(ctx);
    }

    // ---------- note operations ----------

    fn select_note(&mut self, id: String, ctx: &egui::Context) {
        if self.selected.as_deref() == Some(id.as_str()) {
            return;
        }
        if !self.save_current() {
            return;
        }
        if !self.open_tabs.iter().any(|tab| tab == &id) {
            self.open_tabs.push(id.clone());
        }
        self.selected = Some(id);
        self.dirty = false;
        self.revision = self.revision.wrapping_add(1);
        if self.focus == Focus::Editor {
            let id = self.editor_id();
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }

    fn move_selection(&mut self, delta: isize, ctx: &egui::Context) {
        if !self.search.active {
            self.move_cursor(delta, ctx);
            return;
        }
        let visible = self.visible_ids();
        if visible.is_empty() {
            return;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|id| visible.iter().position(|candidate| candidate == id));
        let next = match current {
            Some(index) => {
                (index as isize + delta).clamp(0, visible.len() as isize - 1) as usize
            }
            None => 0,
        };
        self.select_note(visible[next].clone(), ctx);
    }

    fn save_current(&mut self) -> bool {
        self.save_note(false)
    }

    fn force_save(&mut self) -> bool {
        self.save_note(true)
    }

    /// Wrap the current selection in `marker`; with no selection, wrap the
    /// whole line. Wrapping already-wrapped text removes the markers.
    fn format_selection(&mut self, ctx: &egui::Context, marker: &str) {
        if self.search.active {
            return;
        }
        let Some(index) = self.selected_index() else {
            return;
        };
        let id = self.editor_id();
        let Some(mut state) = TextEditState::load(ctx, id) else {
            return;
        };
        let Some(range) = state.cursor.char_range() else {
            return;
        };

        let mut chars: Vec<char> = self.notes[index].body.chars().collect();
        let mut start = range.primary.index.min(range.secondary.index).min(chars.len());
        let mut end = range.primary.index.max(range.secondary.index).min(chars.len());

        if start == end {
            while start > 0 && chars.get(start - 1) != Some(&'\n') {
                start -= 1;
            }
            while end < chars.len() && chars.get(end) != Some(&'\n') {
                end += 1;
            }
        }
        if start == end {
            return;
        }

        let (next_start, next_end) = toggle_inline(&mut chars, start, end, marker);

        self.notes[index].body = chars.into_iter().collect();
        self.touch();
        self.focus = Focus::Editor;

        state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(next_start),
            CCursor::new(next_end),
        )));
        state.store(ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));
    }

    fn save_note(&mut self, force: bool) -> bool {
        let Some(id) = self.selected.clone() else {
            return true;
        };
        if !self.dirty {
            return true;
        }
        let Some(index) = self.notes.iter().position(|note| note.id == id) else {
            return true;
        };
        let Some(store) = &self.store else {
            return false;
        };

        match store.save(&mut self.notes[index], force) {
            Ok(SaveStatus::Saved) => {
                self.dirty = false;
                self.conflict = false;
                self.conflict_notified = false;
                self.save_failures = 0;
                self.retry_at = None;
                self.last_saved = Some(Instant::now());
                let new_id = self.notes[index].id.clone();
                if new_id != id {
                    for tab in &mut self.open_tabs {
                        if tab == &id {
                            *tab = new_id.clone();
                        }
                    }
                    self.selected = Some(new_id);
                    self.tree_dirty = true;
                }
                true
            }
            Ok(SaveStatus::Conflict) => {
                self.conflict = true;
                if !self.conflict_notified {
                    self.conflict_notified = true;
                    self.set_toast("note changed on disk — ⌘S overwrites, ⌘R reloads");
                }
                false
            }
            Err(err) => {
                self.save_failures = self.save_failures.saturating_add(1);
                let seconds = 1u64 << self.save_failures.min(4);
                self.retry_at = Some(Instant::now() + Duration::from_secs(seconds));
                if self.save_failures == 1 {
                    self.set_toast(format!("save failed: {err}"));
                }
                false
            }
        }
    }

    fn new_note(&mut self, ctx: &egui::Context) {
        let Some(dir) = self.current_dir() else {
            return;
        };
        self.create_note_in(dir, ctx);
    }

    fn create_note_in(&mut self, dir: PathBuf, ctx: &egui::Context) {
        if !self.save_current() {
            return;
        }
        let Some(store) = &self.store else {
            return;
        };

        match store.create_in(&dir) {
            Ok(note) => {
                let id = note.id.clone();
                let path = note.path.clone();
                self.notes.insert(0, note);
                if !self.open_tabs.iter().any(|tab| tab == &id) {
                    self.open_tabs.push(id.clone());
                }
                self.selected = Some(id);
                self.dirty = false;
                self.revision = self.revision.wrapping_add(1);
                self.rebuild_meta();
                self.expanded.insert(dir);
                self.pending_cursor = Some(path);
                self.refresh_dirs();
                self.search.active = false;
                self.search.query.clear();
                self.leave_zen();
                self.sidebar = true;
                self.focus_editor(ctx);
            }
            Err(err) => self.set_toast(format!("cannot create note: {err}")),
        }
    }

    fn prompt_delete(&mut self) {
        self.ensure_tree();
        let row = self.tree_rows.get(self.tree_cursor).cloned();
        self.pending = match row {
            Some(row) if row.is_dir => Some(Pending::DeletePath(row.path, true)),
            Some(row) if row.note_id.is_none() => {
                Some(Pending::DeletePath(row.path, false))
            }
            Some(_) | None if self.selected.is_some() => Some(Pending::DeleteNote),
            _ => None,
        };
        if self.pending.is_none() {
            return;
        }
        if matches!(self.pending, Some(Pending::DeleteNote)) && self.conflict {
            self.set_toast("note changed on disk — ⌘R reloads before trashing");
            self.pending = None;
            return;
        }
        self.overlay = Overlay::Confirm;
    }

    fn reveal_selected(&mut self) {
        self.ensure_tree();
        let path = self
            .tree_rows
            .get(self.tree_cursor)
            .map(|row| row.path.clone())
            .or_else(|| self.selected_index().map(|index| self.notes[index].path.clone()))
            .or_else(|| self.workspace_root());
        let Some(path) = path else {
            return;
        };
        match Command::new("open").arg("-R").arg(&path).spawn() {
            Ok(_) => self.set_toast("revealed in Finder"),
            Err(err) => self.set_toast(format!("cannot reveal: {err}")),
        }
    }

    fn copy_path(&mut self, ctx: &egui::Context) {
        self.ensure_tree();
        let path = self
            .tree_rows
            .get(self.tree_cursor)
            .map(|row| row.path.clone())
            .or_else(|| self.selected_index().map(|index| self.notes[index].path.clone()))
            .or_else(|| self.workspace_root());
        let Some(path) = path else {
            return;
        };
        ctx.copy_text(path.display().to_string());
        self.copied_at = Some(Instant::now());
        self.set_toast("path copied");
    }

    fn open_trash(&mut self) {
        let entries = self
            .store
            .as_ref()
            .map(Store::trash_entries)
            .unwrap_or_default();
        if entries.is_empty() {
            self.set_toast("trash is empty");
            return;
        }
        self.trash_entries = entries;
        self.trash_index = 0;
        self.overlay = Overlay::Trash;
    }

    fn trash_keys(&mut self, ctx: &egui::Context) {
        if self.trash_entries.is_empty() {
            self.overlay = Overlay::None;
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::J) || i.key_pressed(Key::ArrowDown)) {
            self.trash_index = (self.trash_index + 1) % self.trash_entries.len();
        }
        if ctx.input(|i| i.key_pressed(Key::K) || i.key_pressed(Key::ArrowUp)) {
            let len = self.trash_entries.len();
            self.trash_index = (self.trash_index + len - 1) % len;
        }
        if ctx.input(|i| i.key_pressed(Key::Enter)) {
            self.restore_trash(self.trash_index, ctx);
        }
    }

    fn restore_trash(&mut self, index: usize, ctx: &egui::Context) {
        let Some(path) = self.trash_entries.get(index).cloned() else {
            return;
        };
        let Some(store) = &self.store else {
            return;
        };
        match store.restore(&path) {
            Ok(Some(restored)) => {
                let id = store.rel_id(&restored);
                let notes = store.load();
                let dirs = store.dirs();
                let files = store.files();
                let entries = store.trash_entries();
                self.notes = notes;
                self.workspace_dirs = dirs;
                self.workspace_files = files;
                self.rebuild_meta();
                self.revision = self.revision.wrapping_add(1);
                self.preview_cache = None;
                self.tree_dirty = true;
                if self.notes.iter().any(|note| note.id == id) {
                    if !self.open_tabs.iter().any(|tab| tab == &id) {
                        self.open_tabs.push(id.clone());
                    }
                    self.selected = Some(id);
                }
                self.trash_entries = entries;
                if self.trash_entries.is_empty() {
                    self.overlay = Overlay::None;
                } else {
                    self.trash_index = self.trash_index.min(self.trash_entries.len() - 1);
                }
                self.focus_list(ctx);
                self.set_toast("restored from trash");
            }
            Ok(None) => self.set_toast("nothing to restore"),
            Err(err) => self.set_toast(format!("cannot restore: {err}")),
        }
    }

    fn prompt_clear_notes(&mut self) {
        if self.notes.is_empty() {
            self.set_toast("no notes to clear");
            return;
        }
        self.pending = Some(Pending::ClearNotes);
        self.overlay = Overlay::Confirm;
    }

    fn prompt_empty_trash(&mut self) {
        let count = self.store.as_ref().map(Store::trash_count).unwrap_or(0);
        if count == 0 {
            self.set_toast("trash is empty");
            return;
        }
        self.pending = Some(Pending::EmptyTrash);
        self.overlay = Overlay::Confirm;
    }

    fn confirm_pending(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending.take() else {
            self.overlay = Overlay::None;
            return;
        };
        self.overlay = Overlay::None;
        match pending {
            Pending::DeleteNote => self.delete_current(ctx),
            Pending::DeletePath(path, _) => self.delete_path(&path),
            Pending::EmptyTrash => self.empty_trash(),
            Pending::ClearNotes => self.clear_notes(),
        }
    }

    fn clear_notes(&mut self) {
        let Some(store) = &self.store else {
            return;
        };
        let paths: Vec<PathBuf> = self.notes.iter().map(|note| note.path.clone()).collect();
        let mut count = 0;
        for path in paths {
            if store.trash_path(&path).is_ok() {
                count += 1;
            }
        }
        let notes = store.load();
        let dirs = store.dirs();
        let files = store.files();
        self.notes = notes;
        self.workspace_dirs = dirs;
        self.workspace_files = files;
        self.selected = None;
        self.open_tabs.clear();
        self.rebuild_meta();
        self.revision = self.revision.wrapping_add(1);
        self.preview_cache = None;
        self.tree_dirty = true;
        self.dirty = false;
        self.set_toast(format!("cleared {count} notes to trash"));
    }

    fn delete_path(&mut self, path: &std::path::Path) {
        let Some(store) = &self.store else {
            return;
        };
        match store.trash_path(path) {
            Ok(_) => {
                let notes = store.load();
                let dirs = store.dirs();
                let files = store.files();
                self.notes = notes;
                self.workspace_dirs = dirs;
                self.workspace_files = files;
                self.rebuild_meta();
                self.revision = self.revision.wrapping_add(1);
                self.preview_cache = None;
                self.tree_dirty = true;
                let keep = self
                    .selected
                    .clone()
                    .filter(|id| self.notes.iter().any(|note| &note.id == id));
                self.selected = keep.or_else(|| self.notes.first().map(|n| n.id.clone()));
                self.prune_tabs();
                self.set_toast("moved to trash");
            }
            Err(err) => self.set_toast(format!("cannot trash: {err}")),
        }
    }

    fn delete_current(&mut self, ctx: &egui::Context) {
        if !self.save_current() {
            return;
        }
        let Some(index) = self.selected_index() else {
            return;
        };
        let Some(store) = &self.store else {
            return;
        };

        match store.trash(&self.notes[index]) {
            Ok(trashed) => {
                self.last_trashed = Some(trashed);
                let uid = self.notes[index].uid;
                self.note_meta.remove(&uid);
                self.revision = self.revision.wrapping_add(1);
                self.tree_dirty = true;
                self.notes.remove(index);
                self.dirty = false;
                self.selected = if self.notes.is_empty() {
                    None
                } else {
                    let next = index.min(self.notes.len() - 1);
                    Some(self.notes[next].id.clone())
                };
                self.prune_tabs();
                self.focus_list(ctx);
                self.set_toast("note moved to trash — ⌘⇧Z restores it");
            }
            Err(err) => self.set_toast(format!("cannot trash note: {err}")),
        }
    }

    fn restore_last(&mut self) {
        let Some(trashed) = self.last_trashed.clone() else {
            self.set_toast("nothing to restore");
            return;
        };
        let Some(store) = &self.store else {
            return;
        };
        match store.restore(&trashed) {
            Ok(Some(path)) => {
                let notes = store.load();
                let dirs = store.dirs();
                let files = store.files();
                let id = store.rel_id(&path);
                self.last_trashed = None;
                self.notes = notes;
                self.workspace_dirs = dirs;
                self.workspace_files = files;
                self.tree_dirty = true;
                self.rebuild_meta();
                self.revision = self.revision.wrapping_add(1);
                if !self.open_tabs.iter().any(|tab| tab == &id) {
                    self.open_tabs.push(id.clone());
                }
                self.selected = Some(id);
                self.dirty = false;
                self.conflict = false;
                self.conflict_notified = false;
                self.set_toast("note restored");
            }
            Ok(None) => {
                self.last_trashed = None;
                self.set_toast("nothing to restore");
            }
            Err(err) => self.set_toast(format!("cannot restore note: {err}")),
        }
    }

    fn empty_trash(&mut self) {
        let Some(store) = &self.store else {
            return;
        };
        match store.empty_trash() {
            Ok(0) => self.set_toast("trash is empty"),
            Ok(count) => {
                self.last_trashed = None;
                let plural = if count == 1 { "note" } else { "notes" };
                self.set_toast(format!("deleted {count} trashed {plural}"));
            }
            Err(err) => self.set_toast(format!("cannot empty trash: {err}")),
        }
    }

    fn reload(&mut self, ctx: &egui::Context) {
        if !self.conflict && !self.save_current() {
            return;
        }
        let Some(store) = &self.store else {
            return;
        };
        self.notes = store.load();
        self.workspace_dirs = store.dirs();
        self.workspace_files = store.files();
        self.tree_dirty = true;
        self.rebuild_meta();
        self.revision = self.revision.wrapping_add(1);
        self.preview_cache = None;
        let keep = self
            .selected
            .clone()
            .filter(|id| self.notes.iter().any(|note| &note.id == id));
        self.selected = keep.or_else(|| self.notes.first().map(|note| note.id.clone()));
        self.prune_tabs();
        self.dirty = false;
        self.conflict = false;
        self.conflict_notified = false;
        self.save_failures = 0;
        self.retry_at = None;
        if self.focus == Focus::Editor {
            let id = self.editor_id();
            ctx.memory_mut(|m| m.request_focus(id));
        }
        self.set_toast(format!("{} notes on disk", self.notes.len()));
    }

    fn reveal_folder(&mut self) {
        let Some(store) = &self.store else {
            return;
        };
        match Command::new("open").arg(&store.dir).spawn() {
            Ok(_) => self.set_toast(format!("opened {}", store.dir.display())),
            Err(err) => self.set_toast(format!("cannot open folder: {err}")),
        }
    }

    // ---------- keyboard ----------

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Q))
        }) {
            if !self.conflict {
                self.save_current();
            }
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        if self.inline.is_some() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.inline = None;
                return;
            }
            if ctx.input(|i| i.key_pressed(Key::Enter)) {
                self.commit_inline(ctx);
                return;
            }
            return;
        }

        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            match self.overlay {
                Overlay::Palette | Overlay::Help => {
                    self.overlay = Overlay::None;
                    return;
                }
                Overlay::Confirm => {
                    self.overlay = Overlay::None;
                    self.pending = None;
                    return;
                }
                Overlay::Trash => {
                    self.overlay = Overlay::None;
                    return;
                }
                Overlay::None => {}
            }
            if self.search.active {
                self.close_search(ctx);
                return;
            }
            if self.preview_only {
                self.preview_only = false;
                return;
            }
            if self.zen {
                self.zen = false;
                self.sidebar = true;
                self.preview = true;
                return;
            }
            if self.focus == Focus::Editor {
                self.focus_list(ctx);
            }
            return;
        }

        match self.overlay {
            Overlay::Palette => {
                self.palette_keys(ctx);
                return;
            }
            Overlay::Confirm => {
                self.confirm_keys(ctx);
                return;
            }
            Overlay::Trash => {
                self.trash_keys(ctx);
                return;
            }
            Overlay::Help => return,
            Overlay::None => {}
        }

        if self.focus == Focus::Editor
            && !self.search.active
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab))
        {
            self.hold_editor_focus(ctx);
            self.insert_text(ctx, "    ");
            return;
        }

        self.global_shortcuts(ctx);
        if self.overlay != Overlay::None {
            return;
        }

        if self.search.active {
            if ctx.input(|i| i.key_pressed(Key::Enter)) {
                self.finish_search(ctx);
            }
            return;
        }

        if self.focus == Focus::List {
            self.list_keys(ctx);
        }
    }

    fn global_shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);

        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::ALT, Key::ArrowUp)))
        {
            self.move_selection(-1, ctx);
            return;
        }
        if ctx
            .input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::ALT, Key::ArrowDown)))
        {
            self.move_selection(1, ctx);
            return;
        }

        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::N))) {
            self.new_note(ctx);
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::K))) {
            self.open_palette(ctx);
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::F))) {
            self.open_search(ctx);
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::S))) {
            if self.conflict {
                if self.force_save() {
                    self.set_toast("overwrote the version on disk");
                }
            } else if self.save_current() {
                self.set_toast("saved");
            }
            return;
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::N,
            ))
        }) {
            if let Some(dir) = self.target_dir() {
                self.start_new_folder(dir, ctx);
                self.sidebar = true;
            }
            return;
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::ALT,
                Key::Backspace,
            ))
        }) {
            self.prompt_clear_notes();
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::B))) {
            self.format_selection(ctx, "**");
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::I))) {
            self.format_selection(ctx, "*");
            return;
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::Z,
            ))
        }) {
            self.restore_last();
            return;
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::Backspace,
            ))
        }) {
            self.prompt_empty_trash();
            return;
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::P,
            ))
        }) {
            self.preview_only = !self.preview_only;
            if self.preview_only {
                self.preview = true;
            }
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::P))) {
            if self.preview_only {
                self.preview_only = false;
                return;
            }
            self.leave_zen();
            self.preview = !self.preview;
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::Backslash))) {
            self.leave_zen();
            if self.sidebar && self.search.active {
                self.close_search(ctx);
            }
            self.sidebar = !self.sidebar;
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::E))) {
            self.toggle_zen(ctx);
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::Backspace))) {
            self.prompt_delete();
            return;
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::C,
            ))
        }) {
            self.copy_path(ctx);
            return;
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::T,
            ))
        }) {
            self.open_trash();
            return;
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::R,
            ))
        }) {
            self.reveal_selected();
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::R))) {
            self.reload(ctx);
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::W))) {
            self.close_tab(ctx);
            return;
        }
        let number_keys = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        for (index, key) in number_keys.into_iter().enumerate() {
            if ctx.input_mut(|i| i.consume_shortcut(&cmd(key))) {
                self.switch_tab(index, ctx);
                return;
            }
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::O,
            ))
        }) {
            self.reveal_folder();
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::O))) {
            self.open_folder(ctx);
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::Slash))) {
            self.overlay = Overlay::Help;
        }
    }

    fn list_keys(&mut self, ctx: &egui::Context) {
        self.ensure_tree();

        let (shift, down, up, right, left, enter, tab, question) = ctx.input(|i| {
            (
                i.modifiers.shift,
                i.key_pressed(Key::J) || i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::K) || i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::L),
                i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::H),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Tab) || i.key_pressed(Key::E),
                i.key_pressed(Key::Questionmark),
            )
        });

        if down {
            self.move_cursor(1, ctx);
        }
        if up {
            self.move_cursor(-1, ctx);
        }
        if right {
            self.expand_or_open(ctx);
        }
        if left {
            self.collapse_or_parent();
        }
        if enter {
            self.start_rename_cursor(ctx);
        }
        if ctx.input(|i| i.key_pressed(Key::Space)) {
            self.activate_cursor(ctx);
        }
        if tab && self.selected.is_some() {
            self.focus_editor(ctx);
        }
        if ctx.input(|i| i.key_pressed(Key::N)) {
            if let Some(dir) = self.target_dir() {
                if shift {
                    self.start_new_folder(dir, ctx);
                } else {
                    self.start_new_file(dir, ctx);
                }
            }
        }
        if ctx.input(|i| i.key_pressed(Key::D)) {
            self.prompt_delete();
        }

        let (slash, slash_shift) = ctx.input(|i| (i.key_pressed(Key::Slash), i.modifiers.shift));
        if slash && !slash_shift {
            self.open_search(ctx);
        }
        if ctx.input(|i| i.key_pressed(Key::P)) {
            self.leave_zen();
            self.preview = !self.preview;
        }
        if ctx.input(|i| i.key_pressed(Key::R)) {
            self.reload(ctx);
        }
        if question || (slash && slash_shift) {
            self.overlay = Overlay::Help;
        }
    }

    fn palette_keys(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::K))
        }) {
            self.overlay = Overlay::None;
            return;
        }

        let matches = self.palette_matches();
        if matches.is_empty() {
            return;
        }
        let len = matches.len();
        if self.palette_index >= len {
            self.palette_index = len - 1;
        }
        if ctx.input(|i| i.key_pressed(Key::ArrowDown)) {
            self.palette_index = (self.palette_index + 1) % len;
            self.palette_scroll = true;
        }
        if ctx.input(|i| i.key_pressed(Key::ArrowUp)) {
            self.palette_index = (self.palette_index + len - 1) % len;
            self.palette_scroll = true;
        }
        if ctx.input(|i| i.key_pressed(Key::Enter)) {
            let index = self.palette_index.min(len - 1);
            let action = ACTIONS[matches[index]].action;
            self.run_action(action, ctx);
        }
    }

    fn confirm_keys(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.key_pressed(Key::Y) || i.key_pressed(Key::Enter)) {
            self.confirm_pending(ctx);
        }
        if ctx.input(|i| i.key_pressed(Key::N)) {
            self.overlay = Overlay::None;
            self.pending = None;
        }
    }

    fn open_palette(&mut self, ctx: &egui::Context) {
        self.overlay = Overlay::Palette;
        self.palette_query.clear();
        self.palette_index = 0;
        self.palette_scroll = false;
        ctx.memory_mut(|m| m.request_focus(Id::new(PALETTE_ID)));
    }

    fn open_search(&mut self, ctx: &egui::Context) {
        self.overlay = Overlay::None;
        self.search.active = true;
        self.search.query.clear();
        self.leave_zen();
        self.sidebar = true;
        self.focus = Focus::List;
        ctx.memory_mut(|m| m.surrender_focus(Id::new(EDITOR_ID)));
        ctx.memory_mut(|m| m.request_focus(Id::new(SEARCH_ID)));
    }

    fn finish_search(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.visible_ids().first().cloned() {
            self.select_note(id, ctx);
            self.search.active = false;
            self.search.query.clear();
            self.focus_editor(ctx);
        } else {
            self.close_search(ctx);
        }
    }

    fn toggle_zen(&mut self, ctx: &egui::Context) {
        if self.search.active {
            self.close_search(ctx);
        }
        self.zen = !self.zen;
        if self.zen {
            self.sidebar = false;
            self.preview = false;
        } else {
            self.sidebar = true;
            self.preview = true;
        }
    }

    fn leave_zen(&mut self) {
        if self.zen {
            self.zen = false;
            self.sidebar = true;
            self.preview = true;
        }
    }

    fn palette_matches(&self) -> Vec<usize> {
        let query = self.palette_query.trim().to_lowercase();
        ACTIONS
            .iter()
            .enumerate()
            .filter(|(_, action)| {
                query.is_empty()
                    || action.label.to_lowercase().contains(&query)
                    || action.keywords.contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn run_action(&mut self, action: Action, ctx: &egui::Context) {
        self.overlay = Overlay::None;
        match action {
            Action::NewNote => self.new_note(ctx),
            Action::SaveNow => {
                if self.conflict {
                    if self.force_save() {
                        self.set_toast("overwrote the version on disk");
                    }
                } else if self.save_current() {
                    self.set_toast("saved");
                }
            }
            Action::OpenFolder => self.open_folder(ctx),
            Action::NewFolder => {
                if let Some(dir) = self.target_dir() {
                    self.start_new_folder(dir, ctx);
                    self.sidebar = true;
                }
            }
            Action::FindNote => self.open_search(ctx),
            Action::CommandPalette => self.open_palette(ctx),
            Action::TogglePreview => {
                self.leave_zen();
                self.preview = !self.preview;
            }
            Action::PreviewOnly => {
                self.preview_only = !self.preview_only;
                if self.preview_only {
                    self.preview = true;
                }
            }
            Action::ToggleSidebar => {
                self.leave_zen();
                if self.sidebar && self.search.active {
                    self.close_search(ctx);
                }
                self.sidebar = !self.sidebar;
            }
            Action::ToggleFocus => self.toggle_zen(ctx),
            Action::BoldLine => self.format_selection(ctx, "**"),
            Action::ItalicLine => self.format_selection(ctx, "*"),
            Action::DeleteNote => self.prompt_delete(),
            Action::RestoreLastTrash => self.restore_last(),
            Action::EmptyTrash => self.prompt_empty_trash(),
            Action::ClearNotes => self.prompt_clear_notes(),
            Action::CopyPath => self.copy_path(ctx),
            Action::RevealSelected => self.reveal_selected(),
            Action::OpenTrash => self.open_trash(),
            Action::Refresh => self.reload(ctx),
            Action::RevealFolder => self.reveal_folder(),
            Action::Help => self.overlay = Overlay::Help,
            Action::Quit => {
                if !self.conflict {
                    self.save_current();
                }
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }

    fn autosave(&mut self) {
        if !self.dirty || self.conflict {
            return;
        }
        if self.retry_at.is_some_and(|at| Instant::now() < at) {
            return;
        }
        if self.last_edit.elapsed() >= Duration::from_millis(AUTOSAVE_MS) {
            self.save_current();
        }
    }

    // ---------- drawing ----------

    fn draw(&mut self, ctx: &egui::Context) {
        self.top_bar(ctx);
        self.status_bar(ctx);
        if self.sidebar {
            self.sidebar_panel(ctx);
        }
        self.central_panel(ctx);
        if self.overlay != Overlay::None {
            self.overlay_ui(ctx);
        }
        self.drag_ghost(ctx);
        self.toast_ui(ctx);
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        let logo = self.logo_texture(ctx);
        let mut commands = false;
        egui::TopBottomPanel::top("notan-top")
            .exact_height(theme::TOP_BAR_HEIGHT)
            .frame(
                Frame::new()
                    .fill(theme::TOP_BAR_BG)
                    .inner_margin(Margin::symmetric(10, 0)),
            )
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.horizontal_centered(|ui| {
                    let (logo_rect, _) =
                        ui.allocate_exact_size(egui::vec2(26.0, 26.0), Sense::hover());
                    ui.painter().image(
                        logo,
                        logo_rect,
                        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (gear_rect, gear) =
                            ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::click());
                        let gear_color = if gear.hovered() {
                            theme::TEXT_BRIGHT
                        } else {
                            theme::DIM
                        };
                        theme::paint_icon(
                            ui.painter(),
                            theme::icon(ui.ctx(), "settings"),
                            Rect::from_center_size(gear_rect.center(), egui::vec2(14.0, 14.0)),
                            gear_color,
                        );
                        if gear.hovered() {
                            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                        }
                        if gear.clicked() {
                            commands = true;
                        }
                        let _ = gear.on_hover_text("commands");
                        ui.add_space(6.0);
                        let (state, color) = if self.conflict {
                            ("changed on disk", theme::DANGER)
                        } else if self.dirty {
                            ("editing", theme::DIM)
                        } else {
                            ("saved", theme::FAINT)
                        };
                        ui.label(RichText::new(state).size(9.5).color(color));
                    });
                });
                theme::hairline_bottom(ui, rect);
            });
        if commands {
            self.open_palette(ctx);
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("notan-status")
            .exact_height(theme::STATUS_BAR_HEIGHT)
            .frame(
                Frame::new()
                    .fill(theme::BG)
                    .inner_margin(Margin::symmetric(18, 0)),
            )
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.left(), rect.top()),
                        egui::pos2(rect.right(), rect.top()),
                    ],
                    Stroke::new(1.0, theme::LINE_SOFT),
                );
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new(self.hints()).size(10.0).color(theme::FAINT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let words = self.word_count();
                        ui.label(
                            RichText::new(format!(
                                "{words} {}",
                                if words == 1 { "word" } else { "words" }
                            ))
                            .size(10.0)
                            .color(theme::FAINT),
                        );
                        ui.label(RichText::new("·").size(10.0).color(theme::FAINT));
                        let notes = self.notes.len();
                        ui.label(
                            RichText::new(format!(
                                "{notes} {}",
                                if notes == 1 { "note" } else { "notes" }
                            ))
                            .size(10.0)
                            .color(theme::FAINT),
                        );
                        if let Some(at) = self.copied_at {
                            let elapsed = at.elapsed();
                            if elapsed < Duration::from_millis(1600) {
                                ui.label(RichText::new("·").size(10.0).color(theme::FAINT));
                                ui.label(
                                    RichText::new("copied").size(10.0).color(theme::DIM),
                                );
                                ctx.request_repaint_after(
                                    Duration::from_millis(1600) - elapsed,
                                );
                            }
                        }
                    });
                });
            });
    }

    fn hints(&self) -> &'static str {
        if self.preview_only {
            return "preview only · ⌘⇧P editor · esc back · ⌘K commands";
        }
        match self.overlay {
            Overlay::Palette => "↑↓ select · ↵ run · esc close",
            Overlay::Help => "esc close",
            Overlay::Confirm => "y confirm · esc cancel",
            Overlay::Trash => "↑↓ select · ↵ restore · esc close",
            Overlay::None => {
                if self.conflict {
                    "changed on disk · ⌘S overwrite · ⌘R reload"
                } else if self.search.active {
                    "type to filter · ↵ open · esc clear"
                } else if self.zen {
                    "esc leave focus · ⌘P preview · ⌘K commands"
                } else if self.focus == Focus::Editor {
                    "esc notes · ⌘S save · ⌘B bold · ⌘I italic · ⌘K commands"
                } else {
                    "j k move · ↵ open · n new · ⇧N folder · / find · ⌘K commands"
                }
            }
        }
    }

    fn sidebar_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("notan-sidebar")
            .resizable(false)
            .exact_width(theme::SIDEBAR_WIDTH)
            .frame(
                Frame::new()
                    .fill(theme::SIDEBAR_BG)
                    .inner_margin(Margin::symmetric(8, 10)),
            )
            .show(ctx, |ui| {
                let workspace = self
                    .store
                    .as_ref()
                    .and_then(|store| store.dir.file_name())
                    .and_then(|name| name.to_str())
                    .unwrap_or("notes")
                    .to_uppercase();
                let root = self.workspace_root();
                let mut new_note = false;
                let mut new_folder = false;

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(truncate(&workspace, 24))
                            .size(9.5)
                            .color(theme::FAINT),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{}", self.notes.len()))
                                .size(9.5)
                                .color(theme::FAINT),
                        );
                        new_folder = header_icon(ui, HeaderIcon::NewFolder).clicked();
                        new_note = header_icon(ui, HeaderIcon::NewNote).clicked();
                    });
                });
                ui.add_space(4.0);

                if new_note {
                    if let Some(root) = root.clone() {
                        self.start_new_file(root, ctx);
                    }
                }
                if new_folder {
                    if let Some(root) = root {
                        self.start_new_folder(root, ctx);
                    }
                }

                if self.search.active {
                    Frame::new()
                        .fill(theme::PANEL)
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(Margin::symmetric(2, 2))
                        .show(ui, |ui| {
                            ui.add(
                                TextEdit::singleline(&mut self.search.query)
                                    .id(Id::new(SEARCH_ID))
                                    .frame(false)
                                    .desired_width(f32::INFINITY)
                                    .hint_text(
                                        RichText::new("find in notes…")
                                            .size(12.0)
                                            .color(theme::FAINT),
                                    )
                                    .font(FontId::proportional(12.0))
                                    .margin(Margin::symmetric(8, 5)),
                            );
                        });
                    ui.add_space(6.0);
                }

                if self.notes.iter().any(|note| !self.note_meta.contains_key(&note.uid)) {
                    self.rebuild_meta();
                }

                if self.search.active {
                    self.sidebar_search(ui, ctx);
                } else {
                    self.sidebar_tree(ui, ctx);
                }
            });
    }

    fn sidebar_search(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let visible = self.visible_ids();
        let selected = self.selected.clone();
        let mut clicked: Option<(String, bool)> = None;

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if visible.is_empty() {
                    ui.add_space(18.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("no matches").size(11.0).color(theme::FAINT));
                    });
                    return;
                }

                for id in &visible {
                    let Some(note) = self.notes.iter().find(|note| &note.id == id) else {
                        continue;
                    };
                    let is_selected = selected.as_deref() == Some(id.as_str());
                    let age = store::relative_time(note.modified);
                    let meta = &self.note_meta[&note.uid];
                    let response = note_row(ui, &meta.title, &age, &meta.snippet, is_selected);
                    if response.clicked() {
                        clicked = Some((id.clone(), response.double_clicked()));
                    }
                }
            });

        if let Some((id, open_editor)) = clicked {
            self.close_search(ctx);
            self.select_note(id, ctx);
            self.focus_list(ctx);
            if open_editor {
                self.focus_editor(ctx);
            }
        }
    }

    fn sidebar_tree(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        self.ensure_tree();

        let pending_new = self.inline.as_ref().is_some_and(|edit| {
            matches!(edit.kind, InlineKind::NewFile | InlineKind::NewFolder)
        });
        if self.tree_rows.is_empty() && !pending_new {
            ui.add_space(18.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("empty folder — n note · ⇧N folder")
                        .size(11.0)
                        .color(theme::FAINT),
                );
            });
            return;
        }

        let root = self.workspace_root();
        let inline_kind = self.inline.as_ref().map(|edit| edit.kind);
        let inline_path = self.inline.as_ref().map(|edit| edit.path.clone());
        let new_inline = matches!(
            inline_kind,
            Some(InlineKind::NewFile | InlineKind::NewFolder)
        );
        let mut new_drawn = false;

        let mut clicked_row: Option<usize> = None;
        let mut toggled: Option<PathBuf> = None;
        let mut opened: Option<(String, bool)> = None;
        let mut drag_started: Option<PathBuf> = None;
        let mut drop_hover: Option<PathBuf> = None;
        let mut drop_released = false;
        let mut folder_action: Option<(PathBuf, FolderAction)> = None;

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;

                if new_inline && inline_path == root {
                    if let Some(edit) = &mut self.inline {
                        let text = &mut edit.text;
                        inline_row(ui, text, 0);
                    }
                    new_drawn = true;
                }

                for index in 0..self.tree_rows.len() {
                    let row = self.tree_rows[index].clone();
                    let renaming = self.inline.as_ref().is_some_and(|edit| {
                        matches!(edit.kind, InlineKind::RenameFile | InlineKind::RenameFolder)
                            && edit.path == row.path
                    });
                    if renaming {
                        if let Some(edit) = &mut self.inline {
                            let text = &mut edit.text;
                            inline_row(ui, text, row.depth);
                        }
                        continue;
                    }
                    let is_cursor = index == self.tree_cursor;
                    let is_drop = row.is_dir
                        && self.drag_path.is_some()
                        && self.drop_target.as_deref() == Some(row.path.as_path());
                    let (response, action) = if row.is_dir {
                        let expanded = self.expanded.contains(&row.path);
                        tree_dir_row(
                            ui, &row.name, row.depth, expanded, is_cursor, is_drop,
                        )
                    } else {
                        let selected = row.note_id.as_deref() == self.selected.as_deref();
                        (
                            tree_file_row(
                                ui,
                                &row.name,
                                row.depth,
                                selected,
                                is_cursor,
                                row.supported,
                            ),
                            None,
                        )
                    };
                    if let Some(action) = action {
                        clicked_row = Some(index);
                        folder_action = Some((row.path.clone(), action));
                    }
                    if new_inline
                        && row.is_dir
                        && inline_path.as_deref() == Some(row.path.as_path())
                    {
                        if let Some(edit) = &mut self.inline {
                            let text = &mut edit.text;
                            inline_row(ui, text, row.depth + 1);
                        }
                        new_drawn = true;
                    }
                    if response.drag_started() {
                        drag_started = Some(row.path.clone());
                    }
                    if self.drag_path.is_some()
                        && row.is_dir
                        && ui
                            .ctx()
                            .pointer_latest_pos()
                            .is_some_and(|pos| response.rect.contains(pos))
                    {
                        drop_hover = Some(row.path.clone());
                    }
                    if response.drag_stopped() {
                        drop_released = true;
                    }
                    if response.clicked() {
                        self.inline = None;
                        clicked_row = Some(index);
                        if row.is_dir {
                            toggled = Some(row.path.clone());
                        } else if let Some(id) = row.note_id.clone() {
                            opened = Some((id, response.double_clicked()));
                        }
                    }
                }
            });

        if let Some(path) = drag_started {
            self.drag_path = Some(path);
            self.drop_target = None;
        }
        if let Some(path) = drop_hover {
            self.drop_target = Some(path);
        }
        if drop_released {
            if let (Some(from), Some(to)) = (self.drag_path.take(), self.drop_target.take()) {
                self.move_entry(from, to);
            }
        }

        if let Some((dir, action)) = folder_action {
            match action {
                FolderAction::NewNote => self.start_new_file(dir, ctx),
                FolderAction::NewFolder => self.start_new_folder(dir, ctx),
            }
        }

        if let Some(index) = clicked_row {
            self.tree_cursor = index;
        }
        if let Some(path) = toggled {
            if !self.expanded.remove(&path) {
                self.expanded.insert(path);
            }
            self.tree_dirty = true;
        }
        if let Some((id, open_editor)) = opened {
            self.select_note(id, ctx);
            self.focus_list(ctx);
            if open_editor {
                self.focus_editor(ctx);
            }
        }
    }

    fn central_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(theme::BG))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                self.tabs_ui(ui, ctx);
                if self.store_error.is_some() {
                    self.welcome(ui);
                    return;
                }
                if self.selected.is_none() {
                    self.welcome(ui);
                    return;
                }

                if self.preview_only {
                    self.preview_ui(ui);
                    return;
                }
                if self.zen || !self.preview {
                    self.editor_ui(ui);
                    return;
                }

                let width = ui.available_width();
                let height = ui.available_height();
                let divider = 1.0;
                let left_width = ((width - divider) * 0.5).floor();

                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.allocate_ui_with_layout(
                        egui::vec2(left_width, height),
                        Layout::top_down(Align::Min),
                        |ui| self.editor_ui(ui),
                    );
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(divider, height), Sense::hover());
                    ui.painter()
                        .rect_filled(rect, CornerRadius::ZERO, theme::LINE_SOFT);
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), height),
                        Layout::top_down(Align::Min),
                        |ui| self.preview_ui(ui),
                    );
                });
            });
    }

    fn tabs_ui(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        self.open_tabs
            .retain(|id| self.notes.iter().any(|note| &note.id == id));
        if self.open_tabs.is_empty() {
            return;
        }
        let tabs: Vec<(String, String, bool)> = self
            .open_tabs
            .iter()
            .map(|id| {
                let title = self
                    .notes
                    .iter()
                    .find(|note| &note.id == id)
                    .map(|note| {
                        self.note_meta
                            .get(&note.uid)
                            .map(|meta| meta.title.clone())
                            .unwrap_or_else(|| note.title())
                    })
                    .unwrap_or_else(|| id.clone());
                (id.clone(), title, self.selected.as_deref() == Some(id.as_str()))
            })
            .collect();

        let mut clicked: Option<String> = None;
        let mut closed: Option<String> = None;
        ScrollArea::horizontal()
            .id_salt("notan-tabs-scroll")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (id, title, active) in &tabs {
                        let (response, close) = tab_row(ui, &truncate(title, 18), *active);
                        if close {
                            closed = Some(id.clone());
                        } else if response.clicked() {
                            clicked = Some(id.clone());
                        }
                    }
                });
            });
        if let Some(id) = closed {
            self.close_tab_id(id, ctx);
        } else if let Some(id) = clicked {
            self.select_note(id, ctx);
        }
    }

    fn welcome(&mut self, ui: &mut Ui) {
        let logo = self.logo_texture(ui.ctx());
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.18).max(28.0));
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(110.0, 96.0), Sense::hover());
            let time = ui.input(|i| i.time) as f32;
            let _ = &response;
            let offset = (time * 1.7).sin() * 4.0;
            let image = Rect::from_center_size(
                rect.center() + egui::vec2(0.0, offset),
                egui::vec2(74.0, 74.0),
            );
            ui.painter().image(
                logo,
                image,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            ui.ctx()
                .request_repaint_after(Duration::from_millis(16));
            ui.add_space(4.0);
            ui.label(
                RichText::new("spec workspace · markdown · offline · keyboard only")
                    .size(11.5)
                    .color(theme::DIM),
            );
            ui.add_space(26.0);

            if let Some(error) = &self.store_error {
                ui.label(RichText::new(error).size(11.5).color(theme::DANGER));
                ui.add_space(10.0);
                hint_line(ui, "⌘O", "open folder");
            } else {
                hint_line(ui, "n", "new note");
                hint_line(ui, "⇧N", "new folder");
                hint_line(ui, "⌘O", "open folder");
                hint_line(ui, "/", "find");
                hint_line(ui, "⌘K", "commands");
            }

            ui.add_space(20.0);
            if let Some(store) = &self.store {
                ui.label(
                    RichText::new(format!("workspace: {}", store.dir.display()))
                        .size(10.0)
                        .color(theme::FAINT),
                );
            }
        });
    }

    fn editor_ui(&mut self, ui: &mut Ui) {
        let Some(index) = self.selected_index() else {
            return;
        };
        ui.spacing_mut().item_spacing.x = 8.0;
        let editor_id = self.editor_id();
        let uid = self.notes[index].uid;

        let mut changed = false;
        let mut clicked = false;
        let mut focused = false;
        let mut selection: Option<String> = None;
        let mut arrow: Option<(usize, usize, &'static str)> = None;

        ScrollArea::vertical()
            .id_salt(("editor-scroll", uid))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                Frame::new()
                    .inner_margin(Margin::symmetric(34, 26))
                    .show(ui, |ui| {
                        let width = ui.available_width().min(780.0);
                        ui.set_width(width);
                        let output = TextEdit::multiline(&mut self.notes[index].body)
                            .id(editor_id)
                            .frame(false)
                            .desired_width(f32::INFINITY)
                            .desired_rows(28)
                            .font(FontId::new(14.5, FontFamily::Monospace))
                            .margin(Margin::ZERO)
                            .hint_text(
                                RichText::new("start writing…")
                                    .size(13.5)
                                    .color(theme::FAINT),
                            )
                            .show(ui);

                        let response = &output.response;
                        changed = response.changed();
                        clicked = response.clicked();
                        focused = response.has_focus();

                        if focused {
                            if let Some(range) = output.cursor_range {
                                let body = &self.notes[index].body;
                                if changed && range.primary.index == range.secondary.index {
                                    arrow = arrow_at(body, range.primary.index);
                                }
                                if range.primary.index != range.secondary.index
                                    && ui.input(|i| i.pointer.any_released())
                                {
                                    let start =
                                        range.primary.index.min(range.secondary.index);
                                    let end = range.primary.index.max(range.secondary.index);
                                    let text: String =
                                        body.chars().skip(start).take(end - start).collect();
                                    if !text.trim().is_empty() {
                                        selection = Some(text);
                                    }
                                }
                            }
                        }
                    });
            });

        if changed {
            self.touch();
        }
        if clicked || focused {
            self.focus = Focus::Editor;
        }
        if let Some(text) = selection {
            ui.ctx().copy_text(text);
            self.copied_at = Some(Instant::now());
        }
        if let Some((start, end, symbol)) = arrow {
            let id = self.editor_id();
            self.notes[index].body = replace_chars(&self.notes[index].body, start, end, symbol);
            if let Some(mut state) = TextEditState::load(ui.ctx(), id) {
                state
                    .cursor
                    .set_char_range(Some(CCursorRange::one(CCursor::new(start + 1))));
                state.store(ui.ctx(), id);
            }
            self.touch();
        }
    }

    /// Insert text at the cursor, replacing any selection.
    fn insert_text(&mut self, ctx: &egui::Context, text: &str) {
        let Some(index) = self.selected_index() else {
            return;
        };
        let id = self.editor_id();
        let Some(mut state) = TextEditState::load(ctx, id) else {
            return;
        };
        let Some(range) = state.cursor.char_range() else {
            return;
        };
        let start = range.primary.index.min(range.secondary.index);
        let end = range.primary.index.max(range.secondary.index);
        self.notes[index].body = replace_chars(&self.notes[index].body, start, end, text);
        self.touch();
        let caret = start + text.chars().count();
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
        state.store(ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));
    }

    /// Tab and arrows also drive egui's focus navigation, which runs before
    /// our key handling. Cancel that direction and keep the editor focused.
    fn hold_editor_focus(&self, ctx: &egui::Context) {
        let id = self.editor_id();
        ctx.memory_mut(|m| {
            m.move_focus(FocusDirection::None);
            m.request_focus(id);
        });
    }

    fn preview_ui(&mut self, ui: &mut Ui) {
        let Some(index) = self.selected_index() else {
            return;
        };
        ui.spacing_mut().item_spacing.x = 8.0;
        let uid = self.notes[index].uid;

        let stale = self
            .preview_cache
            .as_ref()
            .is_none_or(|cache| cache.uid != uid || cache.revision != self.revision);
        if stale {
            let body = &self.notes[index].body;
            let parsed = markdown::parse(body);
            let words = body.split_whitespace().count();
            let title = self.notes[index].title();
            let kind = parsed.meta.kind.clone();
            let kind_color = parsed
                .meta
                .color
                .as_deref()
                .and_then(theme::heading_color)
                .or_else(|| kind.as_deref().and_then(theme::type_color));
            self.preview_cache = Some(PreviewCache {
                uid,
                revision: self.revision,
                blocks: parsed.blocks,
                words,
                title,
                kind,
                kind_color,
            });
        }

        let cache = self.preview_cache.as_ref().expect("preview cache");
        let blocks = &cache.blocks;
        let badge = cache.kind.clone();
        let badge_color = cache.kind_color;
        let base = self.notes[index]
            .path
            .parent()
            .map(std::path::Path::to_path_buf);
        let empty = self.notes[index].body.trim().is_empty();
        let available = ui.available_width();
        let width = (available - 76.0).clamp(160.0, 720.0);
        let pad = ((available - width) * 0.5).max(38.0);
        let mut action = None;

        ScrollArea::vertical()
            .id_salt(("preview-scroll", uid))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;

                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        Frame::new()
                            .inner_margin(Margin::symmetric(0, 30))
                            .show(ui, |ui| {
                                ui.set_width(width);
                                ui.spacing_mut().item_spacing.y = 0.0;
                                if empty {
                                    ui.label(
                                        RichText::new("preview appears here")
                                            .size(11.5)
                                            .color(theme::FAINT),
                                    );
                                } else {
                                    if let Some(kind) = &badge {
                                        ui.label(
                                            RichText::new(kind.to_uppercase())
                                                .size(9.5)
                                                .color(badge_color.unwrap_or(theme::ACCENT)),
                                        );
                                        ui.add_space(10.0);
                                    }
                                    action = markdown::show(ui, blocks, base.as_deref());
                                }
                            });
                    });
                });
            });

        match action {
            Some(markdown::Action::Copy(text)) => {
                ui.ctx().copy_text(text);
                self.copied_at = Some(Instant::now());
            }
            Some(markdown::Action::ToggleTask(task)) => self.toggle_task(task),
            Some(markdown::Action::OpenLink(href)) => self.open_link(&href, ui.ctx()),
            None => {}
        }
    }

    /// Flip the n-th task list marker in the current note.
    fn open_link(&mut self, href: &str, ctx: &egui::Context) {
        if href.starts_with('#') {
            self.set_toast("anchor links are not supported");
            return;
        }
        if href.starts_with("http://")
            || href.starts_with("https://")
            || href.starts_with("mailto:")
        {
            self.set_toast("external links are disabled offline");
            return;
        }
        let Some(index) = self.selected_index() else {
            return;
        };
        let parent = self.notes[index].path.parent().map(std::path::Path::to_path_buf);
        let Some(parent) = parent else {
            return;
        };
        let href = href.split('#').next().unwrap_or(href);
        let mut target = if href.starts_with('/') {
            match self.workspace_root() {
                Some(root) => root.join(href.trim_start_matches('/')),
                None => parent.join(href),
            }
        } else {
            parent.join(href)
        };
        if target.extension().is_none() {
            target.set_extension("md");
        }

        let resolved = std::fs::canonicalize(&target).unwrap_or(target);
        let found = self.notes.iter().find(|note| {
            std::fs::canonicalize(&note.path).is_ok_and(|path| path == resolved)
        });
        if let Some(note) = found {
            let id = note.id.clone();
            self.select_note(id, ctx);
            self.focus_list(ctx);
        } else {
            self.set_toast("link target not found");
        }
    }

    fn toggle_task(&mut self, task: usize) {
        let Some(index) = self.selected_index() else {
            return;
        };
        if let Some(body) = toggle_task_in(&self.notes[index].body, task) {
            self.notes[index].body = body;
            self.touch();
        }
    }

    // ---------- overlays ----------

    fn overlay_ui(&mut self, ctx: &egui::Context) {
        let backdrop_clicked = egui::Area::new(Id::new("notan-backdrop"))
            .fixed_pos(ctx.content_rect().min)
            .order(Order::Foreground)
            .show(ctx, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(ctx.content_rect().size(), Sense::click());
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::ZERO,
                    Color32::from_rgba_premultiplied(0, 0, 0, 120),
                );
                response.clicked()
            })
            .inner;

        if backdrop_clicked {
            self.overlay = Overlay::None;
            self.pending = None;
            return;
        }

        match self.overlay {
            Overlay::Palette => self.palette_ui(ctx),
            Overlay::Help => self.help_ui(ctx),
            Overlay::Confirm => self.confirm_ui(ctx),
            Overlay::Trash => self.trash_ui(ctx),
            Overlay::None => {}
        }
    }

    fn palette_ui(&mut self, ctx: &egui::Context) {
        let mut chosen: Option<Action> = None;

        egui::Area::new(Id::new("notan-palette-area"))
            .anchor(Align2::CENTER_TOP, [0.0, 84.0])
            .order(Order::Tooltip)
            .default_size(egui::vec2(520.0, 600.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(theme::PANEL_RAISED)
                    .stroke(Stroke::new(1.0, theme::LINE))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(9))
                    .show(ui, |ui| {
                        ui.set_width(500.0);
                        let response = ui.add(
                            TextEdit::singleline(&mut self.palette_query)
                                .id(Id::new(PALETTE_ID))
                                .frame(false)
                                .desired_width(f32::INFINITY)
                                .hint_text(
                                    RichText::new("run a command…")
                                        .size(14.0)
                                        .color(theme::FAINT),
                                )
                                .font(FontId::proportional(14.0))
                                .margin(Margin::symmetric(10, 8)),
                        );
                        if response.changed() {
                            self.palette_index = 0;
                            self.palette_scroll = false;
                        }

                        ui.add_space(4.0);

                        let matches = self.palette_matches();
                        if matches.is_empty() {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("no matching commands")
                                    .size(11.5)
                                    .color(theme::FAINT),
                            );
                            ui.add_space(8.0);
                            return;
                        }

                        if self.palette_index >= matches.len() {
                            self.palette_index = matches.len() - 1;
                        }

                        let max_height =
                            (ctx.content_rect().height() - 180.0).clamp(120.0, 460.0);
                        ScrollArea::vertical()
                            .max_height(max_height)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 6.0;
                                for (row, &action_index) in matches.iter().enumerate() {
                                    let def = &ACTIONS[action_index];
                                    let selected = row == self.palette_index;
                                    let response =
                                        menu_row(ui, def.label, def.keys, selected);
                                    if selected && self.palette_scroll {
                                        response.scroll_to_me(Some(Align::Center));
                                    }
                                    if response.clicked() {
                                        chosen = Some(def.action);
                                    }
                                }
                                ui.add_space(4.0);
                            });
                        self.palette_scroll = false;
                    });
            });

        if let Some(action) = chosen {
            self.run_action(action, ctx);
        }
    }

    fn help_ui(&mut self, ctx: &egui::Context) {
        egui::Area::new(Id::new("notan-help"))
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .order(Order::Tooltip)
            .default_size(egui::vec2(700.0, 470.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(theme::PANEL_RAISED)
                    .stroke(Stroke::new(1.0, theme::LINE))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(22))
                    .show(ui, |ui| {
                        ui.set_width(660.0);
                        ui.label(
                            RichText::new("keyboard")
                                .size(16.0)
                                .color(theme::TEXT_BRIGHT),
                        );
                        ui.add_space(3.0);
                        ui.label(
                            RichText::new("every action has a key; nothing has a button")
                                .size(10.5)
                                .color(theme::FAINT),
                        );
                        ui.add_space(12.0);

                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.allocate_ui_with_layout(
                                egui::vec2(310.0, 0.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.spacing_mut().item_spacing.y = 4.0;
                                    help_section(ui, "everywhere");
                                    help_row(ui, "Command palette", "⌘K");
                                    help_row(ui, "New note", "⌘N");
                                    help_row(ui, "New folder", "⌘⇧N");
                                    help_row(ui, "Open folder", "⌘O");
                                    help_row(ui, "Close tab", "⌘W");
                                    help_row(ui, "Switch tab", "⌘1…9");
                                    help_row(ui, "Find note", "⌘F");
                                    help_row(ui, "Save now", "⌘S");
                                    help_row(ui, "Toggle preview", "⌘P");
                                    help_row(ui, "Preview only", "⌘⇧P");
                                    help_row(ui, "Toggle sidebar", "⌘\\");
                                    help_row(ui, "Focus mode", "⌘E");
                                    help_row(ui, "Next / previous note", "⌥↑ / ⌥↓");
                                    help_row(ui, "Move note to trash", "⌘⌫");
                                    help_row(ui, "Restore last trashed note", "⌘⇧Z");
                                    help_row(ui, "Trash browser", "⌘⇧T");
                                    help_row(ui, "Empty trash", "⌘⇧⌫");
                                    help_row(ui, "Clear all notes", "⌥⌘⌫");
                                    help_row(ui, "Copy selected path", "⌘⇧C");
                                    help_row(ui, "Reveal selected", "⌘⇧R");
                                    help_row(ui, "Reload from disk", "⌘R");
                                    help_row(ui, "Reveal workspace", "⌘⇧O");
                                    help_row(ui, "Keyboard reference", "⌘/");
                                    help_row(ui, "Quit", "⌘Q");
                                },
                            );
                            ui.add_space(40.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(310.0, 0.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.spacing_mut().item_spacing.y = 4.0;
                                    help_section(ui, "notes list");
                                    help_row(ui, "Move selection", "j k / ↑ ↓");
                                    help_row(ui, "Open / expand", "↵ / →");
                                    help_row(ui, "Collapse / parent", "←");
                                    help_row(ui, "Open in editor", "e / tab");
                                    help_row(ui, "New note", "n");
                                    help_row(ui, "New folder", "⇧N / ⌘⇧N");
                                    help_row(ui, "Find", "/");
                                    help_row(ui, "Trash file or folder", "d");
                                    help_row(ui, "Reload", "r");
                                    help_row(ui, "Keyboard reference", "?");

                                    help_section(ui, "writing");
                                    help_row(ui, "Bold selection or line", "⌘B");
                                    help_row(ui, "Italicise selection or line", "⌘I");
                                    help_row(ui, "Back to notes list", "esc");
                                    help_row(ui, "Autosave", "on idle");
                                },
                            );
                        });

                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("esc close")
                                .size(10.0)
                                .color(theme::FAINT),
                        );
                    });
            });
    }

    fn trash_ui(&mut self, ctx: &egui::Context) {
        let entries = self.trash_entries.clone();
        let mut restore: Option<usize> = None;

        egui::Area::new(Id::new("notan-trash"))
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .order(Order::Tooltip)
            .default_size(egui::vec2(460.0, 420.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(theme::PANEL_RAISED)
                    .stroke(Stroke::new(1.0, theme::LINE))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(18))
                    .show(ui, |ui| {
                        ui.set_width(420.0);
                        ui.label(
                            RichText::new("trash")
                                .size(15.0)
                                .color(theme::TEXT_BRIGHT),
                        );
                        ui.label(
                            RichText::new("enter restores · folders go back to the root")
                                .size(10.5)
                                .color(theme::FAINT),
                        );
                        ui.add_space(10.0);
                        ScrollArea::vertical()
                            .max_height(320.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                for (index, path) in entries.iter().enumerate() {
                                    let name = path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("item");
                                    let selected = index == self.trash_index;
                                    let response =
                                        menu_row(ui, &truncate(name, 48), "", selected);
                                    if response.clicked() {
                                        self.trash_index = index;
                                        restore = Some(index);
                                    }
                                }
                            });
                    });
            });

        if let Some(index) = restore {
            self.restore_trash(index, ctx);
        }
    }

    fn confirm_ui(&mut self, ctx: &egui::Context) {
        let (heading, detail, note) = match &self.pending {
            Some(Pending::EmptyTrash) => {
                let count = self.store.as_ref().map(Store::trash_count).unwrap_or(0);
                let plural = if count == 1 { "note" } else { "notes" };
                (
                    "delete the trash permanently?".to_owned(),
                    format!("{count} trashed {plural}"),
                    "this cannot be undone",
                )
            }
            Some(Pending::ClearNotes) => (
                "clear all notes?".to_owned(),
                format!("{} Markdown files", self.notes.len()),
                "they move to .trash and can be emptied later",
            ),
            Some(Pending::DeletePath(path, is_dir)) => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("item");
                (
                    if *is_dir {
                        "move folder to trash?".to_owned()
                    } else {
                        "move file to trash?".to_owned()
                    },
                    truncate(name, 64),
                    if *is_dir {
                        "everything inside it moves too"
                    } else {
                        "it stays in .trash"
                    },
                )
            }
            _ => (
                "move note to trash?".to_owned(),
                truncate(&self.current_title(), 64),
                "⌘⇧Z restores it",
            ),
        };

        egui::Area::new(Id::new("notan-confirm"))
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .order(Order::Tooltip)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(theme::PANEL)
                    .stroke(Stroke::new(1.0, theme::LINE))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(22))
                    .show(ui, |ui| {
                        ui.set_min_width(300.0);
                        ui.label(
                            RichText::new(heading)
                                .size(15.0)
                                .color(theme::TEXT_BRIGHT),
                        );
                        ui.add_space(6.0);
                        ui.label(RichText::new(detail).size(12.0).color(theme::DIM));
                        ui.add_space(4.0);
                        ui.label(RichText::new(note).size(10.5).color(theme::FAINT));
                        ui.add_space(14.0);
                        ui.label(
                            RichText::new("y confirm · esc cancel")
                                .size(10.5)
                                .color(theme::FAINT),
                        );
                    });
            });
    }

    fn drag_ghost(&self, ctx: &egui::Context) {
        let (Some(path), Some(pos)) = (&self.drag_path, ctx.pointer_latest_pos()) else {
            return;
        };
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("item")
            .to_owned();
        egui::Area::new(Id::new("notan-drag"))
            .order(Order::Tooltip)
            .fixed_pos(pos + egui::vec2(12.0, 10.0))
            .interactable(false)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(theme::PANEL_RAISED)
                    .stroke(Stroke::new(1.0, theme::LINE))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.label(RichText::new(name).size(10.5).color(theme::TEXT));
                    });
            });
    }

    fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some((message, at)) = &self.toast else {
            return;
        };
        if at.elapsed() > Duration::from_secs(3) {
            self.toast = None;
            return;
        }
        let message = message.clone();

        egui::Area::new(Id::new("notan-toast"))
            .anchor(Align2::CENTER_BOTTOM, [0.0, -44.0])
            .order(Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(theme::PANEL_RAISED)
                    .stroke(Stroke::new(1.0, theme::LINE))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(13, 6))
                    .show(ui, |ui| {
                        ui.label(RichText::new(message).size(11.0).color(theme::TEXT));
                    });
            });
    }
}

impl Notan {
    fn frame(&mut self, ctx: &egui::Context) {
        if let Some(size) = self.pending_window.take() {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
        let theme_flag = Id::new("notan-theme-installed");
        if ctx.data(|data| data.get_temp::<bool>(theme_flag)).is_none() {
            theme::install(ctx);
            ctx.data_mut(|data| data.insert_temp(theme_flag, true));
        }

        self.poll_external();
        if self.last_session_save.elapsed() > Duration::from_secs(10) {
            self.save_session(ctx);
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            self.save_current();
            self.save_session(ctx);
        }

        self.autosave();
        self.handle_keys(ctx);
        self.draw(ctx);

        if self.dirty {
            let elapsed = self.last_edit.elapsed();
            let wait = Duration::from_millis(AUTOSAVE_MS).saturating_sub(elapsed);
            ctx.request_repaint_after(wait);
        }
        if let Some(at) = self.retry_at {
            let wait = at.saturating_duration_since(Instant::now());
            ctx.request_repaint_after(wait.max(Duration::from_millis(120)));
        }
        if self
            .toast
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(3))
        {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        if self.last_saved.is_some() {
            // Keeps the relative timestamps in the sidebar fresh.
            ctx.request_repaint_after(Duration::from_secs(30));
        }
    }
}

impl eframe::App for Notan {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame(ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = theme::BG;
        [
            c.r() as f32 / 255.0,
            c.g() as f32 / 255.0,
            c.b() as f32 / 255.0,
            1.0,
        ]
    }
}

// ---------- small widgets ----------

fn replace_chars(body: &str, start: usize, end: usize, text: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    if start > end || end > chars.len() {
        return body.to_owned();
    }
    let mut out = String::with_capacity(body.len() + text.len());
    out.extend(&chars[..start]);
    out.push_str(text);
    out.extend(&chars[end..]);
    out
}

/// `->` and `<-` typed in prose become arrows. Code blocks are untouched.
fn arrow_at(body: &str, caret: usize) -> Option<(usize, usize, &'static str)> {
    if caret < 2 || code_lang_at(body, caret).is_some() {
        return None;
    }
    let chars: Vec<char> = body.chars().collect();
    match (chars.get(caret - 2), chars.get(caret - 1)) {
        (Some('-'), Some('>')) => Some((caret - 2, caret, "→")),
        (Some('<'), Some('-')) => Some((caret - 2, caret, "←")),
        _ => None,
    }
}

/// Language of the fenced code block containing `caret`, if any.
fn code_lang_at(body: &str, caret: usize) -> Option<String> {
    let line = body.chars().take(caret).filter(|c| *c == '\n').count();
    let mut lang: Option<String> = None;
    for (index, text) in body.lines().enumerate() {
        if index >= line {
            break;
        }
        if let Some(rest) = text.trim_start().strip_prefix("```") {
            lang = if lang.is_none() {
                Some(rest.trim().to_owned())
            } else {
                None
            };
        }
    }
    lang.filter(|l| !l.is_empty())
}

/// Index of the `[ ]` / `[x]` marker inside a list-item line, in chars.
fn task_marker_index(line: &str) -> Option<usize> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }
    match chars.get(i) {
        Some('-' | '*' | '+') => i += 1,
        Some(c) if c.is_ascii_digit() => {
            while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
                i += 1;
            }
            if !matches!(chars.get(i), Some('.' | ')')) {
                return None;
            }
            i += 1;
        }
        _ => return None,
    }
    if chars.get(i) != Some(&' ') {
        return None;
    }
    while chars.get(i) == Some(&' ') {
        i += 1;
    }
    if chars.get(i) == Some(&'[')
        && matches!(chars.get(i + 1), Some(' ' | 'x' | 'X'))
        && chars.get(i + 2) == Some(&']')
    {
        Some(i + 1)
    } else {
        None
    }
}

fn toggle_task_in(body: &str, target: usize) -> Option<String> {
    let mut seen = 0;
    let mut changed = false;
    let mut out = String::with_capacity(body.len());

    for line in body.split_inclusive('\n') {
        match task_marker_index(line) {
            Some(at) if seen == target => {
                let mut chars: Vec<char> = line.chars().collect();
                chars[at] = if chars[at] == ' ' { 'x' } else { ' ' };
                out.extend(&chars);
                changed = true;
                seen += 1;
            }
            Some(_) => {
                out.push_str(line);
                seen += 1;
            }
            None => out.push_str(line),
        }
    }

    changed.then_some(out)
}

fn chars_at(chars: &[char], at: usize, needle: &str) -> bool {
    needle
        .chars()
        .enumerate()
        .all(|(offset, c)| chars.get(at + offset) == Some(&c))
}

/// Add or remove `marker` around `start..end`, returning the new character
/// range. Handles selection that already includes the markers.
fn toggle_inline(chars: &mut Vec<char>, start: usize, end: usize, marker: &str) -> (usize, usize) {
    let width = marker.chars().count();

    if start >= width
        && end + width <= chars.len()
        && chars_at(chars, start - width, marker)
        && chars_at(chars, end, marker)
    {
        chars.drain(end..end + width);
        chars.drain(start - width..start);
        return (start - width, end - width);
    }

    if chars_at(chars, start, marker)
        && end >= start + width
        && chars_at(chars, end - width, marker)
    {
        chars.drain(end - width..end);
        chars.drain(start..start + width);
        return (start, end - width * 2);
    }

    let open: Vec<char> = marker.chars().collect();
    let mut out = Vec::with_capacity(chars.len() + open.len() * 2);
    out.extend_from_slice(&chars[..start]);
    out.extend_from_slice(&open);
    out.extend_from_slice(&chars[start..end]);
    out.extend_from_slice(&open);
    out.extend_from_slice(&chars[end..]);
    *chars = out;

    (start + width, end + width)
}

fn inline_row(ui: &mut Ui, text: &mut String, depth: usize) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(34.0 + depth as f32 * 12.0);
        let id = Id::new(INLINE_ID);
        let response = ui.add(
            TextEdit::singleline(text)
                .id(id)
                .frame(false)
                .desired_width((ui.available_width() - 8.0).max(40.0))
                .font(FontId::proportional(11.5))
                .text_color(theme::TEXT_BRIGHT)
                .margin(Margin::symmetric(2, 2)),
        );
        if !response.has_focus() {
            ui.ctx().memory_mut(|m| m.request_focus(id));
        }
        response
    })
    .inner
}

fn tab_row(ui: &mut Ui, title: &str, active: bool) -> (egui::Response, bool) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(140.0, 30.0), Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::ACTIVE);
    } else if response.hovered() {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::HOVER);
    }

    let clip = Rect::from_min_max(
        egui::pos2(rect.left() + 8.0, rect.top()),
        egui::pos2(rect.right() - 22.0, rect.bottom()),
    );
    painter.with_clip_rect(clip).text(
        egui::pos2(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(11.0),
        if active { theme::TEXT_BRIGHT } else { theme::DIM },
    );

    let close_rect = Rect::from_center_size(
        egui::pos2(rect.right() - 11.0, rect.center().y),
        egui::vec2(16.0, 16.0),
    );
    let close = ui.interact(close_rect, response.id.with("close"), Sense::click());
    if active || response.hovered() || close.hovered() {
        let color = if close.hovered() {
            theme::TEXT_BRIGHT
        } else {
            theme::FAINT
        };
        theme::paint_icon(
            painter,
            theme::icon(ui.ctx(), "x"),
            Rect::from_center_size(close_rect.center(), egui::vec2(10.0, 10.0)),
            color,
        );
    }
    if close.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    (
        response.on_hover_cursor(CursorIcon::PointingHand),
        close.clicked(),
    )
}

#[derive(Clone, Copy)]
enum HeaderIcon {
    NewNote,
    NewFolder,
}

#[derive(Clone, Copy)]
enum FolderAction {
    NewNote,
    NewFolder,
}

fn header_icon(ui: &mut Ui, icon: HeaderIcon) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::click());
    let color = if response.hovered() {
        theme::TEXT_BRIGHT
    } else {
        theme::DIM
    };
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(3), theme::HOVER);
    }
    let name = match icon {
        HeaderIcon::NewNote => "file-plus",
        HeaderIcon::NewFolder => "folder-plus",
    };
    theme::paint_icon(
        ui.painter(),
        theme::icon(ui.ctx(), name),
        Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0)),
        color,
    );
    let label = match icon {
        HeaderIcon::NewNote => "new note at root",
        HeaderIcon::NewFolder => "new folder at root",
    };
    response
        .on_hover_text(label)
        .on_hover_cursor(CursorIcon::PointingHand)
}

fn tree_dir_row(
    ui: &mut Ui,
    name: &str,
    depth: usize,
    expanded: bool,
    cursor: bool,
    drop: bool,
) -> (egui::Response, Option<FolderAction>) {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, 24.0), Sense::click_and_drag());
    let painter = ui.painter();

    if drop {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::HOVER);
        painter.rect_stroke(
            rect.shrink(1.0),
            CornerRadius::ZERO,
            Stroke::new(1.0, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    } else if cursor {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::ACTIVE);
    } else if response.hovered() {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::HOVER);
    }

    for guide in 0..depth {
        let x = rect.left() + 9.0 + guide as f32 * 12.0;
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            Stroke::new(1.0, theme::LINE_SOFT),
        );
    }

    let x = rect.left() + 4.0 + depth as f32 * 12.0;
    let chevron = egui::pos2(x + 7.0, rect.center().y);
    theme::paint_icon(
        painter,
        theme::icon(ui.ctx(), if expanded { "chevron-down" } else { "chevron-right" }),
        Rect::from_center_size(chevron, egui::vec2(12.0, 12.0)),
        theme::DIM,
    );
    theme::paint_icon(
        painter,
        theme::icon(ui.ctx(), "folder"),
        Rect::from_center_size(
            egui::pos2(x + 22.0, rect.center().y),
            egui::vec2(15.0, 15.0),
        ),
        theme::DIM,
    );

    let clip = Rect::from_min_max(
        egui::pos2(x + 30.0, rect.top()),
        egui::pos2(rect.right() - 44.0, rect.bottom()),
    );
    painter.with_clip_rect(clip).text(
        egui::pos2(x + 30.0, rect.center().y),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(11.5),
        if cursor { theme::TEXT_BRIGHT } else { theme::TEXT },
    );

    let hovered = ui.rect_contains_pointer(rect);
    let note_rect = Rect::from_center_size(
        egui::pos2(rect.right() - 34.0, rect.center().y),
        egui::vec2(16.0, 16.0),
    );
    let folder_rect = Rect::from_center_size(
        egui::pos2(rect.right() - 16.0, rect.center().y),
        egui::vec2(16.0, 16.0),
    );
    let note_btn = ui.interact(note_rect, response.id.with("new-note"), Sense::click());
    let folder_btn = ui.interact(folder_rect, response.id.with("new-folder"), Sense::click());

    let mut action = None;
    if note_btn.clicked() {
        action = Some(FolderAction::NewNote);
    } else if folder_btn.clicked() {
        action = Some(FolderAction::NewFolder);
    }

    if hovered {
        let note_color = if note_btn.hovered() {
            theme::TEXT_BRIGHT
        } else {
            theme::FAINT
        };
        let folder_color = if folder_btn.hovered() {
            theme::TEXT_BRIGHT
        } else {
            theme::FAINT
        };
        theme::paint_icon(
            painter,
            theme::icon(ui.ctx(), "file-plus"),
            Rect::from_center_size(note_rect.center(), egui::vec2(14.0, 14.0)),
            note_color,
        );
        theme::paint_icon(
            painter,
            theme::icon(ui.ctx(), "folder-plus"),
            Rect::from_center_size(folder_rect.center(), egui::vec2(14.0, 14.0)),
            folder_color,
        );
    }

    (
        response.on_hover_cursor(CursorIcon::PointingHand),
        action,
    )
}

fn tree_file_row(
    ui: &mut Ui,
    name: &str,
    depth: usize,
    selected: bool,
    cursor: bool,
    supported: bool,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, 24.0), Sense::click_and_drag());
    let painter = ui.painter();

    if cursor {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::ACTIVE);
    } else if selected {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::SELECTED_ROW);
    } else if response.hovered() {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::HOVER);
    }
    if selected {
        painter.rect_filled(
            Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
            CornerRadius::ZERO,
            theme::ACCENT,
        );
    }

    for guide in 0..depth {
        let x = rect.left() + 9.0 + guide as f32 * 12.0;
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            Stroke::new(1.0, theme::LINE_SOFT),
        );
    }

    let x = rect.left() + 4.0 + depth as f32 * 12.0;
    let icon_color = if supported {
        theme::FAINT
    } else {
        theme::LINE
    };
    theme::paint_icon(
        painter,
        theme::icon(ui.ctx(), "file"),
        Rect::from_center_size(
            egui::pos2(x + 22.0, rect.center().y),
            egui::vec2(14.0, 14.0),
        ),
        icon_color,
    );

    let clip = Rect::from_min_max(
        egui::pos2(x + 30.0, rect.top()),
        egui::pos2(rect.right() - 4.0, rect.bottom()),
    );
    let color = if !supported {
        theme::FAINT
    } else if selected || cursor {
        theme::TEXT_BRIGHT
    } else {
        theme::TEXT
    };
    painter.with_clip_rect(clip).text(
        egui::pos2(x + 30.0, rect.center().y),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(11.5),
        color,
    );

    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn note_row(ui: &mut Ui, title: &str, age: &str, snippet: &str, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 50.0), Sense::click());
    let painter = ui.painter();

    if selected {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::SELECTED_ROW);
        painter.rect_filled(
            Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
            CornerRadius::ZERO,
            theme::ACCENT,
        );
    } else if response.hovered() {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::HOVER);
    }

    let title_clip = Rect::from_min_max(
        egui::pos2(rect.left() + 12.0, rect.top()),
        egui::pos2(rect.right() - 46.0, rect.bottom()),
    );
    painter.with_clip_rect(title_clip).text(
        egui::pos2(rect.left() + 13.0, rect.top() + 9.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(12.5),
        if selected {
            theme::TEXT_BRIGHT
        } else {
            theme::TEXT
        },
    );

    painter.text(
        egui::pos2(rect.right() - 12.0, rect.top() + 10.0),
        Align2::RIGHT_TOP,
        age,
        FontId::proportional(9.5),
        theme::FAINT,
    );

    let snippet_clip = Rect::from_min_max(
        egui::pos2(rect.left() + 12.0, rect.top() + 27.0),
        egui::pos2(rect.right() - 12.0, rect.bottom() - 4.0),
    );
    painter.with_clip_rect(snippet_clip).text(
        egui::pos2(rect.left() + 13.0, rect.top() + 27.5),
        Align2::LEFT_TOP,
        snippet,
        FontId::proportional(10.0),
        theme::FAINT,
    );

    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn menu_row(ui: &mut Ui, label: &str, keys: &str, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 30.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, CornerRadius::same(4), theme::ACTIVE);
    } else if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(4), theme::HOVER);
    }

    painter.text(
        egui::pos2(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.5),
        if selected {
            theme::TEXT_BRIGHT
        } else {
            theme::TEXT
        },
    );
    painter.text(
        egui::pos2(rect.right() - 10.0, rect.center().y),
        Align2::RIGHT_CENTER,
        space_keys(keys),
        FontId::new(10.5, FontFamily::Monospace),
        theme::FAINT,
    );

    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn help_section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .size(9.0)
            .color(theme::FAINT),
    );
    ui.add_space(2.0);
}

fn space_keys(keys: &str) -> String {
    const MODIFIERS: &str = "⌘⌥⇧⌃";
    let mut out = String::new();
    for ch in keys.chars() {
        if let Some(prev) = out.chars().last() {
            let needs_space = prev != ' '
                && ch != ' '
                && (MODIFIERS.contains(prev) || MODIFIERS.contains(ch));
            if needs_space {
                out.push(' ');
            }
        }
        out.push(ch);
    }
    out
}

fn help_row(ui: &mut Ui, label: &str, keys: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0).color(theme::TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(space_keys(keys))
                    .size(11.0)
                    .color(theme::DIM)
                    .monospace(),
            );
        });
    });
}

fn hint_line(ui: &mut Ui, key: &str, label: &str) {
    let row_width = 220.0;
    let pad = ((ui.available_width() - row_width) * 0.5).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.allocate_ui_with_layout(
            egui::vec2(44.0, 18.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.label(
                    RichText::new(key)
                        .size(11.0)
                        .color(theme::ACCENT)
                        .monospace(),
                );
            },
        );
        ui.add_space(6.0);
        ui.label(RichText::new(label).size(11.5).color(theme::DIM));
    });
}

fn snippet(body: &str) -> String {
    let mut lines = body
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.to_lowercase().starts_with("type:")
                && !line.to_lowercase().starts_with("color:")
        });
    let _title = lines.next();
    let Some(line) = lines.next() else {
        return "empty note".to_owned();
    };
    let cleaned = line
        .trim()
        .trim_start_matches(['#', '>', '-', '*', '+'])
        .trim()
        .replace(['*', '`', '_'], "");
    if cleaned.is_empty() {
        "empty note".to_owned()
    } else {
        truncate(&cleaned, 58)
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn frame(ctx: &egui::Context, app: &mut Notan) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1120.0, 720.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| app.frame(ctx));
    }

    #[test]
    #[ignore = "timing; run manually"]
    fn bench_large_note_frames() {
        let dir = std::env::temp_dir().join(format!(
            "notan-bench-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("specs")).unwrap();
        std::fs::write(
            dir.join("specs").join("auth.md"),
            "# Auth spec\n\nSession tokens, rotation, and expiry.\n",
        )
        .unwrap();
        std::fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
        let image = image::RgbaImage::from_fn(320, 110, |x, y| {
            let g = (90 + (x % 90)) as u8;
            image::Rgba([g, 140, (80 + (y % 60)) as u8, 255])
        });
        image.save(dir.join("diagram.png")).unwrap();
        let store = Store::open_in(dir.clone()).unwrap();
        let mut note = store.create().unwrap();
        let mut body = String::new();
        for i in 0..40 {
            body.push_str(&format!(
                "## Section {i}\n\nSome **bold** and `code` text with a [link](x).\n\n- item one\n- item two\n\n```rust\nfn x() -> i32 {{ 42 }}\n```\n\n"
            ));
        }
        note.body = body;
        store.save(&mut note, false).unwrap();
        drop(note);

        let app = Notan::from_context(&egui::Context::default(), Ok(store));
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1120.0, 720.0))
            .build_state(
                |ctx, app: &mut Notan| {
                    app.frame(ctx);
                },
                app,
            );
        theme::install(&harness.ctx);
        harness.run_steps(3);

        let start = std::time::Instant::now();
        for _ in 0..20 {
            harness.step();
        }
        println!(
            "frame: {:?} per frame ({} KiB)",
            start.elapsed() / 20,
            harness.state().notes[0].body.len() / 1024
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    #[ignore = "renders preview images; run manually"]
    fn render_previews() {
        let dir = std::env::temp_dir().join("notan-preview").join("Notan");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("specs")).unwrap();
        std::fs::write(
            dir.join("specs").join("auth.md"),
            "# Auth spec\n\nSession tokens, rotation, and expiry.\n",
        )
        .unwrap();
        std::fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
        let image = image::RgbaImage::from_fn(320, 110, |x, y| {
            let g = (90 + (x % 90)) as u8;
            image::Rgba([g, 140, (80 + (y % 60)) as u8, 255])
        });
        image.save(dir.join("diagram.png")).unwrap();
        let store = Store::open_in(dir.clone()).unwrap();
        let mut note = store.create().unwrap();
        note.body = r#"TYPE: architecture
COLOR: blue

# Notan

A lightweight, **keyboard-driven** Markdown notes app. Offline by design.

![Architecture diagram](diagram.png)

## Writing

##[green] Decisions

- Plain `.md` files on disk
- Autosave after a short idle
- `⌘P` toggles this preview

- [x] headings
- [x] lists
- [ ] more

> [!NOTE]
> The preview updates as you type.
> Quotes render as one continuous block.

### Code

```rust
// greet the reader
fn main() {
    let name = "notan";
    println!("hello, {name}");
}
```

| key | action |
| --- | --- |
| ⌘N | new note |
| ⌘K | commands |

---

Back to writing.
"#
        .to_owned();
        store.save(&mut note, false).unwrap();
        drop(note);

        let ctx = egui::Context::default();
        let app = Notan::from_context(&ctx, Ok(store));
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1120.0, 720.0))
            .with_pixels_per_point(2.0)
            .wgpu()
            .build_state(
                |ctx, app: &mut Notan| {
                    app.frame(ctx);
                },
                app,
            );
        theme::install(&harness.ctx);
        let ctx = harness.ctx.clone();
        harness.state_mut().expanded.insert(dir.join("specs"));
        harness.state_mut().tree_dirty = true;
        if let Some(auth) = harness
            .state()
            .notes
            .iter()
            .find(|note| note.id == "specs/auth")
            .map(|note| note.id.clone())
        {
            harness.state_mut().open_tabs.push(auth);
        }
        harness.state_mut().focus_editor(&ctx);
        harness.run();
        harness
            .render()
            .expect("render")
            .save("/tmp/notan-main.png")
            .expect("save");

        harness.state_mut().open_palette(&ctx);
        harness.run();
        harness
            .render()
            .expect("render")
            .save("/tmp/notan-palette.png")
            .expect("save");

        harness.state_mut().overlay = Overlay::Help;
        harness.run();
        harness
            .render()
            .expect("render")
            .save("/tmp/notan-help.png")
            .expect("save");

        harness.state_mut().overlay = Overlay::Confirm;
        harness.state_mut().pending = Some(Pending::DeleteNote);
        harness.run();
        harness
            .render()
            .expect("render")
            .save("/tmp/notan-confirm.png")
            .expect("save");

        harness.state_mut().inline = Some(InlineEdit {
            kind: InlineKind::NewFolder,
            path: dir.clone(),
            text: "api".to_owned(),
        });
        harness.state_mut().tree_dirty = true;
        harness.run();
        harness
            .render()
            .expect("render")
            .save("/tmp/notan-inline.png")
            .expect("save");

        harness.state_mut().inline = None;
        harness.state_mut().pending = None;
        harness.state_mut().conflict = true;
        harness.state_mut().dirty = true;
        harness.run_steps(3);
        harness
            .render()
            .expect("render")
            .save("/tmp/notan-conflict.png")
            .expect("save");

        // Preview-only mode.
        harness.state_mut().conflict = false;
        harness.state_mut().dirty = false;
        harness.state_mut().preview_only = true;
        harness.run_steps(2);
        harness
            .render()
            .expect("render")
            .save("/tmp/notan-preview-only.png")
            .expect("save");
        harness.state_mut().preview_only = false;

        // Text selection in the editor.
        harness.state_mut().conflict = false;
        harness.state_mut().dirty = false;
        let id = harness.state().editor_id();
        if let Some(mut state) = TextEditState::load(&ctx, id) {
            state.cursor.set_char_range(Some(CCursorRange::two(
                CCursor::new(2),
                CCursor::new(30),
            )));
            state.store(&ctx, id);
        }
        harness.state_mut().focus_editor(&ctx);
        harness.run_steps(2);
        harness
            .render()
            .expect("render")
            .save("/tmp/notan-selection.png")
            .expect("save");

        // Welcome screen: empty store, no selected note.
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).ok();
        let welcome_store = Store::open_in(empty).unwrap();
        let welcome_ctx = egui::Context::default();
        let welcome = Notan::from_context(&welcome_ctx, Ok(welcome_store));
        let mut welcome_harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1120.0, 720.0))
            .with_pixels_per_point(2.0)
            .wgpu()
            .build_state(
                |ctx, app: &mut Notan| {
                    app.frame(ctx);
                },
                welcome,
            );
        theme::install(&welcome_harness.ctx);
        welcome_harness.run_steps(3);
        welcome_harness
            .render()
            .expect("render")
            .save("/tmp/notan-welcome.png")
            .expect("save");

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn native_undo_restores_typing() {
        let dir = std::env::temp_dir().join(format!(
            "notan-undo-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = Store::open_in(dir.clone()).unwrap();
        let ctx = egui::Context::default();
        let app = Notan::from_context(&ctx, Ok(store));
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1120.0, 720.0))
            .build_state(
                |ctx, app: &mut Notan| {
                    app.frame(ctx);
                },
                app,
            );
        theme::install(&harness.ctx);
        let ctx = harness.ctx.clone();
        harness.state_mut().new_note(&ctx);
        let index = harness.state().selected_index().unwrap();
        harness.state_mut().notes[index].body = "abc".to_owned();
        harness.state_mut().focus_editor(&ctx);
        harness.run_steps(3);

        let id = harness.state().editor_id();
        let mut state = TextEditState::load(&harness.ctx, id).expect("state");
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(3))));
        state.store(&harness.ctx, id);

        harness.event(egui::Event::Text("d".into()));
        harness.run_steps(3);
        assert_eq!(harness.state().notes[index].body, "abcd");

        harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        harness.run_steps(3);
        assert_eq!(harness.state().notes[index].body, "abc");

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn tab_inserts_four_spaces() {
        let dir = std::env::temp_dir().join(format!(
            "notan-tab-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = Store::open_in(dir.clone()).unwrap();
        let ctx = egui::Context::default();
        let app = Notan::from_context(&ctx, Ok(store));
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1120.0, 720.0))
            .build_state(
                |ctx, app: &mut Notan| {
                    app.frame(ctx);
                },
                app,
            );
        theme::install(&harness.ctx);
        let ctx = harness.ctx.clone();
        harness.state_mut().new_note(&ctx);
        let index = harness.state().selected_index().unwrap();
        harness.state_mut().notes[index].body = "abc".to_owned();
        harness.state_mut().focus_editor(&ctx);
        harness.run_steps(3);

        let id = harness.state().editor_id();
        let mut state = TextEditState::load(&harness.ctx, id).expect("editor state");
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(3))));
        state.store(&harness.ctx, id);

        harness.key_press(Key::Tab);
        harness.run_steps(3);

        assert_eq!(harness.state().notes[index].body, "abc    ");
        assert!(harness.ctx.memory(|m| m.has_focus(id)));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn builds_workspace_tree() {
        let dir = std::env::temp_dir().join(format!(
            "notan-tree-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("specs")).unwrap();
        std::fs::write(dir.join("specs").join("auth.md"), "# Auth\n").unwrap();
        std::fs::write(dir.join("root.md"), "# Root\n").unwrap();
        let store = Store::open_in(dir.clone()).unwrap();
        let ctx = egui::Context::default();
        let mut app = Notan::from_context(&ctx, Ok(store));

        app.ensure_tree();
        assert!(app.tree_rows.iter().any(|row| row.is_dir && row.name == "specs"));
        assert!(app.tree_rows.iter().any(|row| !row.is_dir && row.name == "root"));

        let specs = dir.join("specs");
        app.expanded.insert(specs.clone());
        app.tree_dirty = true;
        app.ensure_tree();
        let folder = app.tree_rows.iter().position(|row| row.path == specs).unwrap();
        assert_eq!(app.tree_rows[folder].depth, 0);
        assert!(
            app.tree_rows
                .iter()
                .any(|row| row.name == "auth" && row.depth == 1)
        );

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_relative_links() {
        let dir = std::env::temp_dir().join(format!(
            "notan-link-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("specs")).unwrap();
        std::fs::write(dir.join("specs").join("auth.md"), "# Auth\n").unwrap();
        std::fs::write(dir.join("specs").join("index.md"), "# Index\n").unwrap();
        let store = Store::open_in(dir.clone()).unwrap();
        let ctx = egui::Context::default();
        let mut app = Notan::from_context(&ctx, Ok(store));
        app.selected = Some("specs/index".to_owned());

        app.open_link("auth.md", &ctx);
        assert_eq!(app.selected.as_deref(), Some("specs/auth"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn polls_external_changes() {
        let dir = std::env::temp_dir().join(format!(
            "notan-poll-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(dir.join("a.md"), "# A\n").unwrap_or_default();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "# A\n").unwrap();
        let store = Store::open_in(dir.clone()).unwrap();
        let ctx = egui::Context::default();
        let mut app = Notan::from_context(&ctx, Ok(store));
        assert_eq!(app.notes[0].body, "# A\n");

        std::fs::write(dir.join("a.md"), "# A changed\n").unwrap();
        app.last_poll = Instant::now() - Duration::from_secs(3);
        app.poll_external();
        assert_eq!(app.notes[0].body, "# A changed\n");

        std::fs::write(dir.join("b.md"), "# B\n").unwrap();
        app.last_poll = Instant::now() - Duration::from_secs(3);
        app.poll_external();
        assert_eq!(app.notes.len(), 2);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn renames_a_note_inline() {
        let dir = std::env::temp_dir().join(format!(
            "notan-rename-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("specs")).unwrap();
        std::fs::write(dir.join("specs").join("auth.md"), "# Auth\n").unwrap();
        let store = Store::open_in(dir.clone()).unwrap();
        let ctx = egui::Context::default();
        let mut app = Notan::from_context(&ctx, Ok(store));
        app.expanded.insert(dir.join("specs"));
        app.tree_dirty = true;

        app.ensure_tree();
        let index = app
            .tree_rows
            .iter()
            .position(|row| !row.is_dir && row.name == "auth")
            .expect("auth row");
        app.tree_cursor = index;
        app.start_rename_cursor(&ctx);
        if let Some(edit) = &mut app.inline {
            edit.text = "authentication".to_owned();
        }
        app.commit_inline(&ctx);

        assert!(dir.join("specs").join("authentication.md").is_file());
        assert_eq!(app.selected.as_deref(), Some("specs/authentication"));
        assert!(app.inline.is_none());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn clear_notes_moves_everything_to_trash() {
        let dir = std::env::temp_dir().join(format!(
            "notan-clear-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = Store::open_in(dir.clone()).unwrap();
        for name in ["a", "b"] {
            let mut note = store.create().unwrap();
            note.body = format!("# {name}\n");
            store.save(&mut note, false).unwrap();
        }
        let ctx = egui::Context::default();
        let mut app = Notan::from_context(&ctx, Ok(store));
        assert_eq!(app.notes.len(), 2);

        app.clear_notes();
        assert!(app.notes.is_empty());
        assert_eq!(dir.join(".trash").read_dir().unwrap().count(), 2);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn creates_folder_from_keyboard() {
        let dir = std::env::temp_dir().join(format!(
            "notan-folder-key-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = Store::open_in(dir.clone()).unwrap();
        let ctx = egui::Context::default();
        let app = Notan::from_context(&ctx, Ok(store));
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1120.0, 720.0))
            .build_state(
                |ctx, app: &mut Notan| {
                    app.frame(ctx);
                },
                app,
            );
        theme::install(&harness.ctx);
        harness.run_steps(3);

        harness.key_press_modifiers(Modifiers::SHIFT, Key::N);
        harness.run_steps(3);
        assert!(harness.state().inline.is_some());

        harness.event(egui::Event::Text("design specs".into()));
        harness.run_steps(3);
        harness.key_press(Key::Enter);
        harness.run_steps(3);

        assert!(dir.join("design-specs").is_dir(), "folder missing");
        assert!(harness.state().inline.is_none());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn creates_notes_in_the_selected_folder() {
        let dir = std::env::temp_dir().join(format!(
            "notan-create-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("specs")).unwrap();
        std::fs::write(dir.join("specs").join("auth.md"), "# Auth\n").unwrap();
        let store = Store::open_in(dir.clone()).unwrap();
        let ctx = egui::Context::default();
        let mut app = Notan::from_context(&ctx, Ok(store));

        app.ensure_tree();
        let index = app
            .tree_rows
            .iter()
            .position(|row| row.is_dir && row.name == "specs")
            .expect("specs row");
        app.tree_cursor = index;
        app.new_note(&ctx);

        let note = &app.notes[0];
        assert_eq!(note.path.parent(), Some(dir.join("specs").as_path()));
        assert_eq!(note.id, "specs/untitled");

        let specs = dir.join("specs");
        app.start_new_folder(specs.clone(), &ctx);
        if let Some(edit) = &mut app.inline {
            edit.text = "design".to_owned();
        }
        app.commit_inline(&ctx);
        assert!(specs.join("design").is_dir());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn toggles_task_markers() {
        let body = "- [ ] one\n- [x] two\n  * [ ] nested\n";
        assert_eq!(
            toggle_task_in(body, 0).unwrap(),
            "- [x] one\n- [x] two\n  * [ ] nested\n"
        );
        assert_eq!(
            toggle_task_in(body, 1).unwrap(),
            "- [ ] one\n- [ ] two\n  * [ ] nested\n"
        );
        assert_eq!(
            toggle_task_in(body, 2).unwrap(),
            "- [ ] one\n- [x] two\n  * [x] nested\n"
        );
        assert!(toggle_task_in(body, 3).is_none());
        assert_eq!(toggle_task_in("1. [ ] num\n", 0).unwrap(), "1. [x] num\n");
    }

    #[test]
    fn typed_arrow_is_replaced() {
        let dir = std::env::temp_dir().join(format!(
            "notan-arrow-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = Store::open_in(dir.clone()).unwrap();
        let ctx = egui::Context::default();
        let app = Notan::from_context(&ctx, Ok(store));
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1120.0, 720.0))
            .build_state(
                |ctx, app: &mut Notan| {
                    app.frame(ctx);
                },
                app,
            );
        theme::install(&harness.ctx);
        let ctx = harness.ctx.clone();
        harness.state_mut().new_note(&ctx);
        let index = harness.state().selected_index().unwrap();
        harness.state_mut().notes[index].body = "a ".to_owned();
        harness.state_mut().focus_editor(&ctx);
        harness.run_steps(3);

        let id = harness.state().editor_id();
        let mut state = TextEditState::load(&harness.ctx, id).expect("state");
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(2))));
        state.store(&harness.ctx, id);

        harness.event(egui::Event::Text("->".into()));
        harness.run_steps(3);

        assert_eq!(harness.state().notes[index].body, "a →");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn converts_arrows_in_prose_only() {
        assert_eq!(arrow_at("a ->", 4), Some((2, 4, "→")));
        assert_eq!(arrow_at("a <-", 4), Some((2, 4, "←")));
        assert_eq!(arrow_at("a <- b", 4).map(|x| x.2), Some("←"));
        assert_eq!(arrow_at("```rust\nlet v = a ->\n", 21), None);
    }

    #[test]
    fn toggles_inline_markers() {
        let mut chars: Vec<char> = "hello world".chars().collect();
        let (a, b) = toggle_inline(&mut chars, 6, 11, "**");
        assert_eq!(chars.iter().collect::<String>(), "hello **world**");
        assert_eq!((a, b), (8, 13));

        let (a, b) = toggle_inline(&mut chars, 8, 13, "**");
        assert_eq!(chars.iter().collect::<String>(), "hello world");
        assert_eq!((a, b), (6, 11));

        // Selecting text that already includes the markers unwraps it.
        let mut chars: Vec<char> = "a *b* c".chars().collect();
        let (a, b) = toggle_inline(&mut chars, 2, 5, "*");
        assert_eq!(chars.iter().collect::<String>(), "a b c");
        assert_eq!((a, b), (2, 3));
    }

    #[test]
    fn refuses_to_clobber_external_edits() {
        let dir = std::env::temp_dir().join(format!(
            "notan-conflict-ui-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = Store::open_in(dir.clone()).unwrap();
        let ctx = egui::Context::default();
        let mut app = Notan::from_context(&ctx, Ok(store));

        app.new_note(&ctx);
        let index = app.selected_index().unwrap();
        app.notes[index].body = "# Mine\n".to_owned();
        app.dirty = true;
        assert!(app.save_current());
        let path = app.notes[app.selected_index().unwrap()].path.clone();

        std::fs::write(&path, "# Theirs\n").unwrap();

        let index = app.selected_index().unwrap();
        app.notes[index].body = "# Mine again\n".to_owned();
        app.dirty = true;
        assert!(!app.save_current());
        assert!(app.conflict);
        assert!(app.dirty);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Theirs\n");

        app.autosave();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Theirs\n");

        assert!(app.force_save());
        assert!(!app.conflict);
        assert!(!app.dirty);
        let path = app.notes[app.selected_index().unwrap()].path.clone();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Mine again\n");

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn renders_every_surface() {
        let dir = std::env::temp_dir().join(format!(
            "notan-ui-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = Store::open_in(dir.clone()).unwrap();
        let ctx = egui::Context::default();
        let mut app = Notan::from_context(&ctx, Ok(store));

        frame(&ctx, &mut app);

        app.new_note(&ctx);
        assert!(app.selected.is_some());
        let index = app.selected_index().unwrap();
        app.notes[index].body = "# Test note\n\nSome **bold**, *italic* and `code`.\n\n- one\n- [x] two\n\n> [!TIP]\n> quotes stay in one block\n> across several lines\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\ntype Result<T> = std::result::Result<T, Error>;\n```\n".to_owned();
        app.dirty = true;
        frame(&ctx, &mut app);

        app.save_current();
        assert_eq!(app.notes[app.selected_index().unwrap()].id, "test-note");

        app.open_palette(&ctx);
        app.palette_query = "prev".to_owned();
        frame(&ctx, &mut app);

        app.overlay = Overlay::Help;
        frame(&ctx, &mut app);

        app.overlay = Overlay::Confirm;
        app.pending = Some(Pending::DeleteNote);
        frame(&ctx, &mut app);
        app.confirm_pending(&ctx);
        assert!(app.notes.is_empty());
        assert!(app.last_trashed.is_some());
        frame(&ctx, &mut app);

        app.restore_last();
        assert_eq!(app.notes.len(), 1);
        frame(&ctx, &mut app);

        app.preview = false;
        app.new_note(&ctx);
        frame(&ctx, &mut app);

        app.zen = true;
        app.sidebar = false;
        frame(&ctx, &mut app);

        app.zen = false;
        app.sidebar = true;
        app.preview = true;
        app.search.active = true;
        app.search.query = "untitled".to_owned();
        frame(&ctx, &mut app);

        std::fs::remove_dir_all(dir).ok();
    }
}
