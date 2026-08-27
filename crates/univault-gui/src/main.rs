//! egui/eframe front-end for tq-univault.
//!
//! Usage: `univault-gui [--game <TQ install dir>] [--vault <vault.json>] [file]`
//!
//! Left pane: one tab per document — the character's inventory
//! (`Player.chr`) with, discovered automatically beside it, the
//! character's bank (its `winsys.dxb`) and the account's shared and
//! relic banks (`SaveData/Sys/winsys.dxb` and `miscsys.dxb`). Right
//! pane: a vault — the default vault under the config directory
//! opens (and is created) at launch; `Open vault…` swaps in any
//! other vault file, and the vault shows one tab at a time. Click
//! or drag items across; right-click sends an item straight to the
//! other pane's open tab; Shift+Right-click sends a copy; Shift+Click
//! duplicates in place; double-click a completed relic/charm or an
//! artifact to (re)pick its completion bonus — chosen from the list,
//! rolled at the game's odds, or removed. Every edit autosaves after a short quiet
//! period — the first write since a file was loaded goes
//! backup-first, later writes reuse that backup. `Reload` re-reads
//! the character and all banks from disk (confirmed when edits not
//! yet autosaved would be lost).
//! Saves splice only the item region and go through the backup-first
//! write path; stashes also get their `.dxg` twin rewritten.
//! Drag-and-drop routes files by extension.

mod chrome;
mod safe_write;
mod search;
mod theme;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use univault_core::cache::{GameCache, SourceStamp};
use univault_core::chr::{self, EquipSlot, Item, PlayerCharacter, RecordId};
use univault_core::gamedata::GameData;
use univault_core::respec;
use univault_core::stash::{self, Stash};
use univault_core::stats;
use univault_core::style;
use univault_core::transfer;
use univault_core::vault::Vault;

fn main() -> eframe::Result {
    let args = CliArgs::parse();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 900.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "TQ UniVault",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            cc.egui_ctx
                .all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
            Ok(Box::new(App::new(args)))
        }),
    )
}

struct CliArgs {
    game_dir: Option<PathBuf>,
    vault: Option<PathBuf>,
    file: Option<PathBuf>,
}

impl CliArgs {
    fn parse() -> Self {
        Self::from_args(std::env::args_os().skip(1))
    }

    fn from_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> Self {
        let mut game_dir = None;
        let mut vault = None;
        let mut file = None;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg == "--game" {
                game_dir = args.next().map(PathBuf::from);
            } else if arg == "--vault" {
                vault = args.next().map(PathBuf::from);
            } else {
                file = Some(PathBuf::from(arg));
            }
        }
        Self {
            game_dir,
            vault,
            file,
        }
    }
}

struct CharacterPane {
    path: PathBuf,
    original: Vec<u8>,
    character: Box<PlayerCharacter>,
    dirty: bool,
    /// The file's identity when we last read or wrote it; a live
    /// stamp that differs means someone else changed the file.
    disk_stamp: Option<SourceStamp>,
}

struct StashPane {
    path: PathBuf,
    original: Vec<u8>,
    stash: Stash,
    dirty: bool,
    disk_stamp: Option<SourceStamp>,
}

/// Which stash document a path or action addresses: the character's
/// own bank, the account-wide shared bank, or the account-wide relic
/// bank (Atlantis+, `miscsys.dxb`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum StashSlot {
    Bank,
    Shared,
    Relic,
}

impl StashSlot {
    fn label(self) -> &'static str {
        match self {
            Self::Bank => "bank",
            Self::Shared => "shared bank",
            Self::Relic => "relic bank",
        }
    }
}

/// How a stash open concluded.
enum StashOpened {
    Loaded,
    /// The `.dxb` was unreadable; the `.dxg` twin supplied the data
    /// and the repaired image will autosave back over the bad file.
    RecoveredFromTwin,
    /// A dirty pane already holding this path was kept untouched.
    KeptDirty,
}

/// The left pane's tab strip: which document is on screen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LeftTab {
    Inventory,
    Bank,
    Shared,
    Relic,
}

impl LeftTab {
    const ALL: [Self; 4] = [Self::Inventory, Self::Bank, Self::Shared, Self::Relic];

    fn title(self) -> &'static str {
        match self {
            Self::Inventory => "Inventory",
            Self::Bank => "Character bank",
            Self::Shared => "Shared bank",
            Self::Relic => "Relic bank",
        }
    }

    /// Hover hint on a greyed-out tab: why its document is absent.
    fn missing_hint(self) -> &'static str {
        match self {
            Self::Inventory => "No character loaded — open a Player.chr.",
            Self::Bank => {
                "No bank file yet — the game creates winsys.dxb the first time \
                 this character opens the caravan stash."
            }
            Self::Shared => "No shared bank found — expected Sys/winsys.dxb up the save tree.",
            Self::Relic => {
                "No relic bank found — the game (Atlantis and later) keeps it as \
                 Sys/miscsys.dxb next to the shared bank."
            }
        }
    }
}

struct VaultPane {
    path: PathBuf,
    vault: Vault,
    dirty: bool,
    selected: Option<(GridId, usize)>,
    disk_stamp: Option<SourceStamp>,
    /// The one tab on screen — only one is ever open, and it is
    /// where sends, copies, and extractions land.
    open_tab: usize,
}

/// One open document, addressable across panes — the unit the
/// auto-refresh watcher reloads or reports conflicts on.
/// `SearchVault` indexes the search view's loaded vaults, which ride
/// the same autosave/refresh/conflict rails as the fixed panes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DocId {
    Character,
    Stash(StashSlot),
    Vault,
    SearchVault(usize),
}

impl DocId {
    /// The fixed pane documents; search vaults join dynamically via
    /// [`App::all_docs`].
    const FIXED: [Self; 5] = [
        Self::Character,
        Self::Stash(StashSlot::Bank),
        Self::Stash(StashSlot::Shared),
        Self::Stash(StashSlot::Relic),
        Self::Vault,
    ];
}

/// Decides when an externally observed file state is worth acting
/// on: a change must hold steady across two consecutive polls so a
/// file caught mid-write (the game saves over SMB) is never read
/// half-written.
#[derive(Default)]
struct RefreshTracker {
    pending: HashMap<PathBuf, SourceStamp>,
}

impl RefreshTracker {
    /// Feeds one poll observation; `true` means the change settled
    /// and the caller should reload or raise a conflict now.
    fn settled(
        &mut self,
        path: &Path,
        observed: Option<&SourceStamp>,
        ours: Option<&SourceStamp>,
    ) -> bool {
        let Some(observed) = observed else {
            // Unreachable file (mount hiccup, mid-rename): never act,
            // never accumulate.
            self.pending.remove(path);
            return false;
        };
        if Some(observed) == ours {
            self.pending.remove(path);
            return false;
        }
        if self.pending.get(path) == Some(observed) {
            return true;
        }
        self.pending.insert(path.to_path_buf(), observed.clone());
        false
    }

    fn forget(&mut self, path: &Path) {
        self.pending.remove(path);
    }
}

/// Background poller behind auto-refresh: stats the watched files on
/// a fixed cadence and reports the snapshots. Stat calls can hang on
/// an unreachable network mount, so they never run on the UI thread.
struct FileWatcher {
    paths: std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>,
    receiver: std::sync::mpsc::Receiver<Vec<(PathBuf, Option<SourceStamp>)>>,
}

fn start_watcher() -> FileWatcher {
    let paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::<PathBuf>::new()));
    let (sender, receiver) = std::sync::mpsc::channel();
    let shared = std::sync::Arc::clone(&paths);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(WATCH_INTERVAL);
            let watched = match shared.lock() {
                Ok(guard) => guard.clone(),
                Err(_) => return,
            };
            if watched.is_empty() {
                continue;
            }
            let snapshot: Vec<(PathBuf, Option<SourceStamp>)> = watched
                .into_iter()
                .map(|path| {
                    let stamp = stamp_of(&path);
                    (path, stamp)
                })
                .collect();
            if sender.send(snapshot).is_err() {
                return;
            }
        }
    });
    FileWatcher { paths, receiver }
}

enum GameStatus {
    Absent,
    Importing(ImportJob),
    Loaded(GameCache),
    Failed(String),
}

/// A game-data import running on a background thread; the window
/// stays live and shows its progress. The thread finishes with
/// `Done` or `Failed`.
struct ImportJob {
    receiver: std::sync::mpsc::Receiver<ImportEvent>,
    progress: ImportProgress,
}

enum ImportEvent {
    Progress(ImportProgress),
    Done(Box<GameCache>),
    Failed(String),
}

#[derive(Clone)]
struct ImportProgress {
    label: String,
    fraction: Option<f32>,
}

/// Resolved display names, cached per record path — name resolution
/// decompresses database records and must not run per frame.
#[derive(Default)]
struct NameCache {
    names: HashMap<String, String>,
}

impl NameCache {
    fn record_name(&mut self, db: Option<&GameCache>, id: &RecordId) -> String {
        if let Some(cached) = self.names.get(id.as_str()) {
            return cached.clone();
        }
        let resolved = db
            .and_then(|db| db.record_name(id))
            .unwrap_or_else(|| id.file_stem().to_string());
        self.names.insert(id.as_str().to_string(), resolved.clone());
        resolved
    }

    fn item_label(&mut self, db: Option<&GameCache>, item: &Item) -> String {
        let mut parts = Vec::new();
        if let Some(prefix) = &item.prefix {
            parts.push(self.record_name(db, prefix));
        }
        parts.push(self.record_name(db, &item.base));
        if let Some(suffix) = &item.suffix {
            parts.push(self.record_name(db, suffix));
        }
        let mut label = parts.join(" ");
        if item.stack_size > 1 {
            use std::fmt::Write as _;
            let _ = write!(label, " ×{}", item.stack_size);
        }
        label
    }
}

/// Per-record caches for what must never run per frame: record
/// decompression, footprint lookups, texture decodes.
#[derive(Default)]
struct Caches {
    names: NameCache,
    footprints: HashMap<String, (i32, i32)>,
    icons: HashMap<String, Option<egui::TextureHandle>>,
    chrome: ChromeSlot,
}

/// Chrome upload state: tried-and-absent is remembered so a cache
/// without chrome isn't re-asked every frame. Import completion
/// resets `Caches`, which retries.
#[derive(Default)]
enum ChromeSlot {
    #[default]
    Untried,
    Missing,
    Ready(Box<chrome::Chrome>),
}

impl Caches {
    /// The game-art chrome, uploaded once per session. A cheap clone
    /// (texture handles are `Arc`s) so callers keep `Caches`
    /// borrowable.
    fn chrome(&mut self, ctx: &egui::Context, db: Option<&GameCache>) -> Option<chrome::Chrome> {
        if matches!(self.chrome, ChromeSlot::Untried) {
            self.chrome = match db.and_then(|db| chrome::Chrome::load(ctx, db)) {
                Some(loaded) => ChromeSlot::Ready(Box::new(loaded)),
                None => ChromeSlot::Missing,
            };
        }
        match &self.chrome {
            ChromeSlot::Ready(loaded) => Some((**loaded).clone()),
            ChromeSlot::Untried | ChromeSlot::Missing => None,
        }
    }

    fn footprint(&mut self, db: Option<&GameCache>, item: &Item) -> (i32, i32) {
        if let Some(cached) = self.footprints.get(item.base.as_str()) {
            return *cached;
        }
        let footprint = db.map_or(univault_core::gamedata::FALLBACK_FOOTPRINT, |db| {
            db.item_footprint(item)
        });
        self.footprints
            .insert(item.base.as_str().to_string(), footprint);
        footprint
    }

    fn icon(
        &mut self,
        ctx: &egui::Context,
        db: Option<&GameCache>,
        item: &Item,
    ) -> Option<egui::TextureHandle> {
        // Partial relics/charms render their shard art, so the key
        // carries completeness alongside the record.
        let shard = db.is_some_and(|db| db.is_incomplete_relic(item));
        let key = format!("{}#{}", item.base.as_str(), u8::from(shard));
        if let Some(cached) = self.icons.get(&key) {
            return cached.clone();
        }
        let handle = db.and_then(|db| db.item_icon(item)).map(|image| {
            let pixels = egui::ColorImage::from_rgba_unmultiplied(
                [image.width, image.height],
                &image.pixels,
            );
            ctx.load_texture(key.clone(), pixels, egui::TextureOptions::LINEAR)
        });
        self.icons.insert(key, handle.clone());
        handle
    }
}

/// Recently opened game files, persisted one path per line under the
/// platform config directory.
struct Recents {
    file: Option<PathBuf>,
    entries: Vec<PathBuf>,
}

const RECENTS_CAP: usize = 10;

impl Recents {
    fn load() -> Self {
        let file = univault_core::platform::config_dir().map(|dir| dir.join("recent-files.txt"));
        let entries = file
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| {
                text.lines()
                    .map(PathBuf::from)
                    .filter(|path| path.exists())
                    .take(RECENTS_CAP)
                    .collect()
            })
            .unwrap_or_default();
        Self { file, entries }
    }

    fn remember(&mut self, path: &Path) {
        self.entries.retain(|existing| existing != path);
        self.entries.insert(0, path.to_path_buf());
        self.entries.truncate(RECENTS_CAP);
        if let Some(file) = &self.file {
            if let Some(parent) = file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let text = self
                .entries
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let _ = std::fs::write(file, text);
        }
    }

    /// "Pally Don" for `.../_Pally Don/Player.chr`, otherwise
    /// "folder — file".
    fn label(path: &Path) -> String {
        let folder = path
            .parent()
            .and_then(std::path::Path::file_name)
            .map(|name| name.to_string_lossy().trim_start_matches('_').to_string())
            .unwrap_or_default();
        let file_name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().to_string(),
        );
        if file_name.eq_ignore_ascii_case("Player.chr") {
            folder
        } else {
            format!("{folder} — {file_name}")
        }
    }
}

struct App {
    game: GameStatus,
    /// Standing advisory about the game cache (e.g. sources changed
    /// since import), distinct from the last-action status line.
    game_note: Option<String>,
    caches: Caches,
    recents: Recents,
    character: Option<CharacterPane>,
    bank: Option<StashPane>,
    shared: Option<StashPane>,
    relics: Option<StashPane>,
    right: Option<VaultPane>,
    /// The one selected item across all left-side grids (sacks and
    /// the banks); the vault keeps its own so cross-pane moves can
    /// aim at the other pane's last selection.
    left_selected: Option<(GridId, usize)>,
    /// Which left-side document the tab strip is showing — also the
    /// destination of vault → left sends.
    active_tab: LeftTab,
    /// Files already backed up since they were last loaded: the
    /// first autosave of a freshly loaded file takes the backup,
    /// later autosaves reuse it, so rotation isn't churned through
    /// by every edit.
    backed_up: HashSet<PathBuf>,
    /// When the pending autosave fires; pushed forward while the
    /// user is still interacting.
    autosave_at: Option<Instant>,
    /// The last action's outcome for this frame; drained into a
    /// toast at the end of the frame, never laid out in the panes.
    status: Option<Result<String, String>>,
    toasts: Vec<Toast>,
    pending_respec: Option<PendingRespec>,
    pending_bonus: Option<PendingBonus>,
    /// A requested reload awaiting confirmation because unsaved
    /// edits would be lost.
    pending_reload: bool,
    drag: Option<DragState>,
    /// Zoom shown by the slider while dragging; applied on release.
    pending_zoom: f32,
    /// The Game.dll socket-patch dialog, holding the inspected file
    /// while open.
    dll_patch: Option<DllPatchDialog>,
    show_help: bool,
    inventory_tab: InventoryTab,
    watcher: FileWatcher,
    refresh: RefreshTracker,
    /// Documents whose file changed on disk while they hold unsaved
    /// edits — resolved by the conflict modal, never silently.
    conflicts: Vec<DocId>,
    /// Which surface fills the window: the two panes, or the
    /// all-vaults search table.
    view: MainView,
    search: search::SearchState,
}

/// The window's main surface.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MainView {
    Panes,
    Search,
}

/// The socket-patch modal's working state: the dll it read and what
/// the bytes said.
struct DllPatchDialog {
    path: PathBuf,
    outcome: Result<(Vec<u8>, univault_core::dllpatch::PatchState), String>,
}

impl App {
    fn new(args: CliArgs) -> Self {
        // --game forces a (re-)import; otherwise the local cache is
        // the runtime database, imported automatically (in the
        // background) from the remembered game dir when it is
        // missing or in an older format.
        let mut game_note = None;
        let game = if let Some(dir) = args.game_dir.clone() {
            GameStatus::Importing(start_import(dir))
        } else if let Some(cache) = load_cached_game_data() {
            game_note = staleness_warning(&cache);
            GameStatus::Loaded(cache)
        } else if let Some(dir) = stored_game_dir() {
            GameStatus::Importing(start_import(dir))
        } else {
            GameStatus::Absent
        };
        let mut app = Self {
            game,
            game_note,
            caches: Caches::default(),
            recents: Recents::load(),
            character: None,
            bank: None,
            shared: None,
            relics: None,
            right: None,
            left_selected: None,
            active_tab: LeftTab::Inventory,
            backed_up: HashSet::new(),
            autosave_at: None,
            status: None,
            toasts: Vec::new(),
            pending_respec: None,
            pending_bonus: None,
            pending_reload: false,
            drag: None,
            pending_zoom: 1.0,
            dll_patch: None,
            show_help: false,
            inventory_tab: InventoryTab::Equipment,
            watcher: start_watcher(),
            refresh: RefreshTracker::default(),
            conflicts: Vec::new(),
            view: MainView::Panes,
            search: search::SearchState::default(),
        };
        app.status = Some(match args.vault {
            Some(path) => app.open(&path),
            None => app.open_default_vault(),
        });
        if let Some(path) = args.file {
            app.status = Some(app.open(&path));
        }
        app
    }

    /// Routes a path into the matching pane by extension.
    fn open(&mut self, path: &Path) -> Result<String, String> {
        let extension = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
        match extension.as_deref() {
            Some("json") => self.open_vault(path),
            Some("vault") => self.import_legacy_vault(path),
            Some("dxb" | "dxg") => {
                let slot = stash_slot_for(path);
                let opened = self.open_stash(slot, path)?;
                self.recents.remember(path);
                Ok(match opened {
                    StashOpened::Loaded => format!("opened {}", path.display()),
                    StashOpened::RecoveredFromTwin => format!(
                        "opened {} — recovered from its .dxg twin (the .dxb was corrupt); \
                         the repaired file saves automatically",
                        path.display()
                    ),
                    StashOpened::KeptDirty => {
                        format!("{} already open with unsaved edits", path.display())
                    }
                })
            }
            _ => self.open_character_file(path),
        }
    }

    /// Opens a character and discovers its companions: the bank
    /// beside it and the shared and relic banks up the save tree.
    /// Missing or unreadable companions never fail the character
    /// open — they are reported in the status line.
    fn open_character_file(&mut self, path: &Path) -> Result<String, String> {
        let disk_stamp = stamp_of(path);
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let character = Box::new(chr::parse_player(&bytes).map_err(|error| error.to_string())?);
        self.backed_up.remove(path);
        self.refresh.forget(path);
        self.character = Some(CharacterPane {
            path: path.to_path_buf(),
            original: bytes,
            character,
            dirty: false,
            disk_stamp,
        });
        self.left_selected = None;
        self.recents.remember(path);

        let mut notes = Vec::new();
        let bank = univault_core::platform::personal_stash_path(path)
            .filter(|candidate| candidate.is_file());
        self.reload_stash_slot(StashSlot::Bank, bank, &mut notes);
        let shared = univault_core::platform::transfer_stash_candidates(path)
            .find(|candidate| candidate.is_file());
        self.reload_stash_slot(StashSlot::Shared, shared, &mut notes);
        let relics = univault_core::platform::relic_bank_candidates(path)
            .find(|candidate| candidate.is_file());
        self.reload_stash_slot(StashSlot::Relic, relics, &mut notes);
        Ok(format!("opened {} ({})", path.display(), notes.join(", ")))
    }

    /// Points a stash slot at a freshly discovered file, or clears
    /// it when none was found near the new character — but never
    /// discards unsaved edits either way.
    fn reload_stash_slot(
        &mut self,
        slot: StashSlot,
        found: Option<PathBuf>,
        notes: &mut Vec<String>,
    ) {
        let label = slot.label();
        if let Some(path) = found {
            match self.open_stash(slot, &path) {
                Ok(StashOpened::Loaded) => notes.push(format!("{label} loaded")),
                Ok(StashOpened::RecoveredFromTwin) => notes.push(format!(
                    "{label} recovered from its .dxg twin — repaired file saves automatically"
                )),
                Ok(StashOpened::KeptDirty) => {
                    notes.push(format!("{label} kept — unsaved edits"));
                }
                Err(error) => notes.push(format!("{label} unreadable: {error}")),
            }
            return;
        }
        let pane = self.stash_slot_mut(slot);
        if pane.as_ref().is_some_and(|pane| pane.dirty) {
            notes.push(format!("no {label} found — keeping unsaved edits"));
        } else {
            *pane = None;
            notes.push(format!("no {label} found"));
        }
    }

    fn stash_slot_mut(&mut self, slot: StashSlot) -> &mut Option<StashPane> {
        match slot {
            StashSlot::Bank => &mut self.bank,
            StashSlot::Shared => &mut self.shared,
            StashSlot::Relic => &mut self.relics,
        }
    }

    /// Parses a stash file into its slot. A dirty pane already
    /// holding the same path is kept as-is — reopening must not
    /// discard unsaved edits. An unreadable `.dxb` falls back to its
    /// `.dxg` twin — the game's own recovery for a corrupt or
    /// truncated write — and the repaired image saves back
    /// automatically (backup-first, so the bad file is kept).
    fn open_stash(&mut self, slot: StashSlot, path: &Path) -> Result<StashOpened, String> {
        self.backed_up.remove(path);
        self.refresh.forget(path);
        let pane = self.stash_slot_mut(slot);
        if pane
            .as_ref()
            .is_some_and(|pane| pane.path == path && pane.dirty)
        {
            return Ok(StashOpened::KeptDirty);
        }
        let disk_stamp = stamp_of(path);
        let direct = std::fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                let stash = stash::parse_stash(&bytes).map_err(|error| error.to_string())?;
                Ok((bytes, stash))
            });
        let (original, stash, opened) = match direct {
            Ok((bytes, stash)) => (bytes, stash, StashOpened::Loaded),
            Err(error) => {
                let recovered = std::fs::read(path.with_extension("dxg"))
                    .ok()
                    .and_then(|twin| stash::restore_from_twin(&twin).ok())
                    .and_then(|restored| {
                        let stash = stash::parse_stash(&restored).ok()?;
                        Some((restored, stash))
                    });
                let Some((restored, stash)) = recovered else {
                    return Err(error);
                };
                (restored, stash, StashOpened::RecoveredFromTwin)
            }
        };
        let dirty = matches!(opened, StashOpened::RecoveredFromTwin);
        *self.stash_slot_mut(slot) = Some(StashPane {
            path: path.to_path_buf(),
            original,
            stash,
            dirty,
            disk_stamp,
        });
        self.left_selected = None;
        Ok(opened)
    }

    fn open_vault(&mut self, path: &Path) -> Result<String, String> {
        // A vault the search view already holds moves over model and
        // all — unsaved edits included — never via a disk round-trip.
        if let Some(doc) = self.search.docs.iter().position(|doc| doc.path == path) {
            self.adopt_search_doc(doc, 0, None);
            return Ok(format!("opened {}", path.display()));
        }
        self.backed_up.remove(path);
        self.refresh.forget(path);
        let disk_stamp = stamp_of(path);
        // A reload of the already-open file (auto-refresh, Reload)
        // keeps the user's tab; a different vault starts at tab 1.
        let open_tab = self
            .right
            .as_ref()
            .filter(|pane| pane.path == path)
            .map_or(0, |pane| pane.open_tab);
        let vault = if path.exists() {
            let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
            Vault::from_json(&text).map_err(|error| error.to_string())?
        } else {
            Vault::new(12)
        };
        let created = !path.exists();
        self.right = Some(VaultPane {
            path: path.to_path_buf(),
            vault,
            dirty: created,
            selected: None,
            disk_stamp,
            open_tab,
        });
        // The pane's rows in the search table point at the old model.
        self.search.mark_data_changed();
        Ok(if created {
            format!(
                "new vault (12 tabs) — will be created at {}",
                path.display()
            )
        } else {
            format!("opened {}", path.display())
        })
    }

    /// Legacy vaults are import-only: the pane's save path becomes the
    /// `.json` sibling so the binary original is never written.
    fn import_legacy_vault(&mut self, path: &Path) -> Result<String, String> {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let vault = Vault::from_legacy_binary(&bytes).map_err(|error| error.to_string())?;
        let json_path = path.with_extension("json");
        self.right = Some(VaultPane {
            path: json_path.clone(),
            vault,
            dirty: true,
            selected: None,
            disk_stamp: stamp_of(&json_path),
            open_tab: 0,
        });
        self.search.mark_data_changed();
        Ok(format!(
            "imported legacy vault; saving writes {}",
            json_path.display()
        ))
    }

    /// Opens the standing default vault, creating the file on first
    /// launch so a vault exists without any setup. `Open vault…`
    /// still swaps in any other vault file.
    fn open_default_vault(&mut self) -> Result<String, String> {
        let path = default_vault_path().ok_or("no config directory on this platform")?;
        if !path.exists() {
            let empty = Vault::new(12);
            let json = empty.to_json().map_err(|error| error.to_string())?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            std::fs::write(&path, json).map_err(|error| error.to_string())?;
            self.right = Some(VaultPane {
                path: path.clone(),
                vault: empty,
                dirty: false,
                selected: None,
                disk_stamp: stamp_of(&path),
                open_tab: 0,
            });
            return Ok(format!("created default vault at {}", path.display()));
        }
        self.open_vault(&path)
    }

    /// Removes the item at `(grid, index)` from its left-side
    /// document. `Err` when the document is gone or the index stale.
    fn take_from_left(&mut self, grid: GridId, index: usize) -> Result<Item, String> {
        match grid {
            GridId::Sack(sack) => {
                let pane = self.character.as_mut().ok_or("no character loaded")?;
                transfer::take_from_character(&mut pane.character, sack, index)
            }
            GridId::Equipment(slot) => {
                let pane = self.character.as_mut().ok_or("no character loaded")?;
                transfer::take_equipped(&mut pane.character, slot)
            }
            GridId::Bank => {
                let pane = self.bank.as_mut().ok_or("no bank loaded")?;
                transfer::take_from_stash(&mut pane.stash, index)
            }
            GridId::Shared => {
                let pane = self.shared.as_mut().ok_or("no shared bank loaded")?;
                transfer::take_from_stash(&mut pane.stash, index)
            }
            GridId::Relic => {
                let pane = self.relics.as_mut().ok_or("no relic bank loaded")?;
                transfer::take_from_stash(&mut pane.stash, index)
            }
            GridId::VaultTab(_) | GridId::SearchDoc { .. } => None,
        }
        .ok_or_else(|| "selection is stale — pick the item again".to_string())
    }

    /// Auto-places an item back into the left-side document it was
    /// taken from; `false` when even that fails.
    fn restore_to_left(&mut self, grid: GridId, item: Item) -> bool {
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        match grid {
            GridId::Sack(sack) => self.character.as_mut().is_some_and(|pane| {
                transfer::place_in_character(&mut pane.character, item, sack, db).is_ok()
            }),
            GridId::Equipment(slot) => self.character.as_mut().is_some_and(|pane| {
                transfer::equip(&mut pane.character, item, slot)
                    .or_else(|rejected| {
                        transfer::place_in_character(&mut pane.character, *rejected.item, 0, db)
                            .map(|_| ())
                    })
                    .is_ok()
            }),
            GridId::Bank => self
                .bank
                .as_mut()
                .is_some_and(|pane| transfer::place_in_stash(&mut pane.stash, item, db).is_ok()),
            GridId::Shared => self
                .shared
                .as_mut()
                .is_some_and(|pane| transfer::place_in_stash(&mut pane.stash, item, db).is_ok()),
            GridId::Relic => self
                .relics
                .as_mut()
                .is_some_and(|pane| transfer::place_in_stash(&mut pane.stash, item, db).is_ok()),
            GridId::VaultTab(_) | GridId::SearchDoc { .. } => false,
        }
    }

    fn move_left_to_vault(&mut self) -> Result<String, String> {
        let (grid, index) = self.left_selected.ok_or("select an item on the left")?;
        self.send_left_to_vault(grid, index)
    }

    fn send_left_to_vault(&mut self, grid: GridId, index: usize) -> Result<String, String> {
        if self.right.is_none() {
            return Err("load a vault first".to_string());
        }
        let item = self.take_from_left(grid, index)?;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let label = self.caches.names.item_label(db, &item);
        let vault_pane = self.right.as_mut().expect("checked above");
        let tab = vault_pane.open_tab;
        match transfer::place_in_vault_tab(&mut vault_pane.vault, item, tab, db) {
            Ok(()) => {
                vault_pane.dirty = true;
                self.mark_dirty(grid);
                self.left_selected = None;
                Ok(format!("{label} → vault tab {}", tab + 1))
            }
            Err(rejected) => {
                let reason = rejected.reason;
                let restored = self.restore_to_left(grid, *rejected.item);
                Err(if restored {
                    format!("{reason}; item returned to its container")
                } else {
                    format!("{reason}; item could not be returned — reload without saving")
                })
            }
        }
    }

    /// Sends every item in the active left tab to the vault — the
    /// open tab first, spilling into the others as they fill. The
    /// inventory tab covers all sacks; equipped gear stays worn.
    fn bulk_left_to_vault(&mut self, mode: BulkMode) -> Result<String, String> {
        if self.right.is_none() {
            return Err("load a vault first".to_string());
        }
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let vault_pane = self.right.as_mut().expect("checked above");
        let tab = vault_pane.open_tab;
        let vault = &mut vault_pane.vault;
        let (outcome, source_grid, label) = match self.active_tab {
            LeftTab::Inventory => {
                let pane = self.character.as_mut().ok_or("no character loaded")?;
                let outcome = pane
                    .character
                    .sacks
                    .iter_mut()
                    .map(|sack| bulk_into_vault(&mut sack.items, vault, tab, db, mode))
                    .fold(
                        transfer::BulkOutcome::default(),
                        transfer::BulkOutcome::merge,
                    );
                (outcome, GridId::Sack(0), "the inventory")
            }
            LeftTab::Bank => {
                let pane = self.bank.as_mut().ok_or("no bank loaded")?;
                let outcome = bulk_into_vault(&mut pane.stash.items, vault, tab, db, mode);
                (outcome, GridId::Bank, "the bank")
            }
            LeftTab::Shared => {
                let pane = self.shared.as_mut().ok_or("no shared bank loaded")?;
                let outcome = bulk_into_vault(&mut pane.stash.items, vault, tab, db, mode);
                (outcome, GridId::Shared, "the shared bank")
            }
            LeftTab::Relic => {
                let pane = self.relics.as_mut().ok_or("no relic bank loaded")?;
                let outcome = bulk_into_vault(&mut pane.stash.items, vault, tab, db, mode);
                (outcome, GridId::Relic, "the relic bank")
            }
        };
        if outcome.placed > 0 {
            self.mark_dirty(GridId::VaultTab(tab));
            if mode == BulkMode::Move {
                self.mark_dirty(source_grid);
                self.left_selected = None;
            }
        }
        let total = outcome.placed + outcome.left_behind;
        if total == 0 {
            return Err(format!("{label} has no items"));
        }
        let verb = match mode {
            BulkMode::Move => "Moved",
            BulkMode::Copy => "Copied",
        };
        if outcome.placed == 0 {
            return Err(format!(
                "no room in any vault tab — nothing fits from {label}"
            ));
        }
        let spill_note = if outcome.spilled > 0 {
            format!(
                " ({} spilled into other tabs)",
                count_items(outcome.spilled)
            )
        } else {
            String::new()
        };
        let message = if outcome.left_behind > 0 {
            format!(
                "{verb} {} of {} from {label} → vault; {} fit in no tab{spill_note}",
                outcome.placed,
                count_items(total),
                outcome.left_behind
            )
        } else {
            format!(
                "{verb} {} from {label} → vault tab {}{spill_note}",
                count_items(outcome.placed),
                tab + 1
            )
        };
        Ok(message)
    }

    /// The grid the active left tab addresses — where vault → left
    /// sends land. The inventory tab prefers the selected sack.
    /// `None` when the tab's document isn't loaded.
    fn active_tab_grid(&self) -> Option<GridId> {
        match self.active_tab {
            LeftTab::Inventory => self.character.as_ref().map(|_| {
                if let Some((GridId::Sack(sack), _)) = self.left_selected {
                    GridId::Sack(sack)
                } else {
                    GridId::Sack(0)
                }
            }),
            LeftTab::Bank => self.bank.is_some().then_some(GridId::Bank),
            LeftTab::Shared => self.shared.is_some().then_some(GridId::Shared),
            LeftTab::Relic => self.relics.is_some().then_some(GridId::Relic),
        }
    }

    fn move_vault_to_left(&mut self) -> Result<String, String> {
        let selected = self.right.as_ref().and_then(|pane| pane.selected);
        let Some((grid @ GridId::VaultTab(_), index)) = selected else {
            return Err("select an item in the vault".to_string());
        };
        self.send_vault_to_left(grid, index)
    }

    /// Sends the item at a vault-side grid (the open pane or a
    /// search-loaded vault) to the active left tab.
    fn send_vault_to_left(&mut self, grid: GridId, index: usize) -> Result<String, String> {
        let tab = vault_tab_of(grid).ok_or("select an item in the vault")?;
        let destination = self
            .active_tab_grid()
            .ok_or("the active left tab has nothing loaded")?;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let vault_item = {
            let (vault, _) = vault_source_mut(&mut self.right, &mut self.search.docs, grid)
                .ok_or("load a vault first")?;
            transfer::take_from_vault(vault, tab, index)
                .ok_or("selection is stale — pick the item again")?
        };
        let label = self.caches.names.item_label(db, &vault_item.item);
        let placed = match destination {
            GridId::Sack(preferred) => match self.character.as_mut() {
                Some(pane) => transfer::place_in_character(
                    &mut pane.character,
                    vault_item.item,
                    preferred,
                    db,
                )
                .map(|sack| format!("{label} → sack {}", sack + 1)),
                None => Err(transfer::Rejected {
                    item: Box::new(vault_item.item),
                    reason: transfer::TransferError::BadIndex,
                }),
            },
            GridId::Bank => match self.bank.as_mut() {
                Some(pane) => transfer::place_in_stash(&mut pane.stash, vault_item.item, db)
                    .map(|()| format!("{label} → bank")),
                None => Err(transfer::Rejected {
                    item: Box::new(vault_item.item),
                    reason: transfer::TransferError::BadIndex,
                }),
            },
            GridId::Shared => match self.shared.as_mut() {
                Some(pane) => transfer::place_in_stash(&mut pane.stash, vault_item.item, db)
                    .map(|()| format!("{label} → shared bank")),
                None => Err(transfer::Rejected {
                    item: Box::new(vault_item.item),
                    reason: transfer::TransferError::BadIndex,
                }),
            },
            GridId::Relic => match self.relics.as_mut() {
                Some(pane) => transfer::place_in_stash(&mut pane.stash, vault_item.item, db)
                    .map(|()| format!("{label} → relic bank")),
                None => Err(transfer::Rejected {
                    item: Box::new(vault_item.item),
                    reason: transfer::TransferError::BadIndex,
                }),
            },
            GridId::Equipment(_) | GridId::VaultTab(_) | GridId::SearchDoc { .. } => {
                Err(transfer::Rejected {
                    item: Box::new(vault_item.item),
                    reason: transfer::TransferError::BadIndex,
                })
            }
        };
        match placed {
            Ok(message) => {
                self.mark_dirty(destination);
                self.mark_dirty(grid);
                if matches!(grid, GridId::VaultTab(_))
                    && let Some(pane) = self.right.as_mut()
                {
                    pane.selected = None;
                }
                Ok(message)
            }
            Err(rejected) => {
                let reason = rejected.reason;
                let restored = vault_source_mut(&mut self.right, &mut self.search.docs, grid)
                    .is_some_and(|(vault, _)| {
                        transfer::place_in_vault(vault, *rejected.item, tab, db).is_ok()
                    });
                Err(if restored {
                    format!("{reason}; item returned to the vault")
                } else {
                    format!("{reason}; item could not be returned — reload without saving")
                })
            }
        }
    }

    /// The left-side documents holding unsaved edits, by display
    /// name — what a reload would discard.
    fn left_dirty_labels(&self) -> Vec<&'static str> {
        let mut labels = Vec::new();
        if self.character.as_ref().is_some_and(|pane| pane.dirty) {
            labels.push("the character");
        }
        for slot in [StashSlot::Bank, StashSlot::Shared, StashSlot::Relic] {
            let dirty = match slot {
                StashSlot::Bank => &self.bank,
                StashSlot::Shared => &self.shared,
                StashSlot::Relic => &self.relics,
            }
            .as_ref()
            .is_some_and(|pane| pane.dirty);
            if dirty {
                labels.push(slot.label());
            }
        }
        labels
    }

    /// Re-reads the character and every bank from disk, discarding
    /// any in-memory edits. With a character loaded this re-runs
    /// companion discovery from its path; otherwise each open stash
    /// pane reloads from its own path.
    fn reload_left(&mut self) -> Result<String, String> {
        if let Some(pane) = &mut self.character {
            pane.dirty = false;
        }
        for slot in [StashSlot::Bank, StashSlot::Shared, StashSlot::Relic] {
            if let Some(pane) = self.stash_slot_mut(slot).as_mut() {
                pane.dirty = false;
            }
        }
        self.left_selected = None;
        if let Some(path) = self.character.as_ref().map(|pane| pane.path.clone()) {
            return self
                .open_character_file(&path)
                .map(|message| format!("reloaded — {message}"));
        }
        let mut reloaded = Vec::new();
        for slot in [StashSlot::Bank, StashSlot::Shared, StashSlot::Relic] {
            if let Some(path) = self
                .stash_slot_mut(slot)
                .as_ref()
                .map(|pane| pane.path.clone())
            {
                self.open_stash(slot, &path)?;
                reloaded.push(slot.label());
            }
        }
        if reloaded.is_empty() {
            return Err("nothing to reload".to_string());
        }
        Ok(format!("reloaded {}", reloaded.join(", ")))
    }

    fn show_reload_modal(&mut self, ctx: &egui::Context) {
        if !self.pending_reload {
            return;
        }
        let dirty = self.left_dirty_labels();
        let mut close = false;
        let mut confirm = false;
        let modal = egui::Modal::new(egui::Id::new("reload-modal")).show(ctx, |ui| {
            ui.set_max_width(340.0);
            ui.label(theme::heading("Reload from disk?"));
            ui.label(format!(
                "Unsaved changes to {} will be lost.",
                dirty.join(", ")
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                close = ui.button("Cancel").clicked();
                confirm = ui.button("Reload").clicked();
            });
        });
        if confirm {
            self.status = Some(self.reload_left());
        }
        if close || confirm || modal.should_close() {
            self.pending_reload = false;
        }
    }

    /// Right-click: sends a left-side item straight to the vault, or
    /// a vault item back to the left side.
    fn quick_move(&mut self, grid: GridId, index: usize) -> Result<String, String> {
        match grid {
            GridId::Sack(_)
            | GridId::Equipment(_)
            | GridId::Bank
            | GridId::Shared
            | GridId::Relic => self.send_left_to_vault(grid, index),
            GridId::VaultTab(_) | GridId::SearchDoc { .. } => self.send_vault_to_left(grid, index),
        }
    }

    /// The one-time game-data import, as a modal over the UI.
    fn show_import_modal(&self, ctx: &egui::Context) {
        let GameStatus::Importing(job) = &self.game else {
            return;
        };
        egui::Modal::new(egui::Id::new("import-modal")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(theme::heading("Importing game data"));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(&job.progress.label);
            });
            ui.add(egui::ProgressBar::new(job.progress.fraction.unwrap_or(0.0)).show_percentage());
            ui.add_space(4.0);
            ui.weak("One time after installing or updating the game — the cache remembers.");
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    /// The gestures-and-behaviors reference, sectioned — opened from
    /// the toolbar's Help button.
    fn show_help_modal(&mut self, ctx: &egui::Context) {
        if !self.show_help {
            return;
        }
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("help-modal")).show(ctx, |ui| {
            ui.set_max_width(560.0);
            ui.label(theme::heading("How TQ UniVault works"));
            egui::ScrollArea::vertical()
                .max_height(440.0)
                .show(ui, |ui| {
                    help_section(
                        ui,
                        "Opening files",
                        &[
                            "Open (or drop) a Player.chr — its character bank and the \
                         account's shared and relic banks load beside it as tabs.",
                            "A stash (.dxb / .dxg) or another vault (.json, legacy .vault) \
                         opens on its own.",
                            "The default vault opens at launch; the vault pane shows one \
                         tab at a time — pick it in the strip.",
                        ],
                    );
                    help_section(
                        ui,
                        "Moving items",
                        &[
                            "Drag an item between any two grids.",
                            "Right-click sends it to the other pane's open tab; \
                         Shift+Right-click sends a copy.",
                            "Shift+Click duplicates in place.",
                            "\"All → Vault\" and \"Copy all → Vault\" move a whole tab, \
                         spilling into the next vault tabs as each fills.",
                        ],
                    );
                    help_section(
                        ui,
                        "Equipment",
                        &[
                            "The paper doll on the Inventory tab is the worn gear: drag \
                         off it to unequip, drag onto a glowing slot to equip.",
                        ],
                    );
                    help_section(
                        ui,
                        "Relics, charms & artifacts",
                        &[
                            "Drag a relic or charm onto gear its type rules allow to \
                         socket it — any rarity, epics and legendaries included.",
                            "Alt+Click an item with a socketed relic/charm to extract \
                         it — both kept.",
                            "Double-click a completed relic, charm, or artifact to \
                         (re)pick its completion bonus.",
                        ],
                    );
                    help_section(
                        ui,
                        "Search",
                        &[
                            "\"Search vaults…\" (⌘F) opens one filterable table over \
                         every vault file.",
                            "Rows take the same gestures as grid items; double-click \
                         shows an item at home in its vault.",
                        ],
                    );
                    help_section(
                        ui,
                        "Saving",
                        &[
                            "Every edit autosaves after a short quiet pause — there are \
                         no save buttons.",
                            "The first write since a file was loaded stores a backup \
                         beside it; Reload re-reads everything from disk.",
                        ],
                    );
                });
            ui.add_space(8.0);
            close = ui.button("Close").clicked();
        });
        if close || modal.should_close() {
            self.show_help = false;
        }
    }

    /// A clone of the item at `(grid, index)`.
    fn item_at(&self, grid: GridId, index: usize) -> Result<Item, String> {
        let stale = "item changed under the click — try again";
        match grid {
            GridId::Sack(sack) => self
                .character
                .as_ref()
                .ok_or("no character loaded")?
                .character
                .sacks
                .get(sack)
                .and_then(|sack| sack.items.get(index))
                .cloned()
                .ok_or_else(|| stale.to_string()),
            GridId::Equipment(slot) => self
                .character
                .as_ref()
                .ok_or("no character loaded")?
                .character
                .equipment
                .get(slot)
                .cloned()
                .ok_or_else(|| stale.to_string()),
            GridId::Bank => self
                .bank
                .as_ref()
                .ok_or("no bank loaded")?
                .stash
                .items
                .get(index)
                .cloned()
                .ok_or_else(|| stale.to_string()),
            GridId::Shared => self
                .shared
                .as_ref()
                .ok_or("no shared bank loaded")?
                .stash
                .items
                .get(index)
                .cloned()
                .ok_or_else(|| stale.to_string()),
            GridId::Relic => self
                .relics
                .as_ref()
                .ok_or("no relic bank loaded")?
                .stash
                .items
                .get(index)
                .cloned()
                .ok_or_else(|| stale.to_string()),
            GridId::VaultTab(tab) => self
                .right
                .as_ref()
                .ok_or("no vault loaded")?
                .vault
                .sacks
                .get(tab)
                .and_then(|sack| sack.items.get(index))
                .map(|entry| entry.item.clone())
                .ok_or_else(|| stale.to_string()),
            GridId::SearchDoc { doc, tab } => self
                .search
                .docs
                .get(doc)
                .ok_or("vault list changed — rescan")?
                .vault
                .sacks
                .get(tab)
                .and_then(|sack| sack.items.get(index))
                .map(|entry| entry.item.clone())
                .ok_or_else(|| stale.to_string()),
        }
    }

    /// Shift+Click: clones the item at `(grid, index)` — same seed,
    /// so an exact copy — and auto-places it in its own container,
    /// spilling to sibling sacks/tabs when the source grid is full.
    fn duplicate_item(&mut self, grid: GridId, index: usize) -> Result<String, String> {
        let item = self.item_at(grid, index)?;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let label = self.caches.names.item_label(db, &item);
        let placed = match grid {
            GridId::Sack(sack) => {
                let pane = self.character.as_mut().ok_or("no character loaded")?;
                transfer::place_in_character(&mut pane.character, item, sack, db).map(|_| ())
            }
            // A worn item's copy lands in the inventory sacks.
            GridId::Equipment(_) => {
                let pane = self.character.as_mut().ok_or("no character loaded")?;
                transfer::place_in_character(&mut pane.character, item, 0, db).map(|_| ())
            }
            GridId::Bank => {
                let pane = self.bank.as_mut().ok_or("no bank loaded")?;
                transfer::place_in_stash(&mut pane.stash, item, db)
            }
            GridId::Shared => {
                let pane = self.shared.as_mut().ok_or("no shared bank loaded")?;
                transfer::place_in_stash(&mut pane.stash, item, db)
            }
            GridId::Relic => {
                let pane = self.relics.as_mut().ok_or("no relic bank loaded")?;
                transfer::place_in_stash(&mut pane.stash, item, db)
            }
            GridId::VaultTab(tab) | GridId::SearchDoc { tab, .. } => {
                let (vault, _) = vault_source_mut(&mut self.right, &mut self.search.docs, grid)
                    .ok_or("no vault loaded")?;
                transfer::place_in_vault_tab(vault, item, tab, db)
            }
        };
        match placed {
            Ok(()) => {
                self.mark_dirty(grid);
                Ok(format!("duplicated {label}"))
            }
            Err(rejected) => Err(format!("cannot duplicate: {}", rejected.reason)),
        }
    }

    /// Alt+Click: splits the socketed relic/charm out of the item at
    /// `(grid, index)` — the cleaned item stays put and the
    /// standalone piece (shard count and bonus preserved) is
    /// auto-placed in the same container. Nothing is destroyed: the
    /// app-side answer to the Enchanter's destroy-one-half recovery.
    fn extract_socketed(&mut self, grid: GridId, index: usize) -> Result<String, String> {
        let gear = self.item_at(grid, index)?;
        let slot =
            transfer::socketed_slot(&gear).ok_or("no relic or charm socketed in that item")?;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let mut cleaned = gear;
        let piece =
            transfer::extract_relic(db, &mut cleaned, slot).map_err(|error| error.to_string())?;
        let piece_label = self.caches.names.record_name(db, &piece.base);
        let gear_label = self.caches.names.record_name(db, &cleaned.base);
        // Place the piece first; the gear is only committed once the
        // piece has a home, so a full container changes nothing.
        let placed = match grid {
            GridId::Sack(sack) => {
                let pane = self.character.as_mut().ok_or("no character loaded")?;
                transfer::place_in_character(&mut pane.character, piece, sack, db).map(|_| ())
            }
            // A piece pulled out of worn gear lands in the sacks;
            // the gear itself stays equipped.
            GridId::Equipment(_) => {
                let pane = self.character.as_mut().ok_or("no character loaded")?;
                transfer::place_in_character(&mut pane.character, piece, 0, db).map(|_| ())
            }
            GridId::Bank => {
                let pane = self.bank.as_mut().ok_or("no bank loaded")?;
                transfer::place_in_stash(&mut pane.stash, piece, db)
            }
            GridId::Shared => {
                let pane = self.shared.as_mut().ok_or("no shared bank loaded")?;
                transfer::place_in_stash(&mut pane.stash, piece, db)
            }
            GridId::Relic => {
                let pane = self.relics.as_mut().ok_or("no relic bank loaded")?;
                transfer::place_in_stash(&mut pane.stash, piece, db)
            }
            GridId::VaultTab(tab) | GridId::SearchDoc { tab, .. } => {
                let (vault, _) = vault_source_mut(&mut self.right, &mut self.search.docs, grid)
                    .ok_or("no vault loaded")?;
                transfer::place_in_vault_tab(vault, piece, tab, db)
            }
        };
        if let Err(rejected) = placed {
            return Err(format!(
                "cannot extract: {} (the item is unchanged)",
                rejected.reason
            ));
        }
        match self.grid_item_mut(grid, index) {
            Some(slot_item) => *slot_item = cleaned,
            None => return Err("item moved under the click — extracted piece placed".to_string()),
        }
        self.mark_dirty(grid);
        Ok(format!(
            "extracted {piece_label} from {gear_label} — both kept"
        ))
    }

    /// Shift+Right-click: places a copy of the item in the other
    /// pane — the vault for left-side items, the active left tab
    /// for vault items — leaving the original in place.
    fn copy_across(&mut self, grid: GridId, index: usize) -> Result<String, String> {
        let item = self.item_at(grid, index)?;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let label = self.caches.names.item_label(db, &item);
        match grid {
            GridId::Sack(_)
            | GridId::Equipment(_)
            | GridId::Bank
            | GridId::Shared
            | GridId::Relic => {
                let vault_pane = self.right.as_mut().ok_or("load a vault first")?;
                let tab = vault_pane.open_tab;
                match transfer::place_in_vault_tab(&mut vault_pane.vault, item, tab, db) {
                    Ok(()) => {
                        vault_pane.dirty = true;
                        Ok(format!("copy of {label} → vault tab {}", tab + 1))
                    }
                    Err(rejected) => Err(format!("cannot copy: {}", rejected.reason)),
                }
            }
            GridId::VaultTab(_) | GridId::SearchDoc { .. } => {
                let destination = self
                    .active_tab_grid()
                    .ok_or("the active left tab has nothing loaded")?;
                let placed = match destination {
                    GridId::Sack(preferred) => {
                        let pane = self.character.as_mut().ok_or("no character loaded")?;
                        transfer::place_in_character(&mut pane.character, item, preferred, db)
                            .map(|sack| format!("copy of {label} → sack {}", sack + 1))
                    }
                    GridId::Bank => {
                        let pane = self.bank.as_mut().ok_or("no bank loaded")?;
                        transfer::place_in_stash(&mut pane.stash, item, db)
                            .map(|()| format!("copy of {label} → bank"))
                    }
                    GridId::Shared => {
                        let pane = self.shared.as_mut().ok_or("no shared bank loaded")?;
                        transfer::place_in_stash(&mut pane.stash, item, db)
                            .map(|()| format!("copy of {label} → shared bank"))
                    }
                    GridId::Relic => {
                        let pane = self.relics.as_mut().ok_or("no relic bank loaded")?;
                        transfer::place_in_stash(&mut pane.stash, item, db)
                            .map(|()| format!("copy of {label} → relic bank"))
                    }
                    GridId::Equipment(_) | GridId::VaultTab(_) | GridId::SearchDoc { .. } => {
                        return Err("the active left tab has nothing loaded".to_string());
                    }
                };
                match placed {
                    Ok(message) => {
                        self.mark_dirty(destination);
                        Ok(message)
                    }
                    Err(rejected) => Err(format!("cannot copy: {}", rejected.reason)),
                }
            }
        }
    }

    fn save_character(&mut self) -> Result<SaveOutcome, String> {
        let pane = self.character.as_mut().ok_or("nothing to save")?;
        if stamp_of(&pane.path) != pane.disk_stamp {
            return Ok(SaveOutcome::Conflict);
        }
        let spliced = chr::replace_inventory(&pane.original, &pane.character.sacks)
            .map_err(|error| error.to_string())?;
        let spliced = chr::replace_equipment(&spliced, &pane.character.equipment)
            .map_err(|error| error.to_string())?;
        let bytes = chr::replace_money(&spliced, pane.character.info.money)
            .map_err(|error| error.to_string())?;
        write_through(&mut self.backed_up, &pane.path, &bytes)?;
        pane.original = bytes;
        pane.dirty = false;
        pane.disk_stamp = stamp_of(&pane.path);
        Ok(SaveOutcome::Saved)
    }

    fn save_stash(&mut self, slot: StashSlot) -> Result<SaveOutcome, String> {
        let pane = match slot {
            StashSlot::Bank => self.bank.as_mut(),
            StashSlot::Shared => self.shared.as_mut(),
            StashSlot::Relic => self.relics.as_mut(),
        }
        .ok_or("nothing to save")?;
        if stamp_of(&pane.path) != pane.disk_stamp {
            return Ok(SaveOutcome::Conflict);
        }
        let bytes = stash::replace_items(&pane.original, &pane.stash.items)
            .map_err(|error| error.to_string())?;
        write_through(&mut self.backed_up, &pane.path, &bytes)?;
        let twin = stash::backup_twin(&bytes).map_err(|error| error.to_string())?;
        std::fs::write(pane.path.with_extension("dxg"), twin).map_err(|error| error.to_string())?;
        pane.original = bytes;
        pane.dirty = false;
        pane.disk_stamp = stamp_of(&pane.path);
        Ok(SaveOutcome::Saved)
    }

    fn save_vault(&mut self) -> Result<SaveOutcome, String> {
        let pane = self.right.as_mut().ok_or("nothing to save")?;
        if stamp_of(&pane.path) != pane.disk_stamp {
            return Ok(SaveOutcome::Conflict);
        }
        let json = pane.vault.to_json().map_err(|error| error.to_string())?;
        write_through(&mut self.backed_up, &pane.path, json.as_bytes())?;
        pane.dirty = false;
        pane.disk_stamp = stamp_of(&pane.path);
        Ok(SaveOutcome::Saved)
    }

    fn any_dirty(&self) -> bool {
        self.character.as_ref().is_some_and(|pane| pane.dirty)
            || self.bank.as_ref().is_some_and(|pane| pane.dirty)
            || self.shared.as_ref().is_some_and(|pane| pane.dirty)
            || self.relics.as_ref().is_some_and(|pane| pane.dirty)
            || self.right.as_ref().is_some_and(|pane| pane.dirty)
            || self.search.docs.iter().any(|doc| doc.dirty)
    }

    /// Saves everything dirty whose file is still as we left it; a
    /// pane whose file changed underneath is skipped and queued as a
    /// conflict instead — the app never overwrites an external
    /// change without being told to.
    fn flush_dirty(&mut self) -> Result<(), String> {
        if self.character.as_ref().is_some_and(|pane| pane.dirty)
            && self.save_character()? == SaveOutcome::Conflict
        {
            self.push_conflict(DocId::Character);
        }
        for slot in [StashSlot::Bank, StashSlot::Shared, StashSlot::Relic] {
            if self
                .stash_slot_mut(slot)
                .as_ref()
                .is_some_and(|pane| pane.dirty)
                && self.save_stash(slot)? == SaveOutcome::Conflict
            {
                self.push_conflict(DocId::Stash(slot));
            }
        }
        if self.right.as_ref().is_some_and(|pane| pane.dirty)
            && self.save_vault()? == SaveOutcome::Conflict
        {
            self.push_conflict(DocId::Vault);
        }
        for index in 0..self.search.docs.len() {
            if self.search.docs[index].dirty
                && self.save_search_doc(index)? == SaveOutcome::Conflict
            {
                self.push_conflict(DocId::SearchVault(index));
            }
        }
        Ok(())
    }

    fn save_search_doc(&mut self, index: usize) -> Result<SaveOutcome, String> {
        let doc = self.search.docs.get_mut(index).ok_or("nothing to save")?;
        if stamp_of(&doc.path) != doc.disk_stamp {
            return Ok(SaveOutcome::Conflict);
        }
        let json = doc.vault.to_json().map_err(|error| error.to_string())?;
        write_through(&mut self.backed_up, &doc.path, json.as_bytes())?;
        doc.dirty = false;
        doc.disk_stamp = stamp_of(&doc.path);
        Ok(SaveOutcome::Saved)
    }

    fn push_conflict(&mut self, doc: DocId) {
        if !self.conflicts.contains(&doc) {
            self.conflicts.push(doc);
        }
    }

    /// Autosave: once anything is dirty, writes land after a short
    /// quiet period — the deadline is pushed while a drag, a pressed
    /// pointer button, or a focused text field means the edit is
    /// still in motion. A failed flush retries on a slower cadence
    /// and reports through the status line.
    fn drive_autosave(&mut self, ctx: &egui::Context) {
        if !self.conflicts.is_empty() {
            // Conflicted panes stay dirty until the user decides;
            // retrying the flush would only re-detect the conflict.
            self.autosave_at = None;
            return;
        }
        if !self.any_dirty() {
            self.autosave_at = None;
            return;
        }
        let now = Instant::now();
        let busy = self.drag.is_some()
            || ctx.input(|input| input.pointer.any_down())
            || ctx.memory(|memory| memory.focused().is_some());
        let deadline = self.autosave_at.get_or_insert(now + AUTOSAVE_DELAY);
        if busy {
            *deadline = now + AUTOSAVE_DELAY;
        }
        if now < *deadline {
            ctx.request_repaint_after(*deadline - now);
            return;
        }
        self.autosave_at = None;
        if let Err(error) = self.flush_dirty() {
            self.status = Some(Err(format!("autosave failed: {error}")));
            self.autosave_at = Some(Instant::now() + AUTOSAVE_RETRY);
            ctx.request_repaint_after(AUTOSAVE_RETRY);
        }
    }

    /// The open document's path, live disk stamp baseline, and dirty
    /// flag — what auto-refresh compares poll observations against.
    fn doc_state(&self, doc: DocId) -> Option<(PathBuf, Option<SourceStamp>, bool)> {
        match doc {
            DocId::Character => self
                .character
                .as_ref()
                .map(|pane| (pane.path.clone(), pane.disk_stamp.clone(), pane.dirty)),
            DocId::Stash(slot) => match slot {
                StashSlot::Bank => &self.bank,
                StashSlot::Shared => &self.shared,
                StashSlot::Relic => &self.relics,
            }
            .as_ref()
            .map(|pane| (pane.path.clone(), pane.disk_stamp.clone(), pane.dirty)),
            DocId::Vault => self
                .right
                .as_ref()
                .map(|pane| (pane.path.clone(), pane.disk_stamp.clone(), pane.dirty)),
            DocId::SearchVault(index) => self
                .search
                .docs
                .get(index)
                .map(|doc| (doc.path.clone(), doc.disk_stamp.clone(), doc.dirty)),
        }
    }

    /// Every open document: the fixed panes plus the search view's
    /// loaded vaults.
    fn all_docs(&self) -> Vec<DocId> {
        DocId::FIXED
            .iter()
            .copied()
            .chain((0..self.search.docs.len()).map(DocId::SearchVault))
            .collect()
    }

    /// Human name of a document for statuses and the conflict modal.
    fn doc_label(&self, doc: DocId) -> String {
        match doc {
            DocId::Character => "the character".to_string(),
            DocId::Stash(slot) => slot.label().to_string(),
            DocId::Vault => "the vault".to_string(),
            DocId::SearchVault(index) => self.search.docs.get(index).map_or_else(
                || "a searched vault".to_string(),
                |doc| format!("vault '{}'", search::vault_label(&doc.path)),
            ),
        }
    }

    /// Auto-refresh: keeps the watcher pointed at the open documents,
    /// drains its snapshots, and acts once a change has settled —
    /// clean panes reload silently, dirty ones raise the conflict
    /// modal. Never acts mid-interaction.
    fn drive_refresh(&mut self, ctx: &egui::Context) {
        let watched: Vec<PathBuf> = self
            .all_docs()
            .into_iter()
            .filter_map(|doc| self.doc_state(doc).map(|(path, _, _)| path))
            .collect();
        if let Ok(mut guard) = self.watcher.paths.lock() {
            *guard = watched;
        }
        ctx.request_repaint_after(WATCH_INTERVAL);
        let mut latest = None;
        while let Ok(snapshot) = self.watcher.receiver.try_recv() {
            latest = Some(snapshot);
        }
        let Some(snapshot) = latest else { return };
        let busy = self.drag.is_some()
            || ctx.input(|input| input.pointer.any_down())
            || ctx.memory(|memory| memory.focused().is_some());
        if busy {
            return;
        }
        let mut reloaded = Vec::new();
        let mut failed = Vec::new();
        for doc in self.all_docs() {
            let Some((path, ours, dirty)) = self.doc_state(doc) else {
                continue;
            };
            let Some((_, observed)) = snapshot.iter().find(|(seen, _)| *seen == path) else {
                continue;
            };
            if !self
                .refresh
                .settled(&path, observed.as_ref(), ours.as_ref())
            {
                continue;
            }
            if dirty {
                self.push_conflict(doc);
            } else {
                match self.reload_doc(doc) {
                    Ok(()) => reloaded.push(self.doc_label(doc)),
                    Err(error) => failed.push(format!("{}: {error}", self.doc_label(doc))),
                }
            }
        }
        if !failed.is_empty() {
            self.status = Some(Err(format!(
                "changed on disk but could not reload {}",
                failed.join("; ")
            )));
        } else if !reloaded.is_empty() {
            self.status = Some(Ok(format!(
                "auto-reloaded {} — changed on disk",
                reloaded.join(", ")
            )));
        }
    }

    /// Re-reads one document from its own path, dropping in-memory
    /// edits for it (callers decide when that is allowed).
    fn reload_doc(&mut self, doc: DocId) -> Result<(), String> {
        let (path, _, _) = self.doc_state(doc).ok_or("nothing loaded")?;
        match doc {
            DocId::Character => {
                if let Some(pane) = &mut self.character {
                    pane.dirty = false;
                }
                self.open_character_file(&path).map(|_| ())
            }
            DocId::Stash(slot) => {
                if let Some(pane) = self.stash_slot_mut(slot).as_mut() {
                    pane.dirty = false;
                }
                self.open_stash(slot, &path).map(|_| ())
            }
            DocId::Vault => {
                if let Some(pane) = &mut self.right {
                    pane.dirty = false;
                }
                self.open_vault(&path).map(|_| ())
            }
            DocId::SearchVault(index) => {
                let slot = self.search.docs.get_mut(index).ok_or("nothing loaded")?;
                *slot = search::load_search_doc(&path)?;
                self.backed_up.remove(&path);
                self.refresh.forget(&path);
                self.search.mark_data_changed();
                Ok(())
            }
        }
    }

    /// Conflict resolution, disk wins: reload every conflicted
    /// document, discarding the app's unsaved edits for them.
    fn resolve_conflicts_reload(&mut self) {
        let mut done = Vec::new();
        let mut failed = Vec::new();
        for doc in std::mem::take(&mut self.conflicts) {
            match self.reload_doc(doc) {
                Ok(()) => done.push(self.doc_label(doc)),
                Err(error) => failed.push(format!("{}: {error}", self.doc_label(doc))),
            }
        }
        self.status = Some(if failed.is_empty() {
            Ok(format!("reloaded {} from disk", done.join(", ")))
        } else {
            Err(format!("reload failed — {}", failed.join("; ")))
        });
    }

    /// Conflict resolution, app wins: the external version gets its
    /// own fresh backup (re-arming backup-first — those bytes never
    /// existed in this session), then autosave overwrites it.
    fn resolve_conflicts_keep(&mut self) {
        for doc in std::mem::take(&mut self.conflicts) {
            let Some((path, _, _)) = self.doc_state(doc) else {
                continue;
            };
            self.backed_up.remove(&path);
            self.refresh.forget(&path);
            let fresh = stamp_of(&path);
            match doc {
                DocId::Character => {
                    if let Some(pane) = &mut self.character {
                        pane.disk_stamp = fresh;
                    }
                }
                DocId::Stash(slot) => {
                    if let Some(pane) = self.stash_slot_mut(slot).as_mut() {
                        pane.disk_stamp = fresh;
                    }
                }
                DocId::Vault => {
                    if let Some(pane) = &mut self.right {
                        pane.disk_stamp = fresh;
                    }
                }
                DocId::SearchVault(index) => {
                    if let Some(doc) = self.search.docs.get_mut(index) {
                        doc.disk_stamp = fresh;
                    }
                }
            }
        }
        self.autosave_at = Some(Instant::now());
        self.status = Some(Ok(
            "keeping your edits — the external version is backed up before the next save"
                .to_string(),
        ));
    }

    fn show_conflict_modal(&mut self, ctx: &egui::Context) {
        if self.conflicts.is_empty() {
            return;
        }
        let labels: Vec<String> = self
            .conflicts
            .iter()
            .map(|doc| self.doc_label(*doc))
            .collect();
        let mut reload = false;
        let mut keep = false;
        // Deliberately ignores Esc/outside-click: a conflict needs a
        // decision — autosave stays suspended until one is made.
        egui::Modal::new(egui::Id::new("conflict-modal")).show(ctx, |ui| {
            ui.set_max_width(400.0);
            ui.label(theme::heading("Changed on disk"));
            ui.label(format!(
                "The game (or another tool) changed {} on disk while you have \
                 unsaved edits here. Saving is paused until you choose.",
                labels.join(", ")
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                reload = ui.button("Reload from disk (drop my edits)").clicked();
                keep = ui
                    .button("Keep my edits (overwrites; backup kept)")
                    .clicked();
            });
        });
        if reload {
            self.resolve_conflicts_reload();
        } else if keep {
            self.resolve_conflicts_keep();
        }
    }
}

/// One transient outcome notification; errors differ in color and
/// linger longer.
struct Toast {
    text: String,
    error: bool,
    born: Instant,
}

impl Toast {
    fn lifetime(&self) -> Duration {
        if self.error {
            TOAST_ERROR_LIFETIME
        } else {
            TOAST_LIFETIME
        }
    }
}

const TOAST_LIFETIME: Duration = Duration::from_secs(4);
const TOAST_ERROR_LIFETIME: Duration = Duration::from_secs(8);
/// Older toasts beyond this many are dropped immediately.
const TOAST_STACK: usize = 6;

/// Quiet period between the last edit and its autosave.
const AUTOSAVE_DELAY: Duration = Duration::from_millis(600);
/// Backoff before an autosave that failed (e.g. an unreachable
/// volume) is retried.
const AUTOSAVE_RETRY: Duration = Duration::from_secs(5);
/// Auto-refresh poll cadence; a change must also hold across two
/// consecutive polls before the app acts on it.
const WATCH_INTERVAL: Duration = Duration::from_secs(2);

/// How one save attempt concluded: written, or skipped because the
/// file changed externally since our baseline.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SaveOutcome {
    Saved,
    Conflict,
}

/// The write path behind autosave: the first write since `path` was
/// last loaded goes backup-first; later writes reuse that backup so
/// per-edit saves don't churn the rotation.
fn write_through(
    backed_up: &mut HashSet<PathBuf>,
    path: &Path,
    bytes: &[u8],
) -> Result<(), String> {
    if backed_up.contains(path) {
        safe_write::write_synced(path, bytes).map_err(|error| error.to_string())
    } else {
        safe_write::backup_first_write(path, bytes).map_err(|error| error.to_string())?;
        backed_up.insert(path.to_path_buf());
        Ok(())
    }
}

/// A `miscsys` stash is the relic bank wherever it sits; a stash
/// under a `Sys` folder is the account-wide transfer stash; anywhere
/// else (a character folder) it is that character's bank.
fn stash_slot_for(path: &Path) -> StashSlot {
    let is_miscsys = path
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("miscsys"));
    if is_miscsys {
        return StashSlot::Relic;
    }
    let in_sys_folder = path
        .parent()
        .and_then(std::path::Path::file_name)
        .is_some_and(|name| name.eq_ignore_ascii_case("Sys"));
    if in_sys_folder {
        StashSlot::Shared
    } else {
        StashSlot::Bank
    }
}

fn cache_file_path() -> Option<PathBuf> {
    univault_core::platform::config_dir().map(|dir| dir.join("gamedata.cache"))
}

fn vaults_dir() -> Option<PathBuf> {
    univault_core::platform::config_dir().map(|dir| dir.join("vaults"))
}

fn default_vault_path() -> Option<PathBuf> {
    vaults_dir().map(|dir| dir.join("Main Vault.json"))
}

fn game_dir_file_path() -> Option<PathBuf> {
    univault_core::platform::config_dir().map(|dir| dir.join("game-dir.txt"))
}

fn stored_game_dir() -> Option<PathBuf> {
    let path = game_dir_file_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let dir = PathBuf::from(text.trim());
    dir.is_dir().then_some(dir)
}

fn load_cached_game_data() -> Option<GameCache> {
    let bytes = std::fs::read(cache_file_path()?).ok()?;
    GameCache::from_bytes(&bytes).ok()
}

fn read_stamped(path: &Path) -> Result<(Vec<u8>, SourceStamp), String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok((
        bytes,
        stamp_of(path).unwrap_or(SourceStamp {
            path: path.display().to_string(),
            size: 0,
            mtime_seconds: 0,
        }),
    ))
}

fn stamp_of(path: &Path) -> Option<SourceStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    let mtime_seconds = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(0);
    Some(SourceStamp {
        path: path.display().to_string(),
        size: i64::try_from(metadata.len()).unwrap_or(0),
        mtime_seconds,
    })
}

/// Kicks off the one-time import on a background thread: the game
/// archives are read and distilled into the cache while the window
/// stays responsive and shows progress.
fn start_import(dir: PathBuf) -> ImportJob {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = match run_import(&dir, &sender) {
            Ok(cache) => ImportEvent::Done(Box::new(cache)),
            Err(message) => ImportEvent::Failed(message),
        };
        let _ = sender.send(outcome);
    });
    ImportJob {
        receiver,
        progress: ImportProgress {
            label: "Preparing game-data import…".to_string(),
            fraction: None,
        },
    }
}

/// The import itself: reads the game archives, distills the item
/// cache, and persists it (plus the game dir for later refreshes)
/// under the config directory, reporting each phase as it goes.
fn run_import(
    dir: &Path,
    sender: &std::sync::mpsc::Sender<ImportEvent>,
) -> Result<GameCache, String> {
    let report = |label: String, fraction: Option<f32>| {
        let _ = sender.send(ImportEvent::Progress(ImportProgress { label, fraction }));
    };
    report("Reading game database (database.arz)…".to_string(), None);
    let (database, database_stamp) = read_stamped(&dir.join("Database/database.arz"))?;
    report("Reading text archive (Text_EN.arc)…".to_string(), None);
    let (text, text_stamp) = read_stamped(&dir.join("Text/Text_EN.arc"))?;
    let mut stamps = vec![database_stamp, text_stamp];
    report("Parsing game database…".to_string(), None);
    let mut data = GameData::from_bytes(database, text).map_err(|error| error.to_string())?;
    let candidates = [
        ("", "Resources/Items.arc"),
        ("XPACK", "Resources/XPack/Items.arc"),
        ("XPACK2", "Resources/XPack2/Items.arc"),
        ("XPACK3", "Resources/XPack3/Items.arc"),
        ("XPACK4", "Resources/XPack4/Items.arc"),
    ];
    for (label, relative) in candidates {
        report(format!("Reading item bitmaps ({relative})…"), None);
        let path = dir.join(relative);
        if let Ok((bytes, stamp)) = read_stamped(&path)
            && let Ok(archive) = univault_core::arc::ArcFile::parse(bytes)
        {
            data.add_items_archive(label, archive);
            stamps.push(stamp);
        }
    }
    // Immortal Throne's UI.arc first: it carries the caravan art the
    // chrome manifest centers on.
    for relative in ["Resources/XPack/UI.arc", "Resources/InGameUI.arc"] {
        report(format!("Reading UI chrome ({relative})…"), None);
        if let Ok((bytes, stamp)) = read_stamped(&dir.join(relative))
            && let Ok(archive) = univault_core::arc::ArcFile::parse(bytes)
        {
            data.add_ui_archive(archive);
            stamps.push(stamp);
        }
    }
    let cache = data.build_cache_with_progress(stamps, |scanned, total| {
        report(
            format!("Distilling item records… {scanned} / {total}"),
            Some(fraction(scanned, total)),
        );
    });
    report("Writing the local cache…".to_string(), None);
    if let Some(path) = cache_file_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&path, cache.to_bytes())
            .map_err(|error| format!("writing cache: {error}"))?;
    }
    if let Some(path) = game_dir_file_path() {
        let _ = std::fs::write(path, dir.display().to_string());
    }
    Ok(cache)
}

// Record counts sit far below f32's exact-integer range.
#[allow(clippy::cast_precision_loss)]
fn fraction(done: usize, total: usize) -> f32 {
    if total == 0 {
        0.0
    } else {
        done as f32 / total as f32
    }
}

/// `Some(warning)` when any imported source file has changed on disk
/// since the cache was built. Unreachable sources are ignored (the
/// game volume may simply not be mounted).
fn staleness_warning(cache: &GameCache) -> Option<String> {
    let changed = cache.stamps().iter().any(|recorded| {
        stamp_of(Path::new(&recorded.path)).is_some_and(|current| current != *recorded)
    });
    changed.then(|| {
        "Game files changed since the last import — use 'Import game data…' to refresh.".to_string()
    })
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::SURFACE.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        if let Some(chrome) = self.caches.chrome(ui.ctx(), db) {
            chrome.backdrop(ui.painter(), ui.ctx().content_rect(), STONE_TINT);
        }
        egui::Frame::NONE
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| self.main_ui(ui));
    }
}

/// The stone backdrop is the character screen's parchment, darkened
/// so cream text outside the panes stays readable.
const STONE_TINT: egui::Color32 = egui::Color32::from_gray(150);

impl App {
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        self.poll_import();
        if let Some(dropped) = first_dropped_path(ui.ctx()) {
            self.status = Some(self.open(&dropped));
        }
        match self.view {
            MainView::Panes => {
                if matches!(self.game, GameStatus::Loaded(_))
                    && ui.ctx().input_mut(|input| {
                        input.consume_key(egui::Modifiers::COMMAND, egui::Key::F)
                    })
                {
                    self.enter_search();
                }
                self.show_header(ui);
                ui.separator();
                let (action, drag_frame) = self.show_panes(ui);
                if let Some((grid, index)) = drag_frame.duplicate {
                    self.status = Some(self.duplicate_item(grid, index));
                }
                if let Some((grid, index)) = drag_frame.quick_move {
                    self.status = Some(self.quick_move(grid, index));
                }
                if let Some((grid, index)) = drag_frame.copy_across {
                    self.status = Some(self.copy_across(grid, index));
                }
                if let Some((grid, index)) = drag_frame.edit_bonus {
                    self.request_bonus_edit(grid, index);
                }
                if let Some((grid, index)) = drag_frame.extract {
                    self.status = Some(self.extract_socketed(grid, index));
                }
                self.update_drag(ui.ctx(), drag_frame);
                match action {
                    Some(PaneAction::MoveToVault) => self.status = Some(self.move_left_to_vault()),
                    Some(PaneAction::MoveAllToVault) => {
                        self.status = Some(self.bulk_left_to_vault(BulkMode::Move));
                    }
                    Some(PaneAction::CopyAllToVault) => {
                        self.status = Some(self.bulk_left_to_vault(BulkMode::Copy));
                    }
                    Some(PaneAction::MoveToFile) => self.status = Some(self.move_vault_to_left()),
                    Some(PaneAction::OpenSearch) => self.enter_search(),
                    Some(PaneAction::PreviewRespec(kind)) => self.preview_respec(kind),
                    None => {}
                }
            }
            MainView::Search => {
                let search_frame = self.show_search_ui(ui);
                if let Some((grid, index)) = search_frame.duplicate {
                    self.status = Some(self.duplicate_item(grid, index));
                }
                if let Some((grid, index)) = search_frame.quick_move {
                    self.status = Some(self.quick_move(grid, index));
                }
                if let Some((grid, index)) = search_frame.copy_across {
                    self.status = Some(self.copy_across(grid, index));
                }
                if let Some((grid, index)) = search_frame.extract {
                    self.status = Some(self.extract_socketed(grid, index));
                }
                if let Some((grid, index)) = search_frame.jump {
                    self.jump_to_search_row(grid, index);
                }
                if search_frame.leave {
                    self.view = MainView::Panes;
                }
            }
        }
        self.drive_refresh(ui.ctx());
        self.drive_autosave(ui.ctx());
        self.show_respec_modal(ui.ctx());
        self.show_reload_modal(ui.ctx());
        self.show_bonus_modal(ui.ctx());
        self.show_dll_patch_modal(ui.ctx());
        self.show_help_modal(ui.ctx());
        self.show_import_modal(ui.ctx());
        self.show_conflict_modal(ui.ctx());
        self.show_toasts(ui.ctx());
    }
}

/// The Inventory view's exclusive sub-tab: the doll, or one sack.
#[derive(Clone, Copy, PartialEq, Eq)]
enum InventoryTab {
    Equipment,
    Sack(usize),
}

enum PaneAction {
    MoveToVault,
    MoveAllToVault,
    CopyAllToVault,
    MoveToFile,
    PreviewRespec(RespecKind),
    OpenSearch,
}

/// Whether a bulk send drains the source or leaves it untouched.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BulkMode {
    Move,
    Copy,
}

fn bulk_into_vault(
    items: &mut Vec<Item>,
    vault: &mut Vault,
    tab: usize,
    db: Option<&GameCache>,
    mode: BulkMode,
) -> transfer::BulkOutcome {
    match mode {
        BulkMode::Move => transfer::move_all_into_vault(items, vault, tab, db),
        BulkMode::Copy => transfer::copy_all_into_vault(items, vault, tab, db),
    }
}

fn count_items(count: usize) -> String {
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} item{plural}")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RespecKind {
    Attributes,
    Skills,
}

/// A respec awaiting the user's confirmation, with its previewed
/// refund.
struct PendingRespec {
    kind: RespecKind,
    points: i32,
    skills_removed: usize,
}

/// A just-completed relic/charm awaiting its completion-bonus pick.
struct PendingBonus {
    grid: GridId,
    index: usize,
    base: RecordId,
    name: String,
    options: Vec<BonusOption>,
    total_weight: i64,
    /// The bonus already on the piece, when re-picking.
    current: Option<RecordId>,
}

/// One completion bonus the finished piece can take.
struct BonusOption {
    record: RecordId,
    weight: i32,
    name: String,
    lines: Vec<stats::StatLine>,
}

impl App {
    /// Applies whatever a running background import has produced:
    /// progress for the header, or its final cache/failure.
    fn poll_import(&mut self) {
        use std::sync::mpsc::TryRecvError;
        let GameStatus::Importing(job) = &mut self.game else {
            return;
        };
        let mut outcome = None;
        loop {
            match job.receiver.try_recv() {
                Ok(ImportEvent::Progress(progress)) => job.progress = progress,
                Ok(ImportEvent::Done(cache)) => {
                    outcome = Some((
                        GameStatus::Loaded(*cache),
                        Ok("game data imported".to_string()),
                    ));
                    break;
                }
                Ok(ImportEvent::Failed(message)) => {
                    outcome = Some((GameStatus::Failed(message.clone()), Err(message)));
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let message = "game-data import stopped unexpectedly".to_string();
                    outcome = Some((GameStatus::Failed(message.clone()), Err(message)));
                    break;
                }
            }
        }
        if let Some((game, status)) = outcome {
            let count = match &game {
                GameStatus::Loaded(cache) => Some(cache.len()),
                GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
            };
            self.game = game;
            self.game_note = None;
            self.caches = Caches::default();
            self.status = Some(match (status, count) {
                (Ok(_), Some(count)) => Ok(format!("imported {count} item records")),
                (status, _) => status,
            });
        }
    }

    /// The Open/Reload/Import/Recent button row. Returns the path
    /// the user asked to open (if any) and whether a reload was
    /// requested.
    fn show_toolbar(&mut self, ui: &mut egui::Ui) -> (Option<PathBuf>, bool) {
        let mut requested: Option<PathBuf> = None;
        let mut reload_requested = false;
        let has_left = self.character.is_some()
            || self.bank.is_some()
            || self.shared.is_some()
            || self.relics.is_some();
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let pane_chrome = self.caches.chrome(ui.ctx(), db);
        let pane_chrome = pane_chrome.as_ref();
        ui.horizontal_wrapped(|ui| {
            if plate_button(ui, pane_chrome, true, "Open character…").clicked() {
                requested = pick_file(
                    "Character / stash",
                    &["chr", "dxb", "dxg"],
                    self.dialog_start_dir(),
                );
            }
            if plate_button(ui, pane_chrome, has_left, "Reload")
                .on_hover_text("Re-read the character and all banks from disk")
                .clicked()
            {
                reload_requested = true;
            }
            if plate_button(ui, pane_chrome, true, "Open vault…").clicked() {
                requested = pick_file("Vault", &["json", "vault"], self.dialog_start_dir());
            }
            if plate_button(
                ui,
                pane_chrome,
                matches!(self.game, GameStatus::Loaded(_)),
                "Search vaults…",
            )
            .on_hover_text(
                "One filterable table of every item in every vault (⌘F / Ctrl+F). \
                 Needs imported game data.",
            )
            .clicked()
            {
                self.enter_search();
            }
            let importing = matches!(self.game, GameStatus::Importing(_));
            if plate_button(ui, pane_chrome, !importing, "Import game data…").clicked() {
                let start = stored_game_dir();
                let mut dialog = rfd::FileDialog::new();
                if let Some(start) = start {
                    dialog = dialog.set_directory(start);
                }
                if let Some(dir) = dialog.pick_folder() {
                    self.game = GameStatus::Importing(start_import(dir));
                    self.game_note = None;
                }
            }
            let recent = plate_button(ui, pane_chrome, true, "Recent");
            egui::Popup::menu(&recent).show(|ui| {
                if self.recents.entries.is_empty() {
                    ui.weak("nothing yet");
                }
                for path in &self.recents.entries {
                    if ui.button(Recents::label(path)).clicked() {
                        requested = Some(path.clone());
                    }
                }
            });
            if stored_game_dir().is_some()
                && plate_button(ui, pane_chrome, true, "Socket patch…")
                    .on_hover_text(
                        "Toggle the Game.dll socket-gate patch: lets the game itself \
                         accept relics/charms on Epic and Legendary items",
                    )
                    .clicked()
            {
                self.open_dll_patch_dialog();
            }
            if plate_button(ui, pane_chrome, true, "Help…").clicked() {
                self.show_help = true;
            }
        });
        (requested, reload_requested)
    }

    fn show_header(&mut self, ui: &mut egui::Ui) {
        let (requested, reload_requested) = self.show_toolbar(ui);
        if let Some(path) = requested {
            self.status = Some(self.open(&path));
        }
        if reload_requested {
            if self.left_dirty_labels().is_empty() {
                self.status = Some(self.reload_left());
            } else {
                self.pending_reload = true;
            }
        }
        self.show_zoom_control(ui);
        if let Some(note) = &self.game_note {
            ui.colored_label(ui.visuals().warn_fg_color, note);
        }
        match &self.game {
            GameStatus::Loaded(_) | GameStatus::Importing(_) => {}
            GameStatus::Absent => {
                ui.weak(
                    "No game data imported yet — use 'Import game data…' and pick your \
                     Titan Quest install (one time; names, icons and sizes come from it).",
                );
            }
            GameStatus::Failed(message) => {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("Game data failed to load: {message}"),
                );
            }
        }
        if self.character.is_none()
            && self.bank.is_none()
            && self.shared.is_none()
            && self.relics.is_none()
        {
            ui.add_space(12.0);
            ui.label(theme::heading("TQ UniVault"));
            ui.label(
                "Open (or drop) a Player.chr to begin — its banks load beside it \
                 as tabs, and the default vault is already open on the right.",
            );
            ui.label("Every gesture and behavior is described under Help…");
        }
    }

    /// Outcome messages as toasts: bottom-right overlay, oldest on
    /// top, auto-expiring, and — deliberately — not interactable, so
    /// the panes underneath never reflow and never lose the pointer
    /// to a message.
    fn show_toasts(&mut self, ctx: &egui::Context) {
        if let Some(outcome) = self.status.take() {
            let (text, error) = match outcome {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            };
            self.toasts.push(Toast {
                text,
                error,
                born: Instant::now(),
            });
            if self.toasts.len() > TOAST_STACK {
                let excess = self.toasts.len() - TOAST_STACK;
                self.toasts.drain(..excess);
            }
        }
        let now = Instant::now();
        self.toasts
            .retain(|toast| now.duration_since(toast.born) < toast.lifetime());
        let Some(soonest) = self
            .toasts
            .iter()
            .map(|toast| {
                toast
                    .lifetime()
                    .saturating_sub(now.duration_since(toast.born))
            })
            .min()
        else {
            return;
        };
        egui::Area::new(egui::Id::new("status-toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                for toast in &self.toasts {
                    let color = if toast.error {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().strong_text_color()
                    };
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(360.0);
                        ui.colored_label(color, &toast.text);
                    });
                }
            });
        // Expiry must repaint even while the app is otherwise idle.
        ctx.request_repaint_after(soonest);
    }

    /// Applying zoom mid-drag rescales the slider out from under the
    /// pointer, so the slider tracks a pending value while dragging
    /// and the zoom only takes effect on release.
    fn show_zoom_control(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Zoom:");
            let mut value = self.pending_zoom;
            let response = ui.add(
                egui::Slider::new(&mut value, 0.75..=2.5)
                    .step_by(0.05)
                    .custom_formatter(|zoom, _| format!("{:.0}%", zoom * 100.0)),
            );
            if response.dragged() {
                self.pending_zoom = value;
            } else if response.drag_stopped() {
                self.pending_zoom = value;
                ui.ctx().set_zoom_factor(value);
            } else {
                // Stay in step with the ⌘+/⌘−/⌘0 shortcuts.
                self.pending_zoom = ui.ctx().zoom_factor();
            }
            ui.weak("(⌘+ / ⌘− / ⌘0 work too)");
            if self.any_dirty() {
                ui.weak("Saving…");
            }
        });
    }

    /// Advances the drag: adopts a newly started one, paints the item
    /// at the pointer, and commits or cancels on release.
    fn update_drag(&mut self, ctx: &egui::Context, frame: DragFrame) {
        if self.drag.is_none() {
            self.drag = frame.begin;
        }
        let Some(state) = self.drag.clone() else {
            return;
        };

        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let footprint = self.caches.footprint(db, &state.item);
        if let Some(pointer) = ctx.pointer_latest_pos() {
            let rect = egui::Rect::from_min_size(
                pointer - state.grab,
                egui::vec2(cells_to_points(footprint.0), cells_to_points(footprint.1)),
            );
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("drag-cursor"),
            ));
            if let Some(texture) = self.caches.icon(ctx, db, &state.item) {
                painter.image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200),
                );
            } else {
                painter.rect_filled(rect, 2.0, egui::Color32::from_black_alpha(120));
            }
        }

        if ctx.input(|input| input.pointer.any_released()) {
            if let Some(candidate) = frame.candidate {
                if let Some(target_index) = candidate.combine_with {
                    self.status = Some(self.perform_combine(&state, candidate.grid, target_index));
                } else if let Some(target_index) = candidate.socket_into {
                    self.status = Some(self.perform_socket(&state, candidate.grid, target_index));
                } else if candidate.equips {
                    self.status = Some(self.perform_equip(&state, candidate.grid));
                } else if candidate.fits {
                    let same_spot =
                        candidate.grid == state.source && candidate.cell == state.item.position;
                    if !same_spot {
                        self.status = Some(self.perform_drop(&state, candidate));
                    }
                }
            }
            self.drag = None;
            ctx.request_repaint();
        }
    }

    /// Removes the item at `(grid, index)` from any container.
    fn take_at(&mut self, grid: GridId, index: usize) -> Result<Item, String> {
        match grid {
            GridId::Sack(_)
            | GridId::Equipment(_)
            | GridId::Bank
            | GridId::Shared
            | GridId::Relic => self.take_from_left(grid, index),
            GridId::VaultTab(tab) | GridId::SearchDoc { tab, .. } => {
                let (vault, _) = vault_source_mut(&mut self.right, &mut self.search.docs, grid)
                    .ok_or("no vault loaded")?;
                transfer::take_from_vault(vault, tab, index)
                    .map(|entry| entry.item)
                    .ok_or_else(|| "item moved under the drag — drop ignored".to_string())
            }
        }
    }

    /// The item at `(grid, index)`, mutable in place.
    fn grid_item_mut(&mut self, grid: GridId, index: usize) -> Option<&mut Item> {
        match grid {
            GridId::Sack(sack) => self
                .character
                .as_mut()?
                .character
                .sacks
                .get_mut(sack)?
                .items
                .get_mut(index),
            GridId::Equipment(slot) => self
                .character
                .as_mut()?
                .character
                .equipment
                .slot_mut(slot)
                .as_mut(),
            GridId::Bank => self.bank.as_mut()?.stash.items.get_mut(index),
            GridId::Shared => self.shared.as_mut()?.stash.items.get_mut(index),
            GridId::Relic => self.relics.as_mut()?.stash.items.get_mut(index),
            GridId::VaultTab(tab) | GridId::SearchDoc { tab, .. } => {
                let (vault, _) = vault_source_mut(&mut self.right, &mut self.search.docs, grid)?;
                vault
                    .sacks
                    .get_mut(tab)?
                    .items
                    .get_mut(index)
                    .map(|entry| &mut entry.item)
            }
        }
    }

    /// Drops a partial relic/charm onto a matching partial: shards
    /// pour into the target up to completion, the remainder stays in
    /// the source, and a completed piece opens the bonus picker.
    fn perform_combine(
        &mut self,
        state: &DragState,
        grid: GridId,
        target_index: usize,
    ) -> Result<String, String> {
        let needed = match &self.game {
            GameStatus::Loaded(db) => db.completed_relic_level(&state.item.base),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        }
        .ok_or("no completion data for this item")?;

        let mut source = self.take_at(state.source, state.index)?;
        if source.base != state.item.base {
            let origin = state.item.position;
            self.restore_dropped(state.source, source, origin)?;
            return Err("item moved under the drag — drop ignored".to_string());
        }
        let origin = source.position;
        let target_index = if state.source == grid && state.index < target_index {
            target_index - 1
        } else {
            target_index
        };
        let outcome = match self.grid_item_mut(grid, target_index) {
            Some(target) if target.base == source.base => {
                transfer::combine_shards(target, &mut source, needed)
            }
            Some(_) | None => {
                self.restore_dropped(state.source, source, origin)?;
                return Err("combine target moved — drop ignored".to_string());
            }
        };
        if !outcome.source_emptied {
            self.restore_dropped(state.source, source, origin)?;
        }
        self.mark_dirty(state.source);
        self.mark_dirty(grid);
        self.left_selected = None;
        if let Some(pane) = &mut self.right {
            pane.selected = None;
        }
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let label = self.caches.names.record_name(db, &state.item.base);
        if outcome.target_completed {
            self.begin_bonus_pick(grid, target_index, state.item.base.clone(), None);
            Ok(format!(
                "{label} completed ({needed}/{needed}) — choose its bonus"
            ))
        } else {
            Ok(format!(
                "poured {} shard(s) into {label}",
                outcome.transferred
            ))
        }
    }

    /// Drops a standalone relic/charm onto allowed gear: the piece
    /// moves into the item's socket — record, shard count, and bonus
    /// — honoring the game's type rules but not its rarity gate, so
    /// epics, legendaries, and set pieces all accept it.
    fn perform_socket(
        &mut self,
        state: &DragState,
        grid: GridId,
        target_index: usize,
    ) -> Result<String, String> {
        let piece = self.take_at(state.source, state.index)?;
        if piece.base != state.item.base {
            let origin = state.item.position;
            self.restore_dropped(state.source, piece, origin)?;
            return Err("item moved under the drag — drop ignored".to_string());
        }
        let origin = piece.position;
        let target_index = if state.source == grid && state.index < target_index {
            target_index - 1
        } else {
            target_index
        };
        let allowed = {
            let db = match &self.game {
                GameStatus::Loaded(data) => Some(data),
                GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
            };
            self.item_at(grid, target_index)
                .is_ok_and(|target| transfer::can_socket(db, &piece, &target))
        };
        if !allowed {
            self.restore_dropped(state.source, piece, origin)?;
            return Err("socket target moved — drop ignored".to_string());
        }
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let piece_label = self.caches.names.record_name(db, &piece.base);
        let target_label = self.item_at(grid, target_index).map_or_else(
            |_| "item".to_string(),
            |t| self.caches.names.record_name(db, &t.base),
        );
        match self.grid_item_mut(grid, target_index) {
            Some(target) => transfer::socket_relic(target, piece),
            None => return Err("socket target moved — drop ignored".to_string()),
        }
        self.mark_dirty(state.source);
        self.mark_dirty(grid);
        self.left_selected = None;
        if let Some(pane) = &mut self.right {
            pane.selected = None;
        }
        Ok(format!("socketed {piece_label} into {target_label}"))
    }

    /// Drops the dragged item into an empty, type-matching paper-doll
    /// slot: it comes off its container and onto the character.
    fn perform_equip(&mut self, state: &DragState, grid: GridId) -> Result<String, String> {
        let GridId::Equipment(slot) = grid else {
            return Err("not an equipment slot".to_string());
        };
        let item = self.take_at(state.source, state.index)?;
        if item.base != state.item.base {
            let origin = state.item.position;
            self.restore_dropped(state.source, item, origin)?;
            return Err("item moved under the drag — drop ignored".to_string());
        }
        let origin = item.position;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        if !transfer::can_equip(db, &item, slot) {
            self.restore_dropped(state.source, item, origin)?;
            return Err("that item cannot be worn there".to_string());
        }
        let label = self.caches.names.item_label(db, &item);
        let pane = self.character.as_mut().ok_or("no character loaded")?;
        match transfer::equip(&mut pane.character, item, slot) {
            Ok(()) => {
                self.mark_dirty(state.source);
                self.mark_dirty(grid);
                self.left_selected = None;
                if let Some(pane) = &mut self.right {
                    pane.selected = None;
                }
                Ok(format!("{label} equipped — {}", slot.label()))
            }
            Err(rejected) => {
                let reason = rejected.reason;
                self.restore_dropped(state.source, *rejected.item, origin)?;
                Err(format!("{reason}; item returned"))
            }
        }
    }

    /// Reads and inspects the install's Game.dll for the modal;
    /// nothing is written here.
    fn open_dll_patch_dialog(&mut self) {
        let Some(dir) = stored_game_dir() else {
            self.status = Some(Err("no game directory remembered yet".to_string()));
            return;
        };
        let path = dir.join("Game.dll");
        let outcome = std::fs::read(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))
            .map(|bytes| {
                let state = univault_core::dllpatch::inspect(&bytes);
                (bytes, state)
            });
        self.dll_patch = Some(DllPatchDialog { path, outcome });
    }

    /// Applies or reverts the socket patch: pristine backup first
    /// (created once, only from a fully vanilla file), then an
    /// atomic-rename write, then a re-read to verify.
    fn toggle_dll_patch(&mut self, enable: bool) -> Result<String, String> {
        use univault_core::dllpatch::{self, PatchState};
        let dialog = self.dll_patch.as_ref().ok_or("no dll loaded")?;
        let path = dialog.path.clone();
        let (bytes, state) = dialog.outcome.as_ref().map_err(Clone::clone)?;
        let mut working = bytes.clone();
        let backup = path.with_extension("dll.univault-original");
        if matches!(state, PatchState::Vanilla { .. }) && !backup.exists() {
            std::fs::write(&backup, &working)
                .map_err(|error| format!("backup {}: {error}", backup.display()))?;
        }
        let changed = if enable {
            dllpatch::enable(&mut working)
        } else {
            dllpatch::disable(&mut working)
        };
        if changed == 0 {
            self.dll_patch = None;
            return Ok("nothing to change".to_string());
        }
        let staging = path.with_extension("dll.univault-tmp");
        std::fs::write(&staging, &working)
            .map_err(|error| format!("write {}: {error}", staging.display()))?;
        std::fs::rename(&staging, &path)
            .map_err(|error| format!("replace {}: {error}", path.display()))?;
        let verified =
            std::fs::read(&path).map_err(|error| format!("verify {}: {error}", path.display()))?;
        let state = dllpatch::inspect(&verified);
        self.dll_patch = None;
        match (enable, state) {
            (true, PatchState::Patched { sites }) => Ok(format!(
                "socket patch ON — {sites} site(s) patched (pristine copy kept as {})",
                backup.display()
            )),
            (false, PatchState::Vanilla { sites }) => {
                Ok(format!("socket patch OFF — {sites} site(s) restored"))
            }
            (_, verified_state) => Err(format!(
                "verification after write found {verified_state:?} — restore the backup at {}",
                backup.display()
            )),
        }
    }

    fn show_dll_patch_modal(&mut self, ctx: &egui::Context) {
        use univault_core::dllpatch::PatchState;
        let Some(dialog) = &self.dll_patch else {
            return;
        };
        let mut close = false;
        let mut apply: Option<bool> = None;
        egui::Modal::new(egui::Id::new("dll-patch-modal")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.label(theme::heading("Game.dll socket patch"));
            ui.label(format!("{}", dialog.path.display()));
            ui.add_space(6.0);
            match &dialog.outcome {
                Err(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                Ok((_, state)) => {
                    match state {
                        PatchState::Vanilla { sites } => {
                            ui.label(format!(
                                "Status: UNPATCHED — {sites} signature site(s) found. \
                                 Enabling lets the game itself socket relics and charms \
                                 into Epic and Legendary items (type rules and costs \
                                 stay the game's own)."
                            ));
                        }
                        PatchState::Patched { sites } => {
                            ui.label(format!(
                                "Status: PATCHED — {sites} site(s) active. Disabling \
                                 restores the original bytes exactly."
                            ));
                        }
                        PatchState::Mixed { vanilla, patched } => {
                            ui.label(format!(
                                "Status: PARTIAL — {patched} patched, {vanilla} untouched \
                                 (an interrupted or older-guide edit). Enable completes \
                                 the patch; disable reverts everything."
                            ));
                        }
                        PatchState::Unrecognized => {
                            ui.label(
                                "Status: UNRECOGNIZED — this Game.dll doesn't match the \
                                 known signature (a new game version, or other mods). \
                                 Nothing will be written.",
                            );
                        }
                    }
                    ui.add_space(6.0);
                    ui.weak(
                        "A pristine copy is kept beside the dll the first time it is \
                         patched. Steam updates and 'verify integrity' replace the dll — \
                         just re-enable afterwards. Considered cheating in multiplayer.",
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let (can_enable, can_disable) = match state {
                            PatchState::Vanilla { .. } => (true, false),
                            PatchState::Patched { .. } => (false, true),
                            PatchState::Mixed { .. } => (true, true),
                            PatchState::Unrecognized => (false, false),
                        };
                        if ui
                            .add_enabled(can_enable, egui::Button::new("Enable patch"))
                            .clicked()
                        {
                            apply = Some(true);
                        }
                        if ui
                            .add_enabled(can_disable, egui::Button::new("Disable patch"))
                            .clicked()
                        {
                            apply = Some(false);
                        }
                    });
                }
            }
            ui.add_space(8.0);
            close = ui.button("Close").clicked();
        });
        if let Some(enable) = apply {
            self.status = Some(self.toggle_dll_patch(enable));
        } else if close {
            self.dll_patch = None;
        }
    }

    /// Prepares the completion-bonus picker for the piece at
    /// `(grid, index)`; silently skipped when the record has no
    /// bonus table (the piece simply completes bonus-less).
    /// `current` marks the bonus already on the piece when
    /// re-picking.
    fn begin_bonus_pick(
        &mut self,
        grid: GridId,
        index: usize,
        base: RecordId,
        current: Option<RecordId>,
    ) {
        let GameStatus::Loaded(db) = &self.game else {
            return;
        };
        let mut options = Vec::new();
        let mut total_weight: i64 = 0;
        for (path, weight) in db.relic_bonuses(&base) {
            let Some(record) = RecordId::parse(path.clone()) else {
                continue;
            };
            let name = self.caches.names.record_name(Some(db), &record);
            let lines = stats::record_lines(db, &record);
            total_weight += i64::from((*weight).max(0));
            options.push(BonusOption {
                record,
                weight: *weight,
                name,
                lines,
            });
        }
        if options.is_empty() {
            return;
        }
        let name = self.caches.names.record_name(Some(db), &base);
        self.pending_bonus = Some(PendingBonus {
            grid,
            index,
            base,
            name,
            options,
            total_weight: total_weight.max(1),
            current,
        });
    }

    /// Double-click on a completed relic/charm or an artifact:
    /// re-open the bonus picker for it. Other items are ignored; a
    /// partial relic piece gets a hint instead.
    fn request_bonus_edit(&mut self, grid: GridId, index: usize) {
        let Ok(item) = self.item_at(grid, index) else {
            return;
        };
        let (needed, has_table) = match &self.game {
            GameStatus::Loaded(db) => (
                db.completed_relic_level(&item.base),
                !db.relic_bonuses(&item.base).is_empty(),
            ),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => (None, false),
        };
        if let Some(needed) = needed
            && item.var1 < needed
        {
            self.status = Some(Err(format!(
                "complete the piece first ({}/{needed} shards)",
                item.var1
            )));
            return;
        }
        if !has_table {
            if needed.is_some() {
                self.status = Some(Err("this piece has no bonus table".to_string()));
            }
            return;
        }
        self.begin_bonus_pick(grid, index, item.base.clone(), item.relic_bonus.clone());
    }

    /// Writes the chosen completion bonus (or none) onto the
    /// completed piece.
    fn apply_bonus(&mut self, choice: Option<RecordId>) -> Result<String, String> {
        let pending = self.pending_bonus.take().ok_or("no bonus pending")?;
        let stale = "the completed piece moved — bonus not applied";
        match self.grid_item_mut(pending.grid, pending.index) {
            Some(item) if item.base == pending.base => {
                item.relic_bonus.clone_from(&choice);
            }
            Some(_) | None => return Err(stale.to_string()),
        }
        self.mark_dirty(pending.grid);
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        Ok(match choice {
            Some(record) => {
                let bonus = self.caches.names.record_name(db, &record);
                format!("{} — bonus: {bonus}", pending.name)
            }
            None => format!("{} — no bonus", pending.name),
        })
    }

    fn show_bonus_modal(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_bonus else {
            return;
        };
        let mut choice: Option<Option<RecordId>> = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("bonus-modal")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.label(theme::heading(format!(
                "Completion bonus — {}",
                pending.name
            )));
            ui.weak("The game would roll one of these; pick yours (odds shown).");
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .max_height(420.0)
                .show(ui, |ui| {
                    for option in &pending.options {
                        let odds = i64::from(option.weight.max(0)) * 100 / pending.total_weight;
                        let is_current = pending.current.as_ref() == Some(&option.record);
                        let label = if is_current {
                            format!("{} — {odds}% (current)", option.name)
                        } else {
                            format!("{} — {odds}%", option.name)
                        };
                        ui.group(|ui| {
                            if ui.button(label).clicked() {
                                choice = Some(Some(option.record.clone()));
                            }
                            for line in &option.lines {
                                ui.label(
                                    egui::RichText::new(&line.text)
                                        .color(game_color(stats::palette_color(line.color)))
                                        .size(12.0),
                                );
                            }
                        });
                    }
                });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Roll (game odds)").clicked() {
                    let weights: Vec<i32> =
                        pending.options.iter().map(|option| option.weight).collect();
                    if let Some(index) = weighted_index(&weights, entropy_roll()) {
                        choice = Some(Some(pending.options[index].record.clone()));
                    }
                }
                if ui.button("No bonus").clicked() {
                    choice = Some(None);
                }
                close = ui.button("Decide later").clicked();
            });
        });
        if let Some(choice) = choice {
            self.status = Some(self.apply_bonus(choice));
        } else if close || modal.should_close() {
            // Nothing is written on close: a just-completed piece
            // stays bonus-less, an edited piece keeps its bonus —
            // double-click it any time to pick again.
            self.pending_bonus = None;
            self.status = Some(Ok(
                "no bonus chosen — double-click the piece to pick later".to_string()
            ));
        }
    }

    /// Moves the dragged item to the drop cell; on a failed placement
    /// the item goes back where it came from.
    fn perform_drop(&mut self, state: &DragState, target: DropCandidate) -> Result<String, String> {
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let label = self.caches.names.item_label(db, &state.item);

        let taken = self.take_at(state.source, state.index)?;

        if taken.base != state.item.base {
            let origin = state.item.position;
            self.restore_dropped(state.source, taken, origin)?;
            return Err("item moved under the drag — drop ignored".to_string());
        }
        let origin = taken.position;

        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let placed = match target.grid {
            GridId::Sack(sack) => {
                let Some(pane) = self.character.as_mut() else {
                    return Err("no character loaded".to_string());
                };
                transfer::place_in_character_at(&mut pane.character, taken, sack, target.cell, db)
            }
            // Equipment drops travel the equip path, never this one.
            GridId::Equipment(_) => {
                self.restore_dropped(state.source, taken, origin)?;
                return Err("drop onto a highlighted slot to equip".to_string());
            }
            GridId::Bank => {
                let Some(pane) = self.bank.as_mut() else {
                    return Err("no bank loaded".to_string());
                };
                transfer::place_in_stash_at(&mut pane.stash, taken, target.cell, db)
            }
            GridId::Shared => {
                let Some(pane) = self.shared.as_mut() else {
                    return Err("no shared bank loaded".to_string());
                };
                transfer::place_in_stash_at(&mut pane.stash, taken, target.cell, db)
            }
            GridId::Relic => {
                let Some(pane) = self.relics.as_mut() else {
                    return Err("no relic bank loaded".to_string());
                };
                transfer::place_in_stash_at(&mut pane.stash, taken, target.cell, db)
            }
            GridId::VaultTab(tab) | GridId::SearchDoc { tab, .. } => {
                let Some((vault, _)) =
                    vault_source_mut(&mut self.right, &mut self.search.docs, target.grid)
                else {
                    return Err("no vault loaded".to_string());
                };
                transfer::place_in_vault_at(vault, taken, tab, target.cell, db)
            }
        };

        match placed {
            Ok(()) => {
                self.mark_dirty(state.source);
                self.mark_dirty(target.grid);
                self.left_selected = None;
                if let Some(pane) = &mut self.right {
                    pane.selected = None;
                }
                let destination = match target.grid {
                    GridId::Sack(sack) => format!("sack {}", sack + 1),
                    GridId::Equipment(slot) => slot.label().to_string(),
                    GridId::Bank => "bank".to_string(),
                    GridId::Shared => "shared bank".to_string(),
                    GridId::Relic => "relic bank".to_string(),
                    GridId::VaultTab(tab) | GridId::SearchDoc { tab, .. } => {
                        format!("vault tab {}", tab + 1)
                    }
                };
                Ok(format!(
                    "{label} → {destination} ({}, {})",
                    target.cell.x, target.cell.y
                ))
            }
            Err(rejected) => {
                let reason = rejected.reason;
                self.restore_dropped(state.source, *rejected.item, origin)?;
                Err(format!("{reason}; item returned"))
            }
        }
    }

    /// Puts a taken item back at its original cell (guaranteed free),
    /// falling back to any open spot.
    fn restore_dropped(
        &mut self,
        source: GridId,
        item: Item,
        position: univault_core::chr::GridPos,
    ) -> Result<(), String> {
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let lost = "item could not be returned — reload without saving".to_string();
        match source {
            GridId::Sack(sack) => {
                let pane = self.character.as_mut().ok_or_else(|| lost.clone())?;
                transfer::place_in_character_at(&mut pane.character, item, sack, position, db)
                    .or_else(|rejected| {
                        transfer::place_in_character(&mut pane.character, *rejected.item, sack, db)
                            .map(|_| ())
                    })
                    .map_err(|_| lost)
            }
            GridId::Equipment(slot) => {
                let pane = self.character.as_mut().ok_or_else(|| lost.clone())?;
                transfer::equip(&mut pane.character, item, slot)
                    .or_else(|rejected| {
                        transfer::place_in_character(&mut pane.character, *rejected.item, 0, db)
                            .map(|_| ())
                    })
                    .map_err(|_| lost)
            }
            GridId::Bank => {
                let pane = self.bank.as_mut().ok_or_else(|| lost.clone())?;
                transfer::place_in_stash_at(&mut pane.stash, item, position, db)
                    .or_else(|rejected| {
                        transfer::place_in_stash(&mut pane.stash, *rejected.item, db)
                    })
                    .map_err(|_| lost)
            }
            GridId::Shared => {
                let pane = self.shared.as_mut().ok_or_else(|| lost.clone())?;
                transfer::place_in_stash_at(&mut pane.stash, item, position, db)
                    .or_else(|rejected| {
                        transfer::place_in_stash(&mut pane.stash, *rejected.item, db)
                    })
                    .map_err(|_| lost)
            }
            GridId::Relic => {
                let pane = self.relics.as_mut().ok_or_else(|| lost.clone())?;
                transfer::place_in_stash_at(&mut pane.stash, item, position, db)
                    .or_else(|rejected| {
                        transfer::place_in_stash(&mut pane.stash, *rejected.item, db)
                    })
                    .map_err(|_| lost)
            }
            GridId::VaultTab(tab) | GridId::SearchDoc { tab, .. } => {
                let (vault, _) = vault_source_mut(&mut self.right, &mut self.search.docs, source)
                    .ok_or_else(|| lost.clone())?;
                transfer::place_in_vault_at(vault, item, tab, position, db)
                    .or_else(|rejected| {
                        transfer::place_in_vault(vault, *rejected.item, tab, db).map(|_| ())
                    })
                    .map_err(|_| lost)
            }
        }
    }

    fn mark_dirty(&mut self, grid: GridId) {
        // Any mutation can move rows the search table points at.
        self.search.mark_data_changed();
        let dirty = match grid {
            GridId::Sack(_) | GridId::Equipment(_) => {
                self.character.as_mut().map(|pane| &mut pane.dirty)
            }
            GridId::Bank => self.bank.as_mut().map(|pane| &mut pane.dirty),
            GridId::Shared => self.shared.as_mut().map(|pane| &mut pane.dirty),
            GridId::Relic => self.relics.as_mut().map(|pane| &mut pane.dirty),
            GridId::VaultTab(_) => self.right.as_mut().map(|pane| &mut pane.dirty),
            GridId::SearchDoc { doc, .. } => {
                self.search.docs.get_mut(doc).map(|doc| &mut doc.dirty)
            }
        };
        if let Some(dirty) = dirty {
            *dirty = true;
        }
    }

    /// Computes a respec's refund from the pane's baseline bytes and
    /// opens the confirmation modal.
    fn preview_respec(&mut self, kind: RespecKind) {
        let Some(pane) = &self.character else { return };
        let preview = match kind {
            RespecKind::Attributes => {
                respec::attribute_refund(&pane.original).map(|points| (points, 0))
            }
            RespecKind::Skills => respec::skill_refund(&pane.original),
        };
        match preview {
            Ok((points, skills_removed)) => {
                self.pending_respec = Some(PendingRespec {
                    kind,
                    points,
                    skills_removed,
                });
            }
            Err(error) => self.status = Some(Err(format!("respec unavailable: {error}"))),
        }
    }

    fn show_respec_modal(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_respec else {
            return;
        };
        let (title, body) = match pending.kind {
            RespecKind::Attributes => (
                "Respec attributes?",
                format!(
                    "Attributes return to base values; {} attribute points will be refunded.",
                    pending.points
                ),
            ),
            RespecKind::Skills => (
                "Respec skills & masteries?",
                format!(
                    "{} skills and both masteries will be removed; {} skill points will be refunded. \
                     The class resets so both masteries can be picked again.",
                    pending.skills_removed, pending.points
                ),
            ),
        };
        let nothing_to_do = pending.points == 0 && pending.skills_removed == 0;
        let kind = pending.kind;
        let mut close = false;
        let mut confirm = false;
        let modal = egui::Modal::new(egui::Id::new("respec-modal")).show(ctx, |ui| {
            ui.set_max_width(340.0);
            ui.label(theme::heading(title));
            if nothing_to_do {
                ui.label("Nothing to refund — this character is already respecced.");
            } else {
                ui.label(body);
                ui.weak("Saves automatically once confirmed.");
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if nothing_to_do {
                    close = ui.button("Close").clicked();
                } else {
                    close = ui.button("Cancel").clicked();
                    confirm = ui.button("Respec").clicked();
                }
            });
        });
        if confirm {
            self.status = Some(self.apply_respec(kind));
        }
        if close || confirm || modal.should_close() {
            self.pending_respec = None;
        }
    }

    fn apply_respec(&mut self, kind: RespecKind) -> Result<String, String> {
        let pane = self.character.as_mut().ok_or("no character loaded")?;
        let result = match kind {
            RespecKind::Attributes => respec::respec_attributes(&pane.original),
            RespecKind::Skills => respec::respec_skills(&pane.original),
        }
        .map_err(|error| error.to_string())?;
        pane.original = result.bytes;
        pane.dirty = true;
        Ok(match kind {
            RespecKind::Attributes => {
                format!("refunded {} attribute points", result.refunded_points)
            }
            RespecKind::Skills => format!(
                "removed {} skills, refunded {} skill points",
                result.skills_removed, result.refunded_points
            ),
        })
    }

    /// Where file dialogs start: near what the user last touched.
    fn dialog_start_dir(&self) -> Option<PathBuf> {
        self.character
            .as_ref()
            .map(|pane| pane.path.clone())
            .or_else(|| self.bank.as_ref().map(|pane| pane.path.clone()))
            .or_else(|| self.shared.as_ref().map(|pane| pane.path.clone()))
            .or_else(|| self.relics.as_ref().map(|pane| pane.path.clone()))
            .or_else(|| self.recents.entries.first().cloned())
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
    }

    fn show_panes(&mut self, ui: &mut egui::Ui) -> (Option<PaneAction>, DragFrame) {
        let caches = &mut self.caches;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let drag = self.drag.clone();
        let mut frame = DragFrame::default();
        let mut action = None;
        let has_left = self.character.is_some()
            || self.bank.is_some()
            || self.shared.is_some()
            || self.relics.is_some();
        let can_move = has_left && self.right.is_some();
        ui.columns(2, |columns| {
            if has_left {
                let mut view = LeftView {
                    character: &mut self.character,
                    bank: &mut self.bank,
                    shared: &mut self.shared,
                    relics: &mut self.relics,
                    active_tab: &mut self.active_tab,
                    inventory_tab: &mut self.inventory_tab,
                    selected: &mut self.left_selected,
                };
                let pane_chrome = caches.chrome(columns[0].ctx(), db);
                let active = show_left_tabs(&mut columns[0], &mut view, pane_chrome.as_ref());
                framed_pane_anchored(&mut columns[0], pane_chrome.as_ref(), active, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("file-pane")
                        .show(ui, |ui| {
                            if let Some(chosen) = show_left_column(
                                ui,
                                &mut view,
                                db,
                                caches,
                                can_move,
                                drag.as_ref(),
                                &mut frame,
                            ) {
                                action = Some(chosen);
                            }
                        });
                });
            } else {
                columns[0].weak("No game file loaded.");
            }
            if let Some(pane) = &mut self.right {
                let pane_chrome = caches.chrome(columns[1].ctx(), db);
                let active =
                    show_vault_tabs(&mut columns[1], pane, pane_chrome.as_ref(), drag.as_ref());
                framed_pane_anchored(&mut columns[1], pane_chrome.as_ref(), active, |ui| {
                    if let Some(chosen) =
                        show_vault_pane(ui, pane, db, caches, can_move, drag.as_ref(), &mut frame)
                    {
                        action = Some(chosen);
                    }
                });
            } else {
                columns[1].weak("No vault loaded.");
            }
        });
        (action, frame)
    }
}

/// The Inventory view's exclusive sub-tab strip:
/// Equipment | Main Sack | Sack 1 | … | Sack n. Mid-drag, pointing
/// at a tab switches to it so a drop can land in any sack or on the
/// doll.
fn show_inventory_tabs(
    ui: &mut egui::Ui,
    pane: &CharacterPane,
    db: Option<&GameCache>,
    caches: &mut Caches,
    inventory_tab: &mut InventoryTab,
    selected: &mut Option<(GridId, usize)>,
    drag: Option<&DragState>,
) -> Option<(egui::Rect, String)> {
    let pane_chrome = caches.chrome(ui.ctx(), db);
    let cursor = ui.ctx().pointer_latest_pos();
    let mut active = None;
    ui.horizontal_wrapped(|ui| {
        let mut tab_button = |ui: &mut egui::Ui, target: InventoryTab, label: &str| {
            let selected_tab = *inventory_tab == target;
            let response = match pane_chrome.as_ref() {
                Some(pane_chrome) => pane_chrome.tab(ui, selected_tab, true, label),
                None => ui.add(egui::Button::selectable(selected_tab, label)),
            };
            if selected_tab {
                active = Some((response.rect, label.to_owned()));
            }
            let drag_over =
                drag.is_some() && cursor.is_some_and(|cursor| response.rect.contains(cursor));
            if (response.clicked() || drag_over) && *inventory_tab != target {
                *inventory_tab = target;
                *selected = None;
            }
        };
        tab_button(ui, InventoryTab::Equipment, "Equipment");
        for (index, sack) in pane.character.sacks.iter().enumerate() {
            let label = if index == 0 {
                format!("Main Sack ({})", sack.items.len())
            } else {
                format!("Sack {} ({})", index, sack.items.len())
            };
            tab_button(ui, InventoryTab::Sack(index), &label);
        }
    });
    active
}

/// Allocates the doll's canvas centered in the pane, scaled to fill
/// the available width.
fn allocate_doll_canvas(ui: &mut egui::Ui) -> (egui::Rect, egui::Response, f32) {
    let cell = fill_cell_size(ui.available_width(), DOLL_CELLS.0);
    let size = egui::vec2(cells_at(DOLL_CELLS.0, cell), cells_at(DOLL_CELLS.1, cell));
    let pad = ((ui.available_width() - size.x) / 2.0).max(0.0);
    let (rect, response) = ui
        .horizontal(|ui| {
            ui.add_space(pad);
            ui.allocate_exact_size(size, egui::Sense::click_and_drag())
        })
        .inner;
    (rect, response, cell)
}

/// The doll's canvas: light flagstone near full brightness, or the
/// painted fallback.
fn paint_doll_canvas(
    painter: &egui::Painter,
    doll_chrome: Option<&chrome::Chrome>,
    rect: egui::Rect,
    visuals: &egui::Visuals,
) {
    match doll_chrome {
        Some(pane_chrome) => pane_chrome.backdrop(painter, rect, egui::Color32::from_gray(235)),
        None => {
            painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
        }
    }
}

/// A doll slot's backdrop: the slot's own engraved plate from the
/// character screen, or the painted fallback fill.
fn paint_slot_inset(
    painter: &egui::Painter,
    doll_chrome: Option<&chrome::Chrome>,
    slot: EquipSlot,
    box_rect: egui::Rect,
) {
    match doll_chrome {
        Some(pane_chrome) => {
            pane_chrome.slot_plate(painter, slot_plate_of(slot), box_rect);
        }
        None => {
            painter.rect_filled(box_rect, 2.0, theme::SURFACE_DEEP.gamma_multiply(0.85));
        }
    }
}

/// The engraved character-screen plate that backs each doll slot.
fn slot_plate_of(slot: EquipSlot) -> chrome::SlotPlate {
    match slot {
        EquipSlot::Head => chrome::SlotPlate::Helm,
        EquipSlot::Neck => chrome::SlotPlate::Amulet,
        EquipSlot::Torso => chrome::SlotPlate::Torso,
        EquipSlot::Legs => chrome::SlotPlate::Legs,
        EquipSlot::Arms => chrome::SlotPlate::Arms,
        EquipSlot::Ring1 | EquipSlot::Ring2 => chrome::SlotPlate::Ring,
        EquipSlot::LeftHand | EquipSlot::LeftHandAlternate => chrome::SlotPlate::WeaponLeft,
        EquipSlot::RightHand | EquipSlot::RightHandAlternate => chrome::SlotPlate::WeaponRight,
        EquipSlot::Artifact => chrome::SlotPlate::Artifact,
    }
}

/// One Help-modal section: a gold section title over bulleted,
/// wrapping lines.
fn help_section(ui: &mut egui::Ui, title: &str, lines: &[&str]) {
    ui.add_space(10.0);
    ui.label(theme::section(title));
    for line in lines {
        ui.label(format!("•  {line}"));
    }
}

/// A pane heading: the iron nameplate under chrome, gold classical
/// text otherwise.
fn pane_heading(ui: &mut egui::Ui, pane_chrome: Option<&chrome::Chrome>, text: &str) {
    match pane_chrome {
        Some(pane_chrome) => pane_chrome.nameplate(ui, text),
        None => {
            ui.label(theme::heading(text));
        }
    }
}

/// An action button: the game's gold plate under chrome, a themed
/// egui button otherwise.
fn plate_button(
    ui: &mut egui::Ui,
    pane_chrome: Option<&chrome::Chrome>,
    enabled: bool,
    text: &str,
) -> egui::Response {
    match pane_chrome {
        Some(pane_chrome) => pane_chrome.button(ui, enabled, text),
        None => ui.add_enabled(enabled, egui::Button::new(text)),
    }
}

/// [`framed_pane`], with a tab strip already rendered above: the
/// frame tucks up under the strip and the active tab is repainted
/// joined through the frame's top band — the game's anchored tabs.
fn framed_pane_anchored(
    ui: &mut egui::Ui,
    pane_chrome: Option<&chrome::Chrome>,
    active: Option<(egui::Rect, String)>,
    add: impl FnOnce(&mut egui::Ui),
) {
    let Some(pane_chrome) = pane_chrome else {
        ui.separator();
        add(ui);
        return;
    };
    ui.add_space(-4.0);
    pane_chrome.interior(ui.painter(), ui.available_rect_before_wrap());
    let response = egui::Frame::NONE
        .inner_margin(chrome::FRAME_MARGIN)
        .show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            add(ui);
        });
    let rect = response.response.rect;
    chrome::inner_shadow(ui.painter(), rect.shrink(12.0), 14.0);
    pane_chrome.pane_frame(ui.painter(), rect);
    if let Some((tab_rect, label)) = active {
        chrome::tab_anchor(pane_chrome, ui.painter(), tab_rect, &label, 14.0);
    }
}

/// Wraps a pane in the caravan window frame when chrome is loaded:
/// content sits inside [`chrome::FRAME_MARGIN`], the frame paints
/// over the reserved bands, and the pane fills the viewport height
/// so the frame encloses the whole column.
fn framed_pane(
    ui: &mut egui::Ui,
    pane_chrome: Option<&chrome::Chrome>,
    add: impl FnOnce(&mut egui::Ui),
) {
    let Some(pane_chrome) = pane_chrome else {
        add(ui);
        return;
    };
    // Painted up front over the predicted rect (the pane fills its
    // column) so content never sits on the stone backdrop.
    pane_chrome.interior(ui.painter(), ui.available_rect_before_wrap());
    let response = egui::Frame::NONE
        .inner_margin(chrome::FRAME_MARGIN)
        .show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            add(ui);
        });
    let rect = response.response.rect;
    chrome::inner_shadow(ui.painter(), rect.shrink(12.0), 14.0);
    pane_chrome.pane_frame(ui.painter(), rect);
}

/// Native size of one grid cell — the textures' 32 pixels; grids
/// scale their on-screen cell up from this to fill their pane.
const CELL_SIZE: f32 = 32.0;

/// The on-screen cell size that fills the available width with
/// `columns` cells: never below native, capped at 2× so icons stay
/// crisp.
#[allow(clippy::cast_precision_loss)] // column counts are tiny
fn fill_cell_size(available: f32, columns: i32) -> f32 {
    (((available - 4.0) / columns.max(1) as f32).floor()).clamp(CELL_SIZE, CELL_SIZE * 2.0)
}

// Grid coordinates are small integers; f32 represents them exactly.
#[allow(clippy::cast_precision_loss)]
fn cells_at(cells: i32, cell: f32) -> f32 {
    cells as f32 * cell
}

// Grid coordinates are small integers; f32 represents them exactly.
#[allow(clippy::cast_precision_loss)]
fn cells_to_points(cells: i32) -> f32 {
    cells as f32 * CELL_SIZE
}

/// Which on-screen grid an item lives in — the address space of
/// selection and drag-and-drop. Equipment carries its slot in the
/// id (the paper doll has no cell grid); the index half of an
/// `(GridId, usize)` address is 0 there.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GridId {
    Sack(usize),
    Equipment(EquipSlot),
    Bank,
    Shared,
    Relic,
    VaultTab(usize),
    /// A tab of one of the search view's loaded vaults (never the
    /// open vault pane — its tabs stay `VaultTab`).
    SearchDoc {
        doc: usize,
        tab: usize,
    },
}

/// The tab a vault-side grid addresses; `None` for left-side grids.
fn vault_tab_of(grid: GridId) -> Option<usize> {
    match grid {
        GridId::VaultTab(tab) | GridId::SearchDoc { tab, .. } => Some(tab),
        GridId::Sack(_) | GridId::Equipment(_) | GridId::Bank | GridId::Shared | GridId::Relic => {
            None
        }
    }
}

/// The vault model (and dirty flag) a vault-side grid addresses: the
/// open pane for `VaultTab`, a search-loaded vault for `SearchDoc`.
/// A free function over the two fields so callers can keep other
/// disjoint borrows of `App` alive.
fn vault_source_mut<'a>(
    right: &'a mut Option<VaultPane>,
    docs: &'a mut [search::SearchDoc],
    grid: GridId,
) -> Option<(&'a mut Vault, &'a mut bool)> {
    match grid {
        GridId::VaultTab(_) => right
            .as_mut()
            .map(|pane| (&mut pane.vault, &mut pane.dirty)),
        GridId::SearchDoc { doc, .. } => docs
            .get_mut(doc)
            .map(|doc| (&mut doc.vault, &mut doc.dirty)),
        GridId::Sack(_) | GridId::Equipment(_) | GridId::Bank | GridId::Shared | GridId::Relic => {
            None
        }
    }
}

/// An in-flight drag: where the item came from and how it was
/// grabbed. The item stays in its container (painted dimmed) until
/// the drop commits.
#[derive(Clone)]
struct DragState {
    source: GridId,
    index: usize,
    item: Item,
    /// Pointer offset from the item's top-left, in points, so the
    /// item hangs where it was grabbed.
    grab: egui::Vec2,
}

/// The cell a drop would land in, computed by whichever grid the
/// pointer is over this frame.
#[derive(Clone, Copy)]
struct DropCandidate {
    grid: GridId,
    cell: univault_core::chr::GridPos,
    fits: bool,
    /// A matching partial relic/charm under the pointer: dropping
    /// combines into the item at this index instead of placing.
    combine_with: Option<usize>,
    /// Socketable gear under the pointer: dropping sockets the
    /// dragged relic/charm into the item at this index.
    socket_into: Option<usize>,
    /// An empty paper-doll slot the dragged item may be worn in
    /// (`grid` is the `Equipment` slot): dropping equips.
    equips: bool,
}

/// What the grids reported back this frame.
#[derive(Default)]
struct DragFrame {
    begin: Option<DragState>,
    candidate: Option<DropCandidate>,
    /// A Shift+Click asking for the item at `(grid, index)` to be
    /// duplicated into its own container.
    duplicate: Option<(GridId, usize)>,
    /// A right-click asking for the item at `(grid, index)` to be
    /// sent straight to the other pane.
    quick_move: Option<(GridId, usize)>,
    /// A Shift+Right-click asking for a copy of the item at
    /// `(grid, index)` to be placed in the other pane.
    copy_across: Option<(GridId, usize)>,
    /// A double-click asking to (re)pick the completion bonus of the
    /// completed relic/charm at `(grid, index)`.
    edit_bonus: Option<(GridId, usize)>,
    /// An Alt+Click asking to split the socketed relic/charm out of
    /// the item at `(grid, index)` — both survive.
    extract: Option<(GridId, usize)>,
}

/// Paints a container as its actual cell grid, with items at their
/// positions (icon when decodable, initial letter otherwise), click
/// selection, a name tooltip on hover, and drag-and-drop with a
/// green/red footprint preview.
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn grid_view(
    ui: &mut egui::Ui,
    dims: (i32, i32),
    entries: &[(usize, &Item)],
    grid: GridId,
    selected: &mut Option<(GridId, usize)>,
    db: Option<&GameCache>,
    caches: &mut Caches,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) {
    let cell = fill_cell_size(ui.available_width(), dims.0);
    let size = egui::vec2(cells_at(dims.0, cell), cells_at(dims.1, cell));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();
    if let Some(chrome) = caches.chrome(ui.ctx(), db) {
        for row in 0..dims.1 {
            for column in 0..dims.0 {
                let tile = egui::Rect::from_min_size(
                    rect.min + egui::vec2(cells_at(column, cell), cells_at(row, cell)),
                    egui::vec2(cell, cell),
                );
                chrome.grid_cell(&painter, tile);
            }
        }
    } else {
        painter.rect_filled(rect, 2.0, theme::GRID_BG);
        let grid_stroke = egui::Stroke::new(0.5, theme::GRID_LINE);
        for column in 0..=dims.0 {
            let x = rect.min.x + cells_at(column, cell);
            painter.line_segment(
                [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
                grid_stroke,
            );
        }
        for row in 0..=dims.1 {
            let y = rect.min.y + cells_at(row, cell);
            painter.line_segment(
                [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
                grid_stroke,
            );
        }
    }

    // The grab position decides which item a starting drag lifts —
    // the pointer may already have moved past egui's drag threshold.
    let press_origin = ui.ctx().input(|input| input.pointer.press_origin());

    let mut hovered: Option<&Item> = None;
    for (index, item) in entries {
        let (width, height) = caches.footprint(db, item);
        let item_rect = egui::Rect::from_min_size(
            rect.min
                + egui::vec2(
                    cells_at(item.position.x, cell),
                    cells_at(item.position.y, cell),
                ),
            egui::vec2(cells_at(width, cell), cells_at(height, cell)),
        )
        .shrink(1.0);
        let is_selected = *selected == Some((grid, *index));
        paint_item_tile(
            ui,
            &painter,
            item_rect,
            item,
            is_selected,
            &visuals,
            db,
            caches,
        );

        // The lifted item stays put but fades until the drop lands.
        if drag.is_some_and(|state| state.source == grid && state.index == *index) {
            painter.rect_filled(item_rect, 2.0, egui::Color32::from_black_alpha(140));
        }

        if response.drag_started()
            && drag.is_none()
            && frame.begin.is_none()
            && press_origin.is_some_and(|origin| item_rect.contains(origin))
        {
            frame.begin = Some(DragState {
                source: grid,
                index: *index,
                item: (*item).clone(),
                grab: press_origin.map_or(egui::Vec2::ZERO, |origin| origin - item_rect.min),
            });
        }

        if drag.is_none()
            && let Some(pointer) = response.hover_pos()
            && item_rect.contains(pointer)
        {
            hovered = Some(item);
            item_gestures(ui, &response, (grid, *index), selected, frame);
        }
    }
    if let Some(item) = hovered {
        response.on_hover_ui(|ui| item_tooltip(ui, item, db, caches));
    }

    if let Some(state) = drag
        && let Some(pointer) = ui.ctx().pointer_latest_pos()
        && rect.contains(pointer)
    {
        frame.candidate = Some(paint_drop_preview(
            &painter, rect, cell, dims, entries, grid, state, pointer, db, caches,
        ));
    }
}

/// The green-fits / red-blocked footprint under a drag.
fn paint_fit_preview(painter: &egui::Painter, preview: egui::Rect, fits: bool) {
    let (fill, stroke) = if fits {
        (
            egui::Color32::from_rgba_unmultiplied(64, 255, 64, 50),
            egui::Color32::from_rgb(64, 255, 64),
        )
    } else {
        (
            egui::Color32::from_rgba_unmultiplied(255, 64, 64, 50),
            egui::Color32::from_rgb(255, 64, 64),
        )
    };
    painter.rect_filled(preview, 2.0, fill);
    painter.rect_stroke(
        preview,
        2.0,
        egui::Stroke::new(2.0, stroke),
        egui::StrokeKind::Inside,
    );
}

/// One item's tile: fill, icon (or initial letter), outline, and
/// stack badge.
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn paint_item_tile(
    ui: &egui::Ui,
    painter: &egui::Painter,
    item_rect: egui::Rect,
    item: &Item,
    is_selected: bool,
    visuals: &egui::Visuals,
    db: Option<&GameCache>,
    caches: &mut Caches,
) {
    let fill = if is_selected {
        visuals.selection.bg_fill
    } else {
        theme::TILE_BG
    };
    painter.rect_filled(item_rect, 2.0, fill);
    if let Some(texture) = caches.icon(ui.ctx(), db, item) {
        painter.image(
            texture.id(),
            item_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        let initial = caches
            .names
            .record_name(db, &item.base)
            .chars()
            .next()
            .unwrap_or('?');
        painter.text(
            item_rect.center(),
            egui::Align2::CENTER_CENTER,
            initial,
            egui::FontId::proportional(12.0),
            visuals.strong_text_color(),
        );
    }
    let outline = if is_selected {
        egui::Stroke::new(2.0, visuals.selection.stroke.color)
    } else {
        egui::Stroke::new(1.0, theme::TILE_EDGE)
    };
    painter.rect_stroke(item_rect, 2.0, outline, egui::StrokeKind::Inside);
    if item.stack_size > 1 {
        painter.text(
            item_rect.right_bottom() - egui::vec2(2.0, 1.0),
            egui::Align2::RIGHT_BOTTOM,
            item.stack_size.to_string(),
            egui::FontId::proportional(10.0),
            visuals.strong_text_color(),
        );
    }
}

/// Snaps the dragged footprint to the hovered cell, paints it green
/// (fits) or red (blocked), and returns the drop candidate.
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn paint_drop_preview(
    painter: &egui::Painter,
    rect: egui::Rect,
    cell_size: f32,
    dims: (i32, i32),
    entries: &[(usize, &Item)],
    grid: GridId,
    state: &DragState,
    cursor: egui::Pos2,
    db: Option<&GameCache>,
    caches: &mut Caches,
) -> DropCandidate {
    let footprint = caches.footprint(db, &state.item);
    let relative = cursor - state.grab - rect.min.to_vec2();
    let cell = univault_core::chr::GridPos {
        x: point_to_cell(relative.x, cell_size, dims.0, footprint.0),
        y: point_to_cell(relative.y, cell_size, dims.1, footprint.1),
    };
    // A matching partial relic/charm under the pointer offers a
    // combine instead of a placement (gold); gear whose family the
    // dragged relic/charm allows offers a socket instead (violet).
    for (index, item) in entries {
        if state.source == grid && state.index == *index {
            continue;
        }
        let combine = transfer::can_combine(db, &state.item, item);
        let socket = !combine && transfer::can_socket(db, &state.item, item);
        if !combine && !socket {
            continue;
        }
        let (width, height) = caches.footprint(db, item);
        let item_rect = egui::Rect::from_min_size(
            rect.min
                + egui::vec2(
                    cells_at(item.position.x, cell_size),
                    cells_at(item.position.y, cell_size),
                ),
            egui::vec2(cells_at(width, cell_size), cells_at(height, cell_size)),
        )
        .shrink(1.0);
        if item_rect.contains(cursor) {
            let color = if combine {
                egui::Color32::from_rgb(255, 200, 40)
            } else {
                egui::Color32::from_rgb(190, 120, 255)
            };
            painter.rect_filled(
                item_rect,
                2.0,
                egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 60),
            );
            painter.rect_stroke(
                item_rect,
                2.0,
                egui::Stroke::new(2.0, color),
                egui::StrokeKind::Inside,
            );
            return DropCandidate {
                grid,
                cell,
                fits: false,
                combine_with: combine.then_some(*index),
                socket_into: socket.then_some(*index),
                equips: false,
            };
        }
    }
    let skip = (state.source == grid).then_some(state.index);
    let occupied: Vec<univault_core::grid::CellRect> = entries
        .iter()
        .filter(|(index, _)| Some(*index) != skip)
        .map(|(_, item)| {
            let (width, height) = caches.footprint(db, item);
            univault_core::grid::CellRect {
                x: item.position.x,
                y: item.position.y,
                width,
                height,
            }
        })
        .collect();
    let fits = transfer::fits_at(&occupied, footprint, cell, dims);
    let preview = egui::Rect::from_min_size(
        rect.min + egui::vec2(cells_at(cell.x, cell_size), cells_at(cell.y, cell_size)),
        egui::vec2(
            cells_at(footprint.0, cell_size),
            cells_at(footprint.1, cell_size),
        ),
    )
    .shrink(1.0);
    paint_fit_preview(painter, preview, fits);
    DropCandidate {
        grid,
        cell,
        fits,
        combine_with: None,
        socket_into: None,
        equips: false,
    }
}

/// Picks an index by cumulative weight from a roll; `None` when
/// every weight is zero or negative.
fn weighted_index(weights: &[i32], roll: u64) -> Option<usize> {
    let clamp = |weight: &i32| u64::try_from((*weight).max(0)).unwrap_or(0);
    let total: u64 = weights.iter().map(clamp).sum();
    if total == 0 {
        return None;
    }
    let mut point = roll % total;
    for (index, weight) in weights.iter().enumerate() {
        let weight = clamp(weight);
        if point < weight {
            return Some(index);
        }
        point -= weight;
    }
    None
}

/// A per-click roll from the wall clock — plenty for picking a game
/// bonus, deliberately not a statistical RNG.
fn entropy_roll() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::from(elapsed.subsec_nanos()) ^ elapsed.as_secs().rotate_left(20)
        })
}

/// The grid cell a point lands in, clamped so the footprint stays
/// inside the container.
#[allow(clippy::cast_possible_truncation)] // grid coordinates are tiny
fn point_to_cell(point: f32, cell_size: f32, grid_cells: i32, footprint_cells: i32) -> i32 {
    let cell = (point / cell_size).round() as i32;
    cell.clamp(0, (grid_cells - footprint_cells).max(0))
}

/// Item details on hover, name colored by rarity. The game's palette
/// assumes its dark backdrop, so the tooltip paints its own instead
/// of inheriting the theme.
fn item_tooltip(ui: &mut egui::Ui, item: &Item, db: Option<&GameCache>, caches: &mut Caches) {
    let item_style = style::item_style(db, item);
    let details = db.map(|db| stats::item_details(db, item));
    let pane_chrome = caches.chrome(ui.ctx(), db);
    let bordered = pane_chrome.is_some();
    let response = egui::Frame::NONE
        .fill(theme::POPUP)
        .stroke(if bordered {
            egui::Stroke::NONE
        } else {
            egui::Stroke::new(1.0, theme::GOLD_DIM)
        })
        .corner_radius(egui::CornerRadius::same(if bordered { 0 } else { 3 }))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.style_mut().visuals.override_text_color = Some(theme::TEXT);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(
                egui::RichText::new(tooltip_title(item, db, details.as_ref(), caches))
                    .color(game_color(style::style_color(item_style)))
                    .size(15.0),
            );
            ui.label(
                egui::RichText::new(item_style.label())
                    .color(theme::TEXT_WEAK)
                    .size(11.0),
            );
            let Some(details) = details else { return };
            for block in &details.blocks {
                ui.add(egui::Separator::default().spacing(6.0));
                for line in block {
                    if line.text.trim().is_empty() {
                        ui.add_space(4.0);
                    } else {
                        ui.label(
                            egui::RichText::new(&line.text)
                                .color(game_color(stats::palette_color(line.color)))
                                .size(12.0),
                        );
                    }
                }
            }
        });
    if let Some(pane_chrome) = pane_chrome {
        pane_chrome.tooltip_frame(ui.painter(), response.response.rect);
    }
}

/// The game's full item name: prefix, quality, base, style, suffix,
/// and the stack count.
fn tooltip_title(
    item: &Item,
    db: Option<&GameCache>,
    details: Option<&stats::ItemDetails>,
    caches: &mut Caches,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(prefix) = &item.prefix {
        parts.push(caches.names.record_name(db, prefix));
    }
    if let Some(quality) = details.and_then(|details| details.quality.clone()) {
        parts.push(quality);
    }
    parts.push(caches.names.record_name(db, &item.base));
    if let Some(style_word) = details.and_then(|details| details.style_word.clone()) {
        parts.push(style_word);
    }
    if let Some(suffix) = &item.suffix {
        parts.push(caches.names.record_name(db, suffix));
    }
    if item.stack_size > 1 {
        parts.push(format!("×{}", item.stack_size));
    }
    parts.join(" ")
}

fn game_color(rgb: style::Rgb) -> egui::Color32 {
    egui::Color32::from_rgb(rgb.r, rgb.g, rgb.b)
}

/// The paper doll's canvas in grid cells.
const DOLL_CELLS: (i32, i32) = (10, 14);

/// `TQVaultAE`'s paper-doll geometry (MIT, `SackCollection.cs`): a
/// slot's top-left cell and size on the doll canvas.
fn doll_geometry(slot: EquipSlot) -> (i32, i32, i32, i32) {
    match slot {
        EquipSlot::Head => (4, 0, 2, 2),
        EquipSlot::Neck => (4, 3, 2, 1),
        EquipSlot::Torso => (4, 5, 2, 3),
        EquipSlot::Legs => (4, 9, 2, 2),
        EquipSlot::Arms => (7, 6, 2, 2),
        EquipSlot::Ring1 => (4, 12, 1, 1),
        EquipSlot::Ring2 => (5, 12, 1, 1),
        EquipSlot::LeftHand => (1, 0, 2, 5),
        EquipSlot::RightHand => (7, 0, 2, 5),
        EquipSlot::LeftHandAlternate => (1, 9, 2, 5),
        EquipSlot::RightHandAlternate => (7, 9, 2, 5),
        EquipSlot::Artifact => (1, 6, 2, 2),
    }
}

/// The worn item's tile inside its slot box: native cell size,
/// centered, downscaled only when it outgrows the box.
fn doll_item_rect(box_rect: egui::Rect, cell: f32, footprint: (i32, i32)) -> egui::Rect {
    let native = egui::vec2(cells_at(footprint.0, cell), cells_at(footprint.1, cell));
    let scale = (box_rect.width() / native.x)
        .min(box_rect.height() / native.y)
        .min(1.0);
    egui::Rect::from_center_size(box_rect.center(), native * scale).shrink(1.0)
}

/// Routes the pointer gestures every item surface shares — click
/// select, Shift/Alt clicks, double-click bonus edit, right-click
/// sends — into the frame. Callers confirm the hover first.
fn item_gestures(
    ui: &egui::Ui,
    response: &egui::Response,
    address: (GridId, usize),
    selected: &mut Option<(GridId, usize)>,
    frame: &mut DragFrame,
) {
    if response.double_clicked() {
        frame.edit_bonus = Some(address);
    } else if response.clicked() {
        let modifiers = ui.ctx().input(|input| input.modifiers);
        if modifiers.shift {
            frame.duplicate = Some(address);
        } else if modifiers.alt {
            frame.extract = Some(address);
        } else {
            *selected = Some(address);
        }
    }
    if response.secondary_clicked() {
        if ui.ctx().input(|input| input.modifiers.shift) {
            frame.copy_across = Some(address);
        } else {
            frame.quick_move = Some(address);
        }
    }
}

/// Drop feedback for one doll slot while a drag is in flight: every
/// legal empty slot glows, and the slot under the cursor shows equip
/// (green), socket-into-worn-gear (violet), or refusal (red),
/// recording the drop candidate for the release.
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn doll_drop_feedback(
    painter: &egui::Painter,
    box_rect: egui::Rect,
    slot: EquipSlot,
    worn: Option<&Item>,
    state: &DragState,
    cursor: Option<egui::Pos2>,
    db: Option<&GameCache>,
    frame: &mut DragFrame,
) {
    let can_wear = worn.is_none() && transfer::can_equip(db, &state.item, slot);
    if can_wear {
        painter.rect_stroke(
            box_rect,
            2.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(64, 255, 64)),
            egui::StrokeKind::Inside,
        );
    }
    if !cursor.is_some_and(|cursor| box_rect.contains(cursor)) {
        return;
    }
    let sockets = worn.is_some_and(|item| transfer::can_socket(db, &state.item, item));
    let color = if sockets {
        egui::Color32::from_rgb(190, 120, 255)
    } else if can_wear {
        egui::Color32::from_rgb(64, 255, 64)
    } else {
        egui::Color32::from_rgb(255, 64, 64)
    };
    painter.rect_filled(
        box_rect,
        2.0,
        egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 60),
    );
    painter.rect_stroke(
        box_rect,
        2.0,
        egui::Stroke::new(2.0, color),
        egui::StrokeKind::Inside,
    );
    if sockets || can_wear {
        frame.candidate = Some(DropCandidate {
            grid: GridId::Equipment(slot),
            cell: univault_core::chr::GridPos { x: 0, y: 0 },
            fits: false,
            combine_with: None,
            socket_into: sockets.then_some(0),
            equips: !sockets,
        });
    }
}

/// The character's worn equipment as an interactive paper doll: slot
/// boxes at `TQVaultAE`'s positions, the same click/drag surface as
/// the grids (select, tooltip, drag out, right-click send, Shift/Alt
/// clicks), plus equipping — while a drag is in flight every legal
/// empty slot glows, and dropping there wears the item. Dropping a
/// relic/charm on worn gear sockets it in place (violet), exactly as
/// in the grids.
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn show_equipment_doll(
    ui: &mut egui::Ui,
    equipment: &univault_core::chr::Equipment,
    selected: &mut Option<(GridId, usize)>,
    db: Option<&GameCache>,
    caches: &mut Caches,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) {
    let (rect, response, cell) = allocate_doll_canvas(ui);
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();
    let doll_chrome = caches.chrome(ui.ctx(), db);
    paint_doll_canvas(&painter, doll_chrome.as_ref(), rect, &visuals);

    let press_origin = ui.ctx().input(|input| input.pointer.press_origin());
    let cursor = ui.ctx().pointer_latest_pos();
    let mut hovered: Option<&Item> = None;

    for slot in EquipSlot::ALL {
        let (x, y, w, h) = doll_geometry(slot);
        let box_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(cells_at(x, cell), cells_at(y, cell)),
            egui::vec2(cells_at(w, cell), cells_at(h, cell)),
        )
        .shrink(1.0);
        let grid = GridId::Equipment(slot);
        paint_slot_inset(&painter, doll_chrome.as_ref(), slot, box_rect);
        if doll_chrome.is_none() {
            painter.rect_stroke(
                box_rect,
                2.0,
                egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
                egui::StrokeKind::Inside,
            );
        }

        match equipment.get(slot) {
            Some(item) => {
                let item_rect = doll_item_rect(box_rect, cell, caches.footprint(db, item));
                let is_selected = *selected == Some((grid, 0));
                paint_item_tile(
                    ui,
                    &painter,
                    item_rect,
                    item,
                    is_selected,
                    &visuals,
                    db,
                    caches,
                );
                if drag.is_some_and(|state| state.source == grid) {
                    painter.rect_filled(item_rect, 2.0, egui::Color32::from_black_alpha(140));
                }
                if response.drag_started()
                    && drag.is_none()
                    && frame.begin.is_none()
                    && press_origin.is_some_and(|origin| box_rect.contains(origin))
                {
                    frame.begin = Some(DragState {
                        source: grid,
                        index: 0,
                        item: item.clone(),
                        grab: press_origin
                            .map_or(egui::Vec2::ZERO, |origin| origin - item_rect.min),
                    });
                }
                if drag.is_none()
                    && let Some(cursor) = response.hover_pos()
                    && box_rect.contains(cursor)
                {
                    hovered = Some(item);
                    item_gestures(ui, &response, (grid, 0), selected, frame);
                }
            }
            None => {
                painter.text(
                    box_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    slot.label(),
                    egui::FontId::proportional(9.0),
                    visuals.weak_text_color(),
                );
            }
        }

        if let Some(state) = drag
            && state.source != grid
        {
            doll_drop_feedback(
                &painter,
                box_rect,
                slot,
                equipment.get(slot),
                state,
                cursor,
                db,
                frame,
            );
        }
    }
    if let Some(item) = hovered {
        response.on_hover_ui(|ui| item_tooltip(ui, item, db, caches));
    }
}

#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn show_character_section(
    ui: &mut egui::Ui,
    pane: &mut CharacterPane,
    db: Option<&GameCache>,
    caches: &mut Caches,
    can_move: bool,
    inventory_tab: &mut InventoryTab,
    selected: &mut Option<(GridId, usize)>,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) -> Option<PaneAction> {
    let mut action = None;
    let pane_chrome = caches.chrome(ui.ctx(), db);
    let pane_chrome = pane_chrome.as_ref();
    ui.horizontal_wrapped(|ui| {
        pane_heading(
            ui,
            pane_chrome,
            pane.character
                .info
                .name
                .as_deref()
                .unwrap_or("Unnamed character"),
        );
        let gold = ui.add(
            egui::DragValue::new(&mut pane.character.info.money)
                .range(0..=i32::MAX)
                .prefix("gold: "),
        );
        if gold.changed() {
            pane.dirty = true;
        }
        let selection_here = matches!(*selected, Some((GridId::Sack(_) | GridId::Equipment(_), _)));
        if plate_button(ui, pane_chrome, can_move && selection_here, "→ Vault").clicked() {
            action = Some(PaneAction::MoveToVault);
        }
        let has_items = pane
            .character
            .sacks
            .iter()
            .any(|sack| !sack.items.is_empty());
        if plate_button(ui, pane_chrome, can_move && has_items, "All → Vault")
            .on_hover_text(
                "Move every item from all sacks into the open vault tab, \
                 spilling into the other tabs as it fills; equipped gear stays on",
            )
            .clicked()
        {
            action = Some(PaneAction::MoveAllToVault);
        }
        if plate_button(ui, pane_chrome, can_move && has_items, "Copy all → Vault")
            .on_hover_text("The same, as copies — every item stays in its sack")
            .clicked()
        {
            action = Some(PaneAction::CopyAllToVault);
        }
        if plate_button(ui, pane_chrome, true, "Respec attributes").clicked() {
            action = Some(PaneAction::PreviewRespec(RespecKind::Attributes));
        }
        if plate_button(ui, pane_chrome, true, "Respec skills & masteries").clicked() {
            action = Some(PaneAction::PreviewRespec(RespecKind::Skills));
        }
    });
    ui.label(theme::path_text(pane.path.display().to_string()));
    if let InventoryTab::Sack(index) = *inventory_tab
        && index >= pane.character.sacks.len()
    {
        *inventory_tab = InventoryTab::Equipment;
    }
    let active = show_inventory_tabs(ui, pane, db, caches, inventory_tab, selected, drag);
    if let Some(chrome_set) = caches.chrome(ui.ctx(), db) {
        ui.add_space(-2.0);
        let inner = egui::Frame::NONE
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                show_inventory_body(ui, pane, db, caches, *inventory_tab, selected, drag, frame);
            });
        chrome::panel_border(ui.painter(), inner.response.rect);
        if let Some((tab_rect, label)) = active {
            chrome::tab_anchor(&chrome_set, ui.painter(), tab_rect, &label, 10.0);
        }
    } else {
        ui.add_space(4.0);
        show_inventory_body(ui, pane, db, caches, *inventory_tab, selected, drag, frame);
    }
    action
}

/// The content the active Inventory sub-tab owns: the doll, or one
/// sack's grid.
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn show_inventory_body(
    ui: &mut egui::Ui,
    pane: &CharacterPane,
    db: Option<&GameCache>,
    caches: &mut Caches,
    inventory_tab: InventoryTab,
    selected: &mut Option<(GridId, usize)>,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) {
    match inventory_tab {
        InventoryTab::Equipment => {
            show_equipment_doll(
                ui,
                &pane.character.equipment,
                selected,
                db,
                caches,
                drag,
                frame,
            );
        }
        InventoryTab::Sack(index) => {
            if let Some(sack) = pane.character.sacks.get(index) {
                let entries: Vec<(usize, &Item)> = sack.items.iter().enumerate().collect();
                grid_view(
                    ui,
                    chr::sack_dimensions(index),
                    &entries,
                    GridId::Sack(index),
                    selected,
                    db,
                    caches,
                    drag,
                    frame,
                );
            }
        }
    }
}

/// Mutable view of the left column's documents and UI state, so the
/// tab strip and the active section render from one place.
struct LeftView<'a> {
    character: &'a mut Option<CharacterPane>,
    bank: &'a mut Option<StashPane>,
    shared: &'a mut Option<StashPane>,
    relics: &'a mut Option<StashPane>,
    active_tab: &'a mut LeftTab,
    inventory_tab: &'a mut InventoryTab,
    selected: &'a mut Option<(GridId, usize)>,
}

impl LeftView<'_> {
    fn loaded(&self, tab: LeftTab) -> bool {
        match tab {
            LeftTab::Inventory => self.character.is_some(),
            LeftTab::Bank => self.bank.is_some(),
            LeftTab::Shared => self.shared.is_some(),
            LeftTab::Relic => self.relics.is_some(),
        }
    }
}

/// The left pane's document strip, rendered above the frame; the
/// active tab's rect and label return for anchoring onto it.
fn show_left_tabs(
    ui: &mut egui::Ui,
    view: &mut LeftView<'_>,
    pane_chrome: Option<&chrome::Chrome>,
) -> Option<(egui::Rect, String)> {
    if !view.loaded(*view.active_tab)
        && let Some(fallback) = LeftTab::ALL.into_iter().find(|tab| view.loaded(*tab))
    {
        *view.active_tab = fallback;
        *view.selected = None;
    }
    let mut active = None;
    ui.horizontal(|ui| {
        for tab in LeftTab::ALL {
            let loaded = view.loaded(tab);
            let response = match pane_chrome {
                Some(pane_chrome) => {
                    pane_chrome.tab(ui, *view.active_tab == tab, loaded, tab.title())
                }
                None => ui.add_enabled(
                    loaded,
                    egui::Button::selectable(*view.active_tab == tab, tab.title()),
                ),
            };
            if *view.active_tab == tab {
                active = Some((response.rect, tab.title().to_owned()));
            }
            let response = if loaded {
                response
            } else {
                response.on_hover_text(tab.missing_hint())
            };
            if loaded && response.clicked() && *view.active_tab != tab {
                *view.active_tab = tab;
                *view.selected = None;
            }
        }
    });
    active
}

/// The left column: the active document's section (the strip above
/// it is [`show_left_tabs`]).
fn show_left_column(
    ui: &mut egui::Ui,
    view: &mut LeftView<'_>,
    db: Option<&GameCache>,
    caches: &mut Caches,
    can_move: bool,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) -> Option<PaneAction> {
    match *view.active_tab {
        LeftTab::Inventory => view.character.as_mut().and_then(|pane| {
            show_character_section(
                ui,
                pane,
                db,
                caches,
                can_move,
                view.inventory_tab,
                view.selected,
                drag,
                frame,
            )
        }),
        LeftTab::Bank => view.bank.as_mut().and_then(|pane| {
            show_stash_section(
                ui,
                pane,
                StashSlot::Bank,
                db,
                caches,
                can_move,
                view.selected,
                drag,
                frame,
            )
        }),
        LeftTab::Shared => view.shared.as_mut().and_then(|pane| {
            show_stash_section(
                ui,
                pane,
                StashSlot::Shared,
                db,
                caches,
                can_move,
                view.selected,
                drag,
                frame,
            )
        }),
        LeftTab::Relic => view.relics.as_mut().and_then(|pane| {
            show_stash_section(
                ui,
                pane,
                StashSlot::Relic,
                db,
                caches,
                can_move,
                view.selected,
                drag,
                frame,
            )
        }),
    }
}

#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn show_stash_section(
    ui: &mut egui::Ui,
    pane: &mut StashPane,
    slot: StashSlot,
    db: Option<&GameCache>,
    caches: &mut Caches,
    can_move: bool,
    selected: &mut Option<(GridId, usize)>,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) -> Option<PaneAction> {
    let mut action = None;
    let (title, grid) = match slot {
        StashSlot::Bank => ("Character bank", GridId::Bank),
        StashSlot::Shared => ("Shared bank", GridId::Shared),
        StashSlot::Relic => ("Relic bank", GridId::Relic),
    };
    let pane_chrome = caches.chrome(ui.ctx(), db);
    let pane_chrome = pane_chrome.as_ref();
    ui.horizontal_wrapped(|ui| {
        pane_heading(
            ui,
            pane_chrome,
            &format!("{title} {}×{}", pane.stash.width, pane.stash.height),
        );
        let selection_here = matches!(*selected, Some((current, _)) if current == grid);
        if plate_button(ui, pane_chrome, can_move && selection_here, "→ Vault").clicked() {
            action = Some(PaneAction::MoveToVault);
        }
        let has_items = !pane.stash.items.is_empty();
        if plate_button(ui, pane_chrome, can_move && has_items, "All → Vault")
            .on_hover_text(
                "Move every item in this bank into the open vault tab, \
                 spilling into the other tabs as it fills",
            )
            .clicked()
        {
            action = Some(PaneAction::MoveAllToVault);
        }
        if plate_button(ui, pane_chrome, can_move && has_items, "Copy all → Vault")
            .on_hover_text("The same, as copies — every item stays in the bank")
            .clicked()
        {
            action = Some(PaneAction::CopyAllToVault);
        }
    });
    ui.label(theme::path_text(pane.path.display().to_string()));
    let entries: Vec<(usize, &Item)> = pane.stash.items.iter().enumerate().collect();
    grid_view(
        ui,
        (pane.stash.width, pane.stash.height),
        &entries,
        grid,
        selected,
        db,
        caches,
        drag,
        frame,
    );
    action
}

#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
/// The vault's numbered tab strip, rendered above the frame; the
/// active tab's rect and label return for anchoring onto it.
/// Mid-drag, pointing at a tab switches to it so any tab can
/// receive the drop; egui suppresses hover while a widget drags, so
/// the check is by pointer position.
fn show_vault_tabs(
    ui: &mut egui::Ui,
    pane: &mut VaultPane,
    pane_chrome: Option<&chrome::Chrome>,
    drag: Option<&DragState>,
) -> Option<(egui::Rect, String)> {
    if pane.open_tab >= pane.vault.sacks.len() {
        pane.open_tab = 0;
    }
    let cursor = ui.ctx().pointer_latest_pos();
    let mut active = None;
    ui.horizontal_wrapped(|ui| {
        for (tab, sack) in pane.vault.sacks.iter().enumerate() {
            let label = if sack.items.is_empty() {
                format!("{}", tab + 1)
            } else {
                format!("{} ({})", tab + 1, sack.items.len())
            };
            let response = match pane_chrome {
                Some(pane_chrome) => pane_chrome.tab(ui, pane.open_tab == tab, true, &label),
                None => ui.add(egui::Button::selectable(
                    pane.open_tab == tab,
                    label.clone(),
                )),
            };
            if pane.open_tab == tab {
                active = Some((response.rect, label));
            }
            let drag_over =
                drag.is_some() && cursor.is_some_and(|cursor| response.rect.contains(cursor));
            if (response.clicked() || drag_over) && pane.open_tab != tab {
                pane.open_tab = tab;
                pane.selected = None;
            }
        }
    });
    active
}

fn show_vault_pane(
    ui: &mut egui::Ui,
    pane: &mut VaultPane,
    db: Option<&GameCache>,
    caches: &mut Caches,
    can_move: bool,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) -> Option<PaneAction> {
    let mut action = None;
    let pane_chrome = caches.chrome(ui.ctx(), db);
    let pane_chrome = pane_chrome.as_ref();
    ui.horizontal(|ui| {
        pane_heading(ui, pane_chrome, "Vault");
        if plate_button(
            ui,
            pane_chrome,
            can_move && pane.selected.is_some(),
            "← To file",
        )
        .clicked()
        {
            action = Some(PaneAction::MoveToFile);
        }
        if plate_button(ui, pane_chrome, db.is_some(), "Filter…")
            .on_hover_text("Search and filter every vault file (⌘F / Ctrl+F)")
            .clicked()
        {
            action = Some(PaneAction::OpenSearch);
        }
    });
    ui.label(theme::path_text(pane.path.display().to_string()));
    let Some(sack) = pane.vault.sacks.get(pane.open_tab) else {
        ui.weak("This vault file has no tabs.");
        return action;
    };
    egui::ScrollArea::vertical()
        .id_salt("vault-pane")
        .show(ui, |ui| {
            let entries: Vec<(usize, &Item)> = sack
                .items
                .iter()
                .enumerate()
                .map(|(index, entry)| (index, &entry.item))
                .collect();
            grid_view(
                ui,
                (
                    univault_core::vault::TAB_WIDTH,
                    univault_core::vault::TAB_HEIGHT,
                ),
                &entries,
                GridId::VaultTab(pane.open_tab),
                &mut pane.selected,
                db,
                caches,
                drag,
                frame,
            );
        });
    action
}

fn pick_file(description: &str, extensions: &[&str], start: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().add_filter(description, extensions);
    if let Some(start) = start {
        dialog = dialog.set_directory(start);
    }
    dialog.pick_file()
}

fn first_dropped_path(ctx: &egui::Context) -> Option<PathBuf> {
    ctx.input(|input| {
        input
            .raw
            .dropped_files
            .first()
            .map(|file| file.path().to_path_buf())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(raw: &[&str]) -> CliArgs {
        CliArgs::from_args(raw.iter().map(std::ffi::OsString::from))
    }

    fn stamp(size: i64, mtime: i64) -> SourceStamp {
        SourceStamp {
            path: "watched".to_string(),
            size,
            mtime_seconds: mtime,
        }
    }

    #[test]
    fn refresh_settles_only_after_two_matching_observations() {
        let mut tracker = RefreshTracker::default();
        let path = Path::new("watched");
        let ours = stamp(10, 100);
        let changed = stamp(20, 200);
        // Matching the pane's stamp never fires.
        assert!(!tracker.settled(path, Some(&ours), Some(&ours)));
        // First sighting of a change arms; the second, identical
        // sighting settles.
        assert!(!tracker.settled(path, Some(&changed), Some(&ours)));
        assert!(tracker.settled(path, Some(&changed), Some(&ours)));
        // Once the pane catches up, the pending state clears.
        assert!(!tracker.settled(path, Some(&changed), Some(&changed)));
        assert!(!tracker.settled(path, Some(&changed), Some(&changed)));
    }

    #[test]
    fn refresh_never_acts_on_a_file_still_being_written() {
        let mut tracker = RefreshTracker::default();
        let path = Path::new("watched");
        let ours = stamp(10, 100);
        // A file growing between polls (mid-write) keeps re-arming.
        assert!(!tracker.settled(path, Some(&stamp(20, 200)), Some(&ours)));
        assert!(!tracker.settled(path, Some(&stamp(30, 201)), Some(&ours)));
        assert!(!tracker.settled(path, Some(&stamp(40, 202)), Some(&ours)));
        // Only stability fires.
        assert!(tracker.settled(path, Some(&stamp(40, 202)), Some(&ours)));
    }

    #[test]
    fn refresh_ignores_unreachable_files() {
        let mut tracker = RefreshTracker::default();
        let path = Path::new("watched");
        let ours = stamp(10, 100);
        let changed = stamp(20, 200);
        assert!(!tracker.settled(path, Some(&changed), Some(&ours)));
        // The mount dropping mid-detection resets everything.
        assert!(!tracker.settled(path, None, Some(&ours)));
        assert!(!tracker.settled(path, Some(&changed), Some(&ours)));
        assert!(tracker.settled(path, Some(&changed), Some(&ours)));
    }

    #[test]
    fn cli_args_route_flags_and_file() {
        let parsed = args(&["--game", "/tq", "--vault", "v.json", "save/Player.chr"]);
        assert_eq!(parsed.game_dir, Some(PathBuf::from("/tq")));
        assert_eq!(parsed.vault, Some(PathBuf::from("v.json")));
        assert_eq!(parsed.file, Some(PathBuf::from("save/Player.chr")));
        let empty = args(&[]);
        assert_eq!(empty.game_dir, None);
        assert_eq!(empty.vault, None);
        assert_eq!(empty.file, None);
        // A dangling flag consumes nothing and breaks nothing.
        assert_eq!(args(&["--game"]).game_dir, None);
    }

    #[test]
    fn weighted_index_walks_cumulative_weights() {
        let weights = [900, 300, 300];
        assert_eq!(weighted_index(&weights, 0), Some(0));
        assert_eq!(weighted_index(&weights, 899), Some(0));
        assert_eq!(weighted_index(&weights, 900), Some(1));
        assert_eq!(weighted_index(&weights, 1199), Some(1));
        assert_eq!(weighted_index(&weights, 1200), Some(2));
        // The roll wraps around the total.
        assert_eq!(weighted_index(&weights, 1500), Some(0));
    }

    #[test]
    fn weighted_index_skips_nonpositive_weights() {
        assert_eq!(weighted_index(&[0, 5, -3], 0), Some(1));
        assert_eq!(weighted_index(&[0, 0], 7), None);
        assert_eq!(weighted_index(&[], 7), None);
    }

    #[test]
    fn stash_files_route_by_their_folder() {
        assert!(matches!(
            stash_slot_for(Path::new("/saves/SaveData/Sys/winsys.dxb")),
            StashSlot::Shared
        ));
        assert!(matches!(
            stash_slot_for(Path::new("/saves/SaveData/Main/_Pally Don/winsys.dxb")),
            StashSlot::Bank
        ));
        assert!(matches!(
            stash_slot_for(Path::new("winsys.dxb")),
            StashSlot::Bank
        ));
        assert!(matches!(
            stash_slot_for(Path::new("/saves/SaveData/Sys/miscsys.dxb")),
            StashSlot::Relic
        ));
        assert!(matches!(
            stash_slot_for(Path::new("MISCSYS.DXG")),
            StashSlot::Relic
        ));
    }

    #[test]
    fn default_vault_lives_under_the_config_dir() {
        let path = default_vault_path().expect("a config dir on a supported platform");
        assert!(path.ends_with("vaults/Main Vault.json"), "{path:?}");
        assert!(
            path.starts_with(univault_core::platform::config_dir().unwrap()),
            "{path:?}"
        );
    }

    #[test]
    fn recents_labels_hide_the_player_chr_boilerplate() {
        let label = |raw: &str| Recents::label(Path::new(raw));
        assert_eq!(label("/saves/_Pally Don/Player.chr"), "Pally Don");
        assert_eq!(
            label("/saves/_Pally Don/winsys.dxb"),
            "Pally Don — winsys.dxb"
        );
        assert_eq!(label("vault.json"), " — vault.json");
    }

    #[test]
    fn tooltip_title_assembles_name_particles() {
        let mut caches = Caches::default();
        let mut item = Item::bare(
            RecordId::parse("records\\item\\sword.dbr".to_string()).unwrap(),
            univault_core::chr::ItemSeed::new(1),
        );
        item.prefix = Some(RecordId::parse("records\\item\\sharp.dbr".to_string()).unwrap());
        item.stack_size = 3;
        // Without game data, names fall back to record file stems.
        assert_eq!(
            tooltip_title(&item, None, None, &mut caches),
            "sharp sword ×3"
        );
        item.prefix = None;
        item.stack_size = 1;
        assert_eq!(tooltip_title(&item, None, None, &mut caches), "sword");
    }

    #[test]
    #[allow(clippy::float_cmp)] // quarters are exact in f32
    fn fraction_is_zero_safe() {
        assert_eq!(fraction(0, 0), 0.0);
        assert_eq!(fraction(1, 4), 0.25);
        assert_eq!(fraction(4, 4), 1.0);
    }
}
