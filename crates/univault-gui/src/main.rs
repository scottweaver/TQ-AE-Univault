//! egui/eframe front-end for tq-univault.
//!
//! Usage: `univault-gui [--game <TQ install dir>] [--vault <vault.json>] [file]`
//!
//! Left pane: one tab per document — the character's inventory
//! (`Player.chr`) with, discovered automatically beside it, the
//! character's bank (its `winsys.dxb`) and the account's shared and
//! relic banks (`SaveData/Sys/winsys.dxb` and `miscsys.dxb`). Right
//! pane: the unified vault store — one file under the config
//! directory, opened (and created) at launch; its tabs are computed
//! type buckets (family plates over a sub-type strip), never stored
//! structure, so dropping an item anywhere in the pane files it by
//! type. `TQVaultAE` vault files are interchange: `Import vault…`
//! (and `--vault`) pulls one in, `Export…` packs items back out.
//! Click or drag items across; right-click sends an item straight to
//! the other pane; Shift+Right-click sends a copy; Shift+Click
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
mod sort;
mod theme;
mod ui_state;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use serde::{Deserialize, Serialize};
use sort::SortDirection;
use univault_core::cache::{GameCache, SourceStamp};
use univault_core::chr::{self, EquipSlot, Item, PlayerCharacter, RecordId};
use univault_core::gamedata::GameData;
use univault_core::respec;
use univault_core::stash::{self, Stash};
use univault_core::stats;
use univault_core::store::{
    Bucket, DuplicateGuard, Family, ImportRecord, StoredItemId, VaultStore,
};
use univault_core::style;
use univault_core::transfer;
use univault_core::vault::Vault;
use univault_gui::components::gilded_border::GildedBorder;
use univault_gui::components::tabbed_panel::{self, TabbedPanel};

fn main() -> eframe::Result {
    let args = CliArgs::parse();
    let (game, game_note) = initial_game_status(args.game_dir.clone());
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 900.0])
        .with_min_inner_size([800.0, 600.0]);
    if let Some(icon) = app_icon(&game) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "TQ UniVault",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            cc.egui_ctx
                .all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
            Ok(Box::new(App::new(args, game, game_note, &cc.egui_ctx)))
        }),
    )
}

/// The record whose inventory art is the app's icon: the Iron Great
/// Helm, Corinthian style (`c04_helm06`).
const APP_ICON_RECORD: &str = r"records\item\equipmenthelm\c04_helm06.dbr";

/// The window (and, on macOS, dock) icon, taken from the game
/// cache's copy of the helm art rather than shipped with the app —
/// extracted game assets stay local (ARCHITECTURE.md). Before the
/// first import there is no cache, and the platform's default icon
/// stands in.
fn app_icon(game: &GameStatus) -> Option<egui::IconData> {
    let GameStatus::Loaded(cache) = game else {
        return None;
    };
    let id = RecordId::parse(APP_ICON_RECORD.to_string())?;
    let image = cache.record_icon(&id)?;
    Some(egui::IconData {
        rgba: image.pixels,
        width: u32::try_from(image.width).ok()?,
        height: u32::try_from(image.height).ok()?,
    })
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

#[derive(Clone)]
struct CharacterPane {
    path: PathBuf,
    original: Vec<u8>,
    character: Box<PlayerCharacter>,
    dirty: bool,
    /// The file's identity when we last read or wrote it; a live
    /// stamp that differs means someone else changed the file.
    disk_stamp: Option<SourceStamp>,
}

#[derive(Clone)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
enum LeftTab {
    #[default]
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

/// What the store pane orders each bucket's items *by*; which way it
/// reads is [`SortDirection`], carried alongside in [`StoreSort`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum StoreSortKey {
    Name,
    Rarity,
    Level,
}

impl StoreSortKey {
    const ALL: [Self; 3] = [Self::Name, Self::Rarity, Self::Level];

    fn label(self) -> &'static str {
        match self {
            Self::Name => "By name",
            Self::Rarity => "By rarity",
            Self::Level => "By level",
        }
    }

    /// The direction a freshly picked key reads best in — names run
    /// A→Z, but "best first" is what anyone picking rarity or level
    /// means. The toggle overrides it and then stays put.
    fn natural(self) -> SortDirection {
        match self {
            Self::Name => SortDirection::Ascending,
            Self::Rarity | Self::Level => SortDirection::Descending,
        }
    }
}

/// The store pane's full ordering: a key and the way it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreSort {
    key: StoreSortKey,
    direction: SortDirection,
}

impl Default for StoreSort {
    fn default() -> Self {
        Self::by(StoreSortKey::Name)
    }
}

impl StoreSort {
    fn by(key: StoreSortKey) -> Self {
        Self {
            key,
            direction: key.natural(),
        }
    }
}

struct StorePane {
    path: PathBuf,
    store: VaultStore,
    dirty: bool,
    selected: Option<ItemAddr>,
    disk_stamp: Option<SourceStamp>,
    view: StoreView,
}

/// How the store pane is being looked at — the open bucket (its
/// family follows from it), the ordering, and the filters in force.
/// Pure view state over a store that has no tab structure, and the
/// part of the pane that survives a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct StoreView {
    bucket: Bucket,
    sort: StoreSort,
    /// Gates bulk sends only: with it set, `All → Store` and the
    /// per-sack buttons skip an item whose seed is already stored in
    /// its type. Single sends and drops are deliberate acts and
    /// always land.
    skip_duplicate_seeds: bool,
    slot_filter: SlotFilter,
}

impl Default for StoreView {
    fn default() -> Self {
        Self {
            bucket: Family::Armor.buckets()[0],
            sort: StoreSort::default(),
            skip_duplicate_seeds: false,
            slot_filter: SlotFilter::default(),
        }
    }
}

impl StoreView {
    fn family(&self) -> Family {
        self.bucket.family()
    }
}

/// The equipment families the Relic and Charm buckets are narrowed
/// to. Empty is no narrowing; otherwise a piece shows when the game's
/// own allow-flags admit it to *any* chosen family, so lighting
/// Helmet and Torso lists everything that fits either.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SlotFilter {
    /// Kept in [`style::GearSlot::ALL`] order so the file reads like
    /// the chip row.
    slots: Vec<style::GearSlot>,
}

impl SlotFilter {
    fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    fn contains(&self, slot: style::GearSlot) -> bool {
        self.slots.contains(&slot)
    }

    fn toggle(&mut self, slot: style::GearSlot) {
        if self.contains(slot) {
            self.slots.retain(|chosen| *chosen != slot);
        } else {
            self.slots.push(slot);
            self.slots.sort_by_key(|chosen| {
                style::GearSlot::ALL
                    .iter()
                    .position(|other| other == chosen)
            });
        }
    }

    fn clear(&mut self) {
        self.slots.clear();
    }

    /// Whether a relic/charm passes: with nothing chosen everything
    /// does; otherwise it must fit at least one chosen family.
    fn admits(&self, db: Option<&GameCache>, item: &Item) -> bool {
        self.is_empty()
            || db.is_some_and(|db| {
                self.slots
                    .iter()
                    .any(|slot| db.relic_allows(&item.base, *slot))
            })
    }
}

/// The buckets the slot filter applies to — the socketable pieces.
fn takes_slot_filter(bucket: Bucket) -> bool {
    use univault_core::query::ItemCategory;
    matches!(
        bucket,
        Bucket::Category(ItemCategory::Relic | ItemCategory::Charm)
    )
}

/// A pane held aside across a reload, so a read that turns out to be
/// a half-written game save can be undone.
enum PaneSnapshot {
    Character(Box<CharacterPane>),
    Stash(StashSlot, Box<StashPane>),
}

/// One open document, addressable across panes — the unit the
/// auto-refresh watcher reloads or reports conflicts on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DocId {
    Character,
    Stash(StashSlot),
    Store,
}

impl DocId {
    const FIXED: [Self; 5] = [
        Self::Character,
        Self::Stash(StashSlot::Bank),
        Self::Stash(StashSlot::Shared),
        Self::Stash(StashSlot::Relic),
        Self::Store,
    ];
}

/// Decides when an externally observed file state is worth acting
/// on: a change must hold steady across two consecutive polls so a
/// file caught mid-write (the game saves over SMB) is never read
/// half-written — and, when a settled file still reads short or
/// corrupt, how long to keep quiet about it while the write lands.
#[derive(Default)]
struct RefreshTracker {
    pending: HashMap<PathBuf, SourceStamp>,
    /// Consecutive failed reloads per path since the last success.
    failures: HashMap<PathBuf, u32>,
    /// Consecutive emptying reads held back per game-owned file,
    /// while the evidence still says a save is in flight.
    empty_defers: HashMap<PathBuf, u32>,
}

/// What a failed auto-reload means for the user: nothing yet — a
/// file whose stamp held still can still read short over SMB while
/// the game's write lands, and the next settle retries — or a
/// failure that has outlasted any write and deserves a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReloadFailure {
    Transient,
    Persisting,
}

/// Failed reloads of one file tolerated silently before one is
/// reported; attempts are two polls apart, so this is ~12 s of
/// patience.
const RELOAD_PATIENCE: u32 = 3;

/// Emptying reads of one file held back while the evidence says a
/// save is in flight; attempts are two polls apart, so this caps the
/// protection at roughly 20 s.
const EMPTY_PATIENCE: u32 = 5;

/// Whether auto-refresh is actually running, and the worst stall it
/// has seen. The watcher thread polls on its own cadence but only the
/// paint loop consumes it, so a fully hidden window (macOS stops
/// repainting one) or a held pointer suspends refreshing with nothing
/// on screen to say so. Recording the gap makes that diagnosable
/// after the fact instead of a guess.
#[derive(Default)]
struct WatchHealth {
    last_checked: Option<Instant>,
    longest_stall: Option<Duration>,
}

/// A gap between consumed polls longer than this means the paint loop
/// stopped, not that a poll ran late — the watcher's cadence is
/// [`WATCH_INTERVAL`].
const STALL_THRESHOLD: Duration = Duration::from_secs(10);

impl WatchHealth {
    /// Records a consumed poll, remembering the worst gap so the
    /// shell can report a stall that has already ended.
    fn checked(&mut self, now: Instant) {
        if let Some(previous) = self.last_checked {
            let gap = now.saturating_duration_since(previous);
            if gap >= STALL_THRESHOLD && self.longest_stall.is_none_or(|worst| gap > worst) {
                self.longest_stall = Some(gap);
            }
        }
        self.last_checked = Some(now);
    }

    /// One line for the toolbar: how long since a poll landed, plus
    /// the worst stall once there has been one worth reporting.
    fn summary(&self, now: Instant) -> String {
        let Some(last) = self.last_checked else {
            return "watching: starting…".to_string();
        };
        let ago = brief_duration(now.saturating_duration_since(last));
        match self.longest_stall {
            Some(stall) => format!(
                "watching: checked {ago} ago · longest pause {} (window hidden?)",
                brief_duration(stall)
            ),
            None => format!("watching: checked {ago} ago"),
        }
    }

    /// Whether the current gap already looks like a stall — the
    /// toolbar line goes warm so a live stall reads at a glance.
    fn stalled_now(&self, now: Instant) -> bool {
        self.last_checked
            .is_none_or(|last| now.saturating_duration_since(last) >= STALL_THRESHOLD)
    }
}

fn brief_duration(span: Duration) -> String {
    let seconds = span.as_secs();
    if seconds < 60 {
        return format!("{seconds}s");
    }
    format!("{}m{:02}s", seconds / 60, seconds % 60)
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

    /// Drops the pending observation for a freshly (re)opened file.
    /// The failure count is deliberately kept: every reload attempt
    /// opens the file, and clearing it here would reset the count
    /// before it could ever reach [`RELOAD_PATIENCE`].
    fn forget(&mut self, path: &Path) {
        self.pending.remove(path);
    }

    fn reload_failed(&mut self, path: &Path) -> ReloadFailure {
        let count = self.failures.entry(path.to_path_buf()).or_insert(0);
        *count += 1;
        if *count >= RELOAD_PATIENCE {
            ReloadFailure::Persisting
        } else {
            ReloadFailure::Transient
        }
    }

    fn reload_succeeded(&mut self, path: &Path) {
        self.failures.remove(path);
    }

    /// Whether an emptying read should be held back rather than
    /// clearing the pane. A game save caught between truncating and
    /// writing its items parses as a perfectly valid empty stash, so
    /// nothing fails and [`RELOAD_PATIENCE`] never fires; the caller
    /// supplies the evidence that a write is still in flight and this
    /// bounds how long that evidence is believed, so a twin that goes
    /// permanently stale cannot pin a genuinely emptied bank forever.
    fn defer_empty(&mut self, path: &Path) -> bool {
        let count = self.empty_defers.entry(path.to_path_buf()).or_insert(0);
        *count += 1;
        *count <= EMPTY_PATIENCE
    }

    /// Forgets the deferrals once a read is adopted, so the *next*
    /// emptying is questioned too rather than waved through.
    fn empty_settled(&mut self, path: &Path) {
        self.empty_defers.remove(path);
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
    store: Option<StorePane>,
    /// The one selected item across all left-side grids (sacks and
    /// the banks); the store keeps its own so cross-pane moves can
    /// aim at the other pane's last selection.
    left_selected: Option<ItemAddr>,
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
    /// How auto-refresh is actually faring — the watcher runs off the
    /// paint loop, so a hidden window silently stops consuming it.
    watch_health: WatchHealth,
    /// Whether the gold field holds keyboard focus, reported by the
    /// character pane each frame. The one focusable widget bound to
    /// document state, so it is the only one auto-refresh defers to —
    /// and only for the character.
    editing_gold: bool,
    /// Documents whose file changed on disk while they hold unsaved
    /// edits — resolved by the conflict modal, never silently.
    conflicts: Vec<DocId>,
    /// Which surface fills the store column: the type-bucket pane, or
    /// the search table in its place.
    view: MainView,
    search: search::SearchState,
    /// The restart-persistent view state and its file.
    ui_state: ui_state::UiStateFile,
    /// The hand-drawn component chrome (bundled art, uploaded once).
    tabbed_panel: TabbedPanel,
    gilded_border: GildedBorder,
}

/// The store column's surface — the game-file pane stays put either
/// way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
enum MainView {
    #[default]
    Panes,
    Search,
}

/// The socket-patch modal's working state: the dll it read and what
/// the bytes said.
struct DllPatchDialog {
    path: PathBuf,
    outcome: Result<(Vec<u8>, univault_core::dllpatch::PatchState), String>,
}

/// Where the item database comes from at launch: `--game` forces a
/// (re-)import; otherwise the local cache is the runtime database,
/// imported automatically (in the background) from the remembered
/// game dir when it is missing or in an older format. The second
/// value is the standing advisory about the cache, if any.
fn initial_game_status(forced_dir: Option<PathBuf>) -> (GameStatus, Option<String>) {
    if let Some(dir) = forced_dir {
        return (GameStatus::Importing(start_import(dir)), None);
    }
    if let Some(cache) = load_cached_game_data() {
        let note = staleness_warning(&cache);
        return (GameStatus::Loaded(cache), note);
    }
    match stored_game_dir() {
        Some(dir) => (GameStatus::Importing(start_import(dir)), None),
        None => (GameStatus::Absent, None),
    }
}

impl App {
    fn new(
        args: CliArgs,
        game: GameStatus,
        game_note: Option<String>,
        ctx: &egui::Context,
    ) -> Self {
        let ui_state = ui_state::UiStateFile::load();
        let restored = ui_state.on_disk().clone();
        // The search table needs the database; without one the pane
        // is the only surface worth restoring.
        let view = match game {
            GameStatus::Loaded(_) => restored.view,
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => {
                MainView::Panes
            }
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
            store: None,
            left_selected: None,
            active_tab: restored.left_tab,
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
            inventory_tab: restored.inventory_tab,
            watcher: start_watcher(),
            refresh: RefreshTracker::default(),
            watch_health: WatchHealth::default(),
            editing_gold: false,
            conflicts: Vec::new(),
            view,
            search: search::SearchState::with_settings(restored.search),
            ui_state,
            tabbed_panel: TabbedPanel::load(ctx),
            gilded_border: GildedBorder::load(ctx),
        };
        app.status = Some(app.open_store());
        if let Some(path) = args.vault {
            app.status = Some(app.import_vault_file(&path));
        }
        if let Some(path) = args.file {
            app.status = Some(app.open(&path));
        } else if let Some(last) = app
            .recents
            .entries
            .iter()
            .find(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("chr"))
            })
            .cloned()
        {
            app.status = Some(app.open(&last));
        }
        app
    }

    /// Routes a path into the matching pane by extension.
    fn open(&mut self, path: &Path) -> Result<String, String> {
        let extension = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
        match extension.as_deref() {
            Some("json") => self.import_vault_file(path),
            Some("vault") => self.import_legacy_vault_file(path),
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

    /// Opens the unified store — the one authoritative file for
    /// vaulted items — creating it on first launch so storage exists
    /// without any setup. Legacy `TQVaultAE` vaults sitting in the
    /// vaults folder are migrated in once, on that first creation.
    fn open_store(&mut self) -> Result<String, String> {
        let path = store_path().ok_or("no config directory on this platform")?;
        self.backed_up.remove(&path);
        self.refresh.forget(&path);
        let disk_stamp = stamp_of(&path);
        let kept = self.store.as_ref().filter(|pane| pane.path == path);
        let view = kept.map_or_else(
            || self.ui_state.on_disk().store.clone(),
            |pane| pane.view.clone(),
        );
        let existing = path.exists();
        let store = if existing {
            let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
            VaultStore::from_json(&text).map_err(|error| error.to_string())?
        } else {
            VaultStore::new()
        };
        self.store = Some(StorePane {
            path: path.clone(),
            store,
            dirty: false,
            selected: None,
            disk_stamp,
            view,
        });
        self.search.mark_data_changed();
        if existing {
            return Ok(format!("opened {}", path.display()));
        }
        let migrated = self.migrate_legacy_vaults();
        let pane = self.store.as_mut().expect("just set");
        pane.dirty = true;
        Ok(match migrated {
            0 => format!("new item store — will be created at {}", path.display()),
            count => format!(
                "new item store at {} — migrated {} from the vaults folder",
                path.display(),
                count_items(count)
            ),
        })
    }

    /// One-time migration on store creation: every `TQVaultAE` vault
    /// in the vaults folder is read in, its provenance recorded so a
    /// later rescan never double-imports, and the files themselves are
    /// left untouched.
    fn migrate_legacy_vaults(&mut self) -> usize {
        let Some(pane) = self.store.as_mut() else {
            return 0;
        };
        let mut migrated = 0;
        for path in vault_files_in_vaults_dir() {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(vault) = Vault::from_json(&text) else {
                continue;
            };
            let Some(source) = import_source_name(&path) else {
                continue;
            };
            if pane.store.is_imported(&source) {
                continue;
            }
            let count = pane.store.import_vault(&vault);
            pane.store
                .record_import(import_record(&path, source, count));
            migrated += count;
        }
        migrated
    }

    /// Imports one `TQVaultAE` vault file into the store. The file is
    /// only ever read — interchange, never a second authoritative
    /// store — and re-importing the same file is the user's call to
    /// make, so it simply adds again.
    fn import_vault_file(&mut self, path: &Path) -> Result<String, String> {
        let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let vault = Vault::from_json(&text).map_err(|error| error.to_string())?;
        self.absorb_vault(path, &vault)
    }

    /// Imports a legacy binary `.vault` file into the store; the
    /// binary original is never written.
    fn import_legacy_vault_file(&mut self, path: &Path) -> Result<String, String> {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let vault = Vault::from_legacy_binary(&bytes).map_err(|error| error.to_string())?;
        self.absorb_vault(path, &vault)
    }

    fn absorb_vault(&mut self, path: &Path, vault: &Vault) -> Result<String, String> {
        let pane = self.store.as_mut().ok_or("no item store open")?;
        let count = pane.store.import_vault(vault);
        if let Some(source) = import_source_name(path) {
            pane.store.record_import(import_record(path, source, count));
        }
        pane.dirty = true;
        self.search.mark_data_changed();
        Ok(format!(
            "imported {} from {}",
            count_items(count),
            path.display()
        ))
    }

    /// Exports the store (or just the open bucket) as a fresh
    /// `TQVaultAE`-readable vault file: a snapshot for the other tool,
    /// never a live mirror.
    fn export_vault_file(&mut self, path: &Path, whole_store: bool) -> Result<String, String> {
        let db = loaded_db(&self.game);
        let pane = self.store.as_ref().ok_or("no item store open")?;
        let items: Vec<Item> = if whole_store {
            pane.store
                .entries()
                .map(|entry| entry.item.clone())
                .collect()
        } else {
            let bucket = pane.view.bucket;
            pane.store
                .entries()
                .filter(|entry| univault_core::store::bucket_of(db, &entry.item) == bucket)
                .map(|entry| entry.item.clone())
                .collect()
        };
        if items.is_empty() {
            return Err("nothing to export".to_string());
        }
        let count = items.len();
        let vault = univault_core::store::export_to_vault(items, db);
        let json = vault.to_json().map_err(|error| error.to_string())?;
        safe_write::backup_first_write(path, json.as_bytes()).map_err(|error| error.to_string())?;
        Ok(format!(
            "exported {} to {}",
            count_items(count),
            path.display()
        ))
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
        }
        .ok_or_else(|| "selection is stale — pick the item again".to_string())
    }

    fn move_left_to_store(&mut self) -> Result<String, String> {
        let Some(ItemAddr::Grid { grid, index }) = self.left_selected else {
            return Err("select an item on the left".to_string());
        };
        self.send_left_to_store(grid, index)
    }

    /// Sends one left-side item into the store. The store is
    /// unbounded and files by type, so — unlike the old fixed vault
    /// tabs — this cannot fail for want of room.
    fn send_left_to_store(&mut self, grid: GridId, index: usize) -> Result<String, String> {
        if self.store.is_none() {
            return Err("no item store open".to_string());
        }
        let item = self.take_from_left(grid, index)?;
        let label = self.caches.names.item_label(loaded_db(&self.game), &item);
        let bucket = univault_core::store::bucket_of(loaded_db(&self.game), &item);
        let pane = self.store.as_mut().expect("checked above");
        pane.store.add(item);
        pane.dirty = true;
        self.mark_left_dirty(grid);
        self.left_selected = None;
        self.search.mark_data_changed();
        Ok(format!("{label} → {}", bucket.label()))
    }

    /// Sends every item of the active left tab into the store: the
    /// whole document, or — for the Inventory — one sack when `sack`
    /// names it. Equipped gear stays worn. Nothing is ever left
    /// behind for want of room; the store has no capacity. The
    /// store pane's duplicate box is consulted here and only here —
    /// with it set, an item whose seed is already stored in its type
    /// is passed over and stays put.
    fn bulk_left_to_store(
        &mut self,
        mode: BulkMode,
        sack: Option<usize>,
    ) -> Result<String, String> {
        if self.store.is_none() {
            return Err("no item store open".to_string());
        }
        let db = loaded_db(&self.game);
        let mut guard = self
            .store
            .as_ref()
            .filter(|pane| pane.view.skip_duplicate_seeds)
            .map(|pane| DuplicateGuard::over(&pane.store, db));
        let (items, source_grid, label): (Vec<Item>, GridId, &str) = match (self.active_tab, sack) {
            (LeftTab::Inventory, Some(index)) => {
                let pane = self.character.as_mut().ok_or("no character loaded")?;
                let sack = pane
                    .character
                    .sacks
                    .get_mut(index)
                    .ok_or("that sack no longer exists")?;
                let label = if index == 0 {
                    "the Main Sack"
                } else {
                    "the sack"
                };
                (
                    drain_or_clone(&mut sack.items, mode, db, guard.as_mut()),
                    GridId::Sack(index),
                    label,
                )
            }
            (LeftTab::Inventory, None) => {
                let pane = self.character.as_mut().ok_or("no character loaded")?;
                let items = pane
                    .character
                    .sacks
                    .iter_mut()
                    .flat_map(|sack| drain_or_clone(&mut sack.items, mode, db, guard.as_mut()))
                    .collect();
                (items, GridId::Sack(0), "the inventory")
            }
            (LeftTab::Bank, _) => {
                let pane = self.bank.as_mut().ok_or("no bank loaded")?;
                (
                    drain_or_clone(&mut pane.stash.items, mode, db, guard.as_mut()),
                    GridId::Bank,
                    "the bank",
                )
            }
            (LeftTab::Shared, _) => {
                let pane = self.shared.as_mut().ok_or("no shared bank loaded")?;
                (
                    drain_or_clone(&mut pane.stash.items, mode, db, guard.as_mut()),
                    GridId::Shared,
                    "the shared bank",
                )
            }
            (LeftTab::Relic, _) => {
                let pane = self.relics.as_mut().ok_or("no relic bank loaded")?;
                (
                    drain_or_clone(&mut pane.stash.items, mode, db, guard.as_mut()),
                    GridId::Relic,
                    "the relic bank",
                )
            }
        };
        let skipped = guard.map_or(0, |guard| guard.skipped());
        if items.is_empty() {
            // Everything filtered out is a no-op, not a failure — and
            // marking the source dirty would autosave identical bytes.
            return match skipped {
                0 => Err(format!("{label} has no items")),
                _ => Ok(format!(
                    "Nothing sent — every item in {label} is already stored ({skipped} seen)"
                )),
            };
        }
        let pane = self.store.as_mut().expect("checked above");
        let moved = pane.store.add_all(items);
        pane.dirty = true;
        self.search.mark_data_changed();
        if mode == BulkMode::Move {
            self.mark_left_dirty(source_grid);
            self.left_selected = None;
        }
        let verb = match mode {
            BulkMode::Move => "Moved",
            BulkMode::Copy => "Copied",
        };
        Ok(format!(
            "{verb} {} from {label} → store, filed by type{}",
            count_items(moved),
            skipped_note(skipped)
        ))
    }

    /// The grid the active left tab addresses — where store → left
    /// sends land. The inventory tab prefers the selected sack.
    /// `None` when the tab's document isn't loaded.
    fn active_tab_grid(&self) -> Option<GridId> {
        match self.active_tab {
            LeftTab::Inventory => self.character.as_ref().map(|_| {
                if let Some(ItemAddr::Grid {
                    grid: GridId::Sack(sack),
                    ..
                }) = self.left_selected
                {
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

    fn move_store_to_left(&mut self) -> Result<String, String> {
        let id = self
            .store
            .as_ref()
            .and_then(|pane| pane.selected)
            .and_then(ItemAddr::stored_id)
            .ok_or("select an item in the store")?;
        self.send_store_to_left(id)
    }

    /// Sends one stored item to the active left tab; a failed
    /// placement puts it straight back in the store.
    fn send_store_to_left(&mut self, id: StoredItemId) -> Result<String, String> {
        let destination = self
            .active_tab_grid()
            .ok_or("the active left tab has nothing loaded")?;
        let item = self
            .store
            .as_mut()
            .ok_or("no item store open")?
            .store
            .remove(id)
            .ok_or("selection is stale — pick the item again")?;
        let db = loaded_db(&self.game);
        let label = self.caches.names.item_label(db, &item);
        let bad_index = |item: Item| transfer::Rejected {
            item: Box::new(item),
            reason: transfer::TransferError::BadIndex,
        };
        let placed = match destination {
            GridId::Sack(preferred) => match self.character.as_mut() {
                Some(pane) => {
                    transfer::place_in_character(&mut pane.character, item, preferred, db)
                        .map(|sack| format!("{label} → sack {}", sack + 1))
                }
                None => Err(bad_index(item)),
            },
            GridId::Bank => match self.bank.as_mut() {
                Some(pane) => transfer::place_in_stash(&mut pane.stash, item, db)
                    .map(|()| format!("{label} → bank")),
                None => Err(bad_index(item)),
            },
            GridId::Shared => match self.shared.as_mut() {
                Some(pane) => transfer::place_in_stash(&mut pane.stash, item, db)
                    .map(|()| format!("{label} → shared bank")),
                None => Err(bad_index(item)),
            },
            GridId::Relic => match self.relics.as_mut() {
                Some(pane) => transfer::place_in_stash(&mut pane.stash, item, db)
                    .map(|()| format!("{label} → relic bank")),
                None => Err(bad_index(item)),
            },
            GridId::Equipment(_) => Err(bad_index(item)),
        };
        match placed {
            Ok(message) => {
                self.mark_left_dirty(destination);
                self.mark_store_dirty();
                if let Some(pane) = self.store.as_mut() {
                    pane.selected = None;
                }
                Ok(message)
            }
            Err(rejected) => {
                let reason = rejected.reason;
                match self.store.as_mut() {
                    Some(pane) => {
                        pane.store.add(*rejected.item);
                        Err(format!("{reason}; item returned to the store"))
                    }
                    None => Err(format!(
                        "{reason}; item could not be returned — reload without saving"
                    )),
                }
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

    /// Right-click: sends a left-side item straight to the store, or
    /// a stored item back to the left side.
    fn quick_move(&mut self, addr: ItemAddr) -> Result<String, String> {
        match addr {
            ItemAddr::Grid { grid, index } => self.send_left_to_store(grid, index),
            ItemAddr::Stored(id) => self.send_store_to_left(id),
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
                            "A stash (.dxb / .dxg) opens on its own.",
                            "Your item store opens at launch and holds everything you \
                         put away. It is one file, and it files each item by type — \
                         pick a family plate, then a type below it.",
                        ],
                    );
                    help_section(
                        ui,
                        "The item store",
                        &[
                            "Tabs are types, not containers: an item lands under its own \
                         type wherever you drop it in the store, and no type ever fills up.",
                            "\"Import vault…\" reads a TQVaultAE vault into the store \
                         (that file is never changed); vaults already in your vaults \
                         folder were imported once, the first time the store was created.",
                            "\"Export type…\" and \"Export all…\" write a fresh \
                         TQVaultAE-readable vault — a snapshot to hand to that tool, not \
                         a live copy.",
                        ],
                    );
                    help_section(
                        ui,
                        "Moving items",
                        &[
                            "Drag an item between any two grids.",
                            "Right-click sends it to the other pane; Shift+Right-click \
                         sends a copy.",
                            "Shift+Click duplicates in place.",
                            "\"Move all → Store\" and \"Copy all → Store\" send a whole \
                         sack or bank at once; every item is filed under its own type.",
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
                            "Drag a partial piece onto another of the same record to \
                         pour its shards in; the remainder stays behind.",
                            "Double-click a completed relic, charm, or artifact to \
                         (re)pick its completion bonus.",
                            "Gear holding a piece shows an orange pip in its lower-left \
                         corner — one per filled socket. Hovering any item lists the \
                         gestures it accepts.",
                        ],
                    );
                    help_section(
                        ui,
                        "Search",
                        &[
                            "\"Search store…\" (⌘F) shows one filterable table over the \
                         whole store, in the store pane's place.",
                            "Rows take the same gestures as grid items; double-click \
                         jumps to an item's type tab. Esc brings the store back.",
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
    fn item_at(&self, addr: ItemAddr) -> Result<Item, String> {
        let stale = "item changed under the click — try again";
        let (grid, index) = match addr {
            ItemAddr::Stored(id) => {
                return self
                    .store
                    .as_ref()
                    .ok_or("no item store open")?
                    .store
                    .get(id)
                    .cloned()
                    .ok_or_else(|| stale.to_string());
            }
            ItemAddr::Grid { grid, index } => (grid, index),
        };
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
        }
    }

    /// Auto-places `item` into the container `addr` names: the store
    /// files it by type (never full), a grid finds it an open spot —
    /// and a worn item's companion lands in the sacks, since the doll
    /// has no free slots to auto-place into. The home for duplicates,
    /// extracted pieces, and items coming back from a failed move.
    fn place_beside(&mut self, addr: ItemAddr, item: Item) -> Result<(), String> {
        let db = loaded_db(&self.game);
        let grid = match addr {
            ItemAddr::Stored(_) => {
                let pane = self.store.as_mut().ok_or("no item store open")?;
                pane.store.add(item);
                return Ok(());
            }
            ItemAddr::Grid { grid, .. } => grid,
        };
        match grid {
            GridId::Sack(sack) => {
                let pane = self.character.as_mut().ok_or("no character loaded")?;
                transfer::place_in_character(&mut pane.character, item, sack, db).map(|_| ())
            }
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
        }
        .map_err(|rejected| rejected.reason.to_string())
    }

    /// Shift+Click: clones the item — same seed, so an exact copy —
    /// and auto-places it in its own container.
    fn duplicate_item(&mut self, addr: ItemAddr) -> Result<String, String> {
        let item = self.item_at(addr)?;
        let label = self.caches.names.item_label(loaded_db(&self.game), &item);
        match self.place_beside(addr, item) {
            Ok(()) => {
                self.mark_addr_dirty(addr);
                Ok(format!("duplicated {label}"))
            }
            Err(reason) => Err(format!("cannot duplicate: {reason}")),
        }
    }

    /// Alt+Click: splits the socketed relic/charm out of the item —
    /// the cleaned item stays put and the standalone piece (shard
    /// count and bonus preserved) is auto-placed in the same
    /// container. Nothing is destroyed: the app-side answer to the
    /// Enchanter's destroy-one-half recovery.
    fn extract_socketed(&mut self, addr: ItemAddr) -> Result<String, String> {
        let gear = self.item_at(addr)?;
        let slot =
            transfer::socketed_slot(&gear).ok_or("no relic or charm socketed in that item")?;
        let db = loaded_db(&self.game);
        let mut cleaned = gear;
        let piece =
            transfer::extract_relic(db, &mut cleaned, slot).map_err(|error| error.to_string())?;
        let piece_label = self.caches.names.record_name(db, &piece.base);
        let gear_label = self.caches.names.record_name(db, &cleaned.base);
        // Place the piece first; the gear is only committed once the
        // piece has a home, so a full container changes nothing.
        if let Err(reason) = self.place_beside(addr, piece) {
            return Err(format!("cannot extract: {reason} (the item is unchanged)"));
        }
        match self.item_mut(addr) {
            Some(slot_item) => *slot_item = cleaned,
            None => return Err("item moved under the click — extracted piece placed".to_string()),
        }
        self.mark_addr_dirty(addr);
        Ok(format!(
            "extracted {piece_label} from {gear_label} — both kept"
        ))
    }

    /// Shift+Right-click: places a copy of the item in the other
    /// pane — the store for left-side items, the active left tab for
    /// stored items — leaving the original in place.
    fn copy_across(&mut self, addr: ItemAddr) -> Result<String, String> {
        let item = self.item_at(addr)?;
        let db = loaded_db(&self.game);
        let label = self.caches.names.item_label(db, &item);
        match addr {
            ItemAddr::Grid { .. } => {
                let bucket = univault_core::store::bucket_of(db, &item);
                let pane = self.store.as_mut().ok_or("no item store open")?;
                pane.store.add(item);
                pane.dirty = true;
                self.search.mark_data_changed();
                Ok(format!("copy of {label} → {}", bucket.label()))
            }
            ItemAddr::Stored(_) => {
                let destination = self
                    .active_tab_grid()
                    .ok_or("the active left tab has nothing loaded")?;
                let placed = self.place_beside(ItemAddr::grid(destination, 0), item);
                match placed {
                    Ok(()) => {
                        self.mark_left_dirty(destination);
                        Ok(format!("copy of {label} → {}", left_label(destination)))
                    }
                    Err(reason) => Err(format!("cannot copy: {reason}")),
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

    fn save_store(&mut self) -> Result<SaveOutcome, String> {
        let pane = self.store.as_mut().ok_or("nothing to save")?;
        if stamp_of(&pane.path) != pane.disk_stamp {
            return Ok(SaveOutcome::Conflict);
        }
        let json = pane.store.to_json().map_err(|error| error.to_string())?;
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
            || self.store.as_ref().is_some_and(|pane| pane.dirty)
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
        if self.store.as_ref().is_some_and(|pane| pane.dirty)
            && self.save_store()? == SaveOutcome::Conflict
        {
            self.push_conflict(DocId::Store);
        }
        Ok(())
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
            DocId::Store => self
                .store
                .as_ref()
                .map(|pane| (pane.path.clone(), pane.disk_stamp.clone(), pane.dirty)),
        }
    }

    /// Auto-refresh: keeps the watcher pointed at the open documents,
    /// drains its snapshots, and acts once a change has settled —
    /// clean panes reload silently, dirty ones raise the conflict
    /// modal. Never acts mid-interaction.
    fn drive_refresh(&mut self, ctx: &egui::Context) {
        let watched: Vec<PathBuf> = DocId::FIXED
            .into_iter()
            .filter_map(|doc| self.doc_state(doc).map(|(path, _, _)| path))
            .collect();
        if let Ok(mut guard) = self.watcher.paths.lock() {
            *guard = watched;
        }
        ctx.request_repaint_after(WATCH_INTERVAL);
        let mut polls = Vec::new();
        while let Ok(snapshot) = self.watcher.receiver.try_recv() {
            polls.push(snapshot);
        }
        if polls.is_empty() {
            return;
        }
        // A held pointer or a drag means the user is mid-gesture and
        // the panes must not move underneath it. Keyboard focus is
        // deliberately not part of this: it is sticky, so a caret left
        // in the search box used to disable refreshing for every pane
        // indefinitely. Only the character defers to it, below.
        if self.drag.is_some() || ctx.input(|input| input.pointer.any_down()) {
            return;
        }
        self.watch_health.checked(Instant::now());
        // Every queued poll is evidence, not just the newest: the
        // settle rule counts *disk* observations, and dropping the
        // backlog a hidden window accumulated would restart the count
        // from zero and stall the catch-up by another poll or two.
        let mut ready: Vec<DocId> = Vec::new();
        for snapshot in &polls {
            for doc in DocId::FIXED {
                let Some((path, ours, _)) = self.doc_state(doc) else {
                    continue;
                };
                let Some((_, observed)) = snapshot.iter().find(|(seen, _)| *seen == path) else {
                    continue;
                };
                if self
                    .refresh
                    .settled(&path, observed.as_ref(), ours.as_ref())
                    && !ready.contains(&doc)
                {
                    ready.push(doc);
                }
            }
        }
        let mut reloaded = Vec::new();
        let mut failed = Vec::new();
        for doc in ready {
            let Some((path, _, dirty)) = self.doc_state(doc) else {
                continue;
            };
            // The gold field edits the character in place through an
            // uncommitted buffer; reloading under it would land the
            // typed value on a replaced character.
            if doc == DocId::Character && self.editing_gold {
                continue;
            }
            if dirty {
                self.push_conflict(doc);
            } else {
                let held = self.game_doc_item_count(doc).unwrap_or(0);
                let restore = (held > 0).then(|| self.snapshot_doc(doc)).flatten();
                match self.reload_doc(doc) {
                    Ok(()) => {
                        // A game save caught between truncating and
                        // writing its items parses as a valid empty
                        // stash, so nothing failed and the patience
                        // counter never fires. Make the emptying prove
                        // itself once before the pane clears.
                        if self.game_doc_item_count(doc) == Some(0)
                            && let Some(restore) = restore
                            && mid_save(doc, &path)
                            && self.refresh.defer_empty(&path)
                        {
                            self.restore_doc(restore);
                            continue;
                        }
                        self.refresh.empty_settled(&path);
                        self.refresh.reload_succeeded(&path);
                        reloaded.push(doc_label(doc));
                    }
                    Err(error) => match self.refresh.reload_failed(&path) {
                        ReloadFailure::Transient => {}
                        ReloadFailure::Persisting => {
                            failed.push(format!("{}: {error}", doc_label(doc)));
                        }
                    },
                }
            }
        }
        if !failed.is_empty() {
            self.status = Some(Err(format!(
                "changed on disk but still not readable — {} (retrying; Reload forces it)",
                failed.join("; ")
            )));
        } else if !reloaded.is_empty() {
            self.status = Some(Ok(format!(
                "auto-reloaded {} — changed on disk",
                reloaded.join(", ")
            )));
        }
    }

    /// Items a game-owned document holds. `None` for the item store,
    /// which only this app writes — nothing can catch it mid-save, so
    /// it needs no empty-read guard.
    fn game_doc_item_count(&self, doc: DocId) -> Option<usize> {
        match doc {
            DocId::Character => self.character.as_ref().map(|pane| {
                let worn = pane.character.equipment.slots.iter().flatten().count();
                let carried: usize = pane
                    .character
                    .sacks
                    .iter()
                    .map(|sack| sack.items.len())
                    .sum();
                worn + carried
            }),
            DocId::Stash(slot) => match slot {
                StashSlot::Bank => &self.bank,
                StashSlot::Shared => &self.shared,
                StashSlot::Relic => &self.relics,
            }
            .as_ref()
            .map(|pane| pane.stash.items.len()),
            DocId::Store => None,
        }
    }

    /// The pane as it stands, so a reload that turns out to be a
    /// half-written save can be put back.
    fn snapshot_doc(&self, doc: DocId) -> Option<PaneSnapshot> {
        match doc {
            DocId::Character => self
                .character
                .clone()
                .map(|pane| PaneSnapshot::Character(Box::new(pane))),
            DocId::Stash(slot) => match slot {
                StashSlot::Bank => &self.bank,
                StashSlot::Shared => &self.shared,
                StashSlot::Relic => &self.relics,
            }
            .clone()
            .map(|pane| PaneSnapshot::Stash(slot, Box::new(pane))),
            DocId::Store => None,
        }
    }

    /// Puts a snapshot back, stamp included — the older stamp is what
    /// makes the next poll re-examine the file rather than call it
    /// settled.
    fn restore_doc(&mut self, snapshot: PaneSnapshot) {
        match snapshot {
            PaneSnapshot::Character(pane) => self.character = Some(*pane),
            PaneSnapshot::Stash(slot, pane) => *self.stash_slot_mut(slot) = Some(*pane),
        }
    }

    /// Re-reads one document from its own path, dropping in-memory
    /// edits for it (callers decide when that is allowed). A read
    /// that fails leaves the pane exactly as it was — edits included,
    /// still marked unsaved — so a failed disk-wins reload cannot
    /// strand edits that look saved but never are.
    fn reload_doc(&mut self, doc: DocId) -> Result<(), String> {
        let (path, _, was_dirty) = self.doc_state(doc).ok_or("nothing loaded")?;
        self.set_dirty(doc, false);
        let reloaded = match doc {
            DocId::Character => self.open_character_file(&path).map(|_| ()),
            DocId::Stash(slot) => self.open_stash(slot, &path).map(|_| ()),
            DocId::Store => self.open_store().map(|_| ()),
        };
        if reloaded.is_err() {
            self.set_dirty(doc, was_dirty);
        }
        reloaded
    }

    fn set_dirty(&mut self, doc: DocId, dirty: bool) {
        match doc {
            DocId::Character => {
                if let Some(pane) = &mut self.character {
                    pane.dirty = dirty;
                }
            }
            DocId::Stash(slot) => {
                if let Some(pane) = self.stash_slot_mut(slot).as_mut() {
                    pane.dirty = dirty;
                }
            }
            DocId::Store => {
                if let Some(pane) = &mut self.store {
                    pane.dirty = dirty;
                }
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
                Ok(()) => done.push(doc_label(doc)),
                Err(error) => failed.push(format!("{}: {error}", doc_label(doc))),
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
                DocId::Store => {
                    if let Some(pane) = &mut self.store {
                        pane.disk_stamp = fresh;
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
        let labels: Vec<String> = self.conflicts.iter().map(|doc| doc_label(*doc)).collect();
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

/// The loaded game cache; `None` while it is absent, importing, or
/// failed. A free function over the one field so callers can keep
/// other disjoint borrows of `App` alive.
fn loaded_db(game: &GameStatus) -> Option<&GameCache> {
    match game {
        GameStatus::Loaded(data) => Some(data),
        GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
    }
}

/// Taking the dragged item out of a positional container shifts every
/// later item in it down one; a target sitting after the taken slot
/// has to be re-aimed. Store addresses are identities, so they never
/// move.
fn shift_after_take(target: ItemAddr, source: ItemAddr) -> ItemAddr {
    let (
        ItemAddr::Grid { grid, index },
        ItemAddr::Grid {
            grid: from,
            index: taken,
        },
    ) = (target, source)
    else {
        return target;
    };
    if grid == from && taken < index {
        ItemAddr::grid(grid, index - 1)
    } else {
        target
    }
}

/// Human name of a document for statuses and the conflict modal.
/// Whether a document that just read empty looks like a save still
/// in flight rather than a bank someone emptied.
///
/// A stash carries the evidence itself: the game keeps the `.dxg`
/// twin as its last good write, so a `.dxb` reading empty while its
/// twin still holds items is a write caught in the middle. The
/// character has no twin — a half-written one almost always fails to
/// parse and is covered by [`RELOAD_PATIENCE`] — so it falls back to
/// asking for one confirming read.
fn mid_save(doc: DocId, path: &Path) -> bool {
    match doc {
        DocId::Stash(_) => std::fs::read(path.with_extension("dxg"))
            .ok()
            .and_then(|twin| stash::restore_from_twin(&twin).ok())
            .and_then(|bytes| stash::parse_stash(&bytes).ok())
            .is_some_and(|twin| !twin.items.is_empty()),
        DocId::Character => true,
        DocId::Store => false,
    }
}

fn doc_label(doc: DocId) -> String {
    match doc {
        DocId::Character => "the character".to_string(),
        DocId::Stash(slot) => slot.label().to_string(),
        DocId::Store => "the item store".to_string(),
    }
}

/// A game-side container's name for status messages.
fn left_label(grid: GridId) -> String {
    match grid {
        GridId::Sack(sack) => format!("sack {}", sack + 1),
        GridId::Equipment(slot) => slot.label().to_string(),
        GridId::Bank => "bank".to_string(),
        GridId::Shared => "shared bank".to_string(),
        GridId::Relic => "relic bank".to_string(),
    }
}

/// A bulk send's payload: a move drains the source, a copy clones it.
/// A `guard` admits each item first — what it turns down is never
/// taken, so a skipped duplicate keeps its place and its bytes in the
/// source file rather than being drained and put back.
fn drain_or_clone(
    items: &mut Vec<Item>,
    mode: BulkMode,
    db: Option<&GameCache>,
    guard: Option<&mut DuplicateGuard>,
) -> Vec<Item> {
    let Some(guard) = guard else {
        return match mode {
            BulkMode::Move => std::mem::take(items),
            BulkMode::Copy => items.clone(),
        };
    };
    match mode {
        BulkMode::Move => {
            let (taken, left) = items.drain(..).partition(|item| guard.admit(db, item));
            *items = left;
            taken
        }
        BulkMode::Copy => items
            .iter()
            .filter(|item| guard.admit(db, item))
            .cloned()
            .collect(),
    }
}

/// Provenance key for an imported vault file: its file name, which is
/// what makes a re-import recognizable across runs.
fn import_source_name(path: &Path) -> Option<String> {
    Some(path.file_name()?.to_string_lossy().into_owned())
}

fn import_record(path: &Path, source: String, count: usize) -> ImportRecord {
    let stamp = stamp_of(path);
    ImportRecord {
        source,
        size: stamp
            .as_ref()
            .and_then(|stamp| u64::try_from(stamp.size).ok())
            .unwrap_or(0),
        mtime: stamp.map_or(0, |stamp| stamp.mtime_seconds),
        count,
    }
}

/// The `TQVaultAE`-format vault files sitting in the vaults folder —
/// migration input on first launch, never a store.
fn vault_files_in_vaults_dir() -> Vec<PathBuf> {
    let Some(dir) = vaults_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        })
        .collect();
    files.sort();
    files
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

/// The unified store's file — one per install, under the config
/// directory beside the vaults folder it superseded.
fn store_path() -> Option<PathBuf> {
    univault_core::platform::config_dir().map(|dir| dir.join("vault-store.json"))
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

    fn on_exit(&mut self) {
        let current = self.ui_snapshot();
        self.ui_state.flush(current);
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
        self.gilded_border
            .paint(ui.painter(), ui.ctx().content_rect().shrink(3.0));
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
        if matches!(self.game, GameStatus::Loaded(_))
            && ui
                .ctx()
                .input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
        {
            match self.view {
                MainView::Panes => self.enter_search(),
                MainView::Search => self.view = MainView::Panes,
            }
        }
        self.show_header(ui);
        ui.separator();
        let (action, drag_frame, search_frame) = self.show_panes(ui);
        if let Some(addr) = drag_frame.duplicate {
            self.status = Some(self.duplicate_item(addr));
        }
        if let Some(addr) = drag_frame.quick_move {
            self.status = Some(self.quick_move(addr));
        }
        if let Some(addr) = drag_frame.copy_across {
            self.status = Some(self.copy_across(addr));
        }
        if let Some(addr) = drag_frame.edit_bonus {
            self.request_bonus_edit(addr);
        }
        if let Some(addr) = drag_frame.extract {
            self.status = Some(self.extract_socketed(addr));
        }
        self.update_drag(ui.ctx(), drag_frame);
        match action {
            Some(PaneAction::MoveToStore) => self.status = Some(self.move_left_to_store()),
            Some(PaneAction::MoveAllToStore) => {
                self.status = Some(self.bulk_left_to_store(BulkMode::Move, None));
            }
            Some(PaneAction::CopyAllToStore) => {
                self.status = Some(self.bulk_left_to_store(BulkMode::Copy, None));
            }
            Some(PaneAction::MoveSackToStore(sack)) => {
                self.status = Some(self.bulk_left_to_store(BulkMode::Move, Some(sack)));
            }
            Some(PaneAction::CopySackToStore(sack)) => {
                self.status = Some(self.bulk_left_to_store(BulkMode::Copy, Some(sack)));
            }
            Some(PaneAction::MoveToFile) => self.status = Some(self.move_store_to_left()),
            Some(PaneAction::OpenSearch) => self.enter_search(),
            Some(PaneAction::PreviewRespec(kind)) => self.preview_respec(kind),
            Some(PaneAction::ImportVault) => {
                if let Some(path) = pick_file("Vault", &["json", "vault"], self.dialog_start_dir())
                {
                    self.status = Some(self.open(&path));
                }
            }
            Some(action @ (PaneAction::ExportBucket | PaneAction::ExportAll)) => {
                let whole = matches!(action, PaneAction::ExportAll);
                if let Some(path) = save_file("Vault", "json", self.export_start_name(whole)) {
                    self.status = Some(self.export_vault_file(&path, whole));
                }
            }
            None => {}
        }
        if let Some(addr) = search_frame.duplicate {
            self.status = Some(self.duplicate_item(addr));
        }
        if let Some(addr) = search_frame.quick_move {
            self.status = Some(self.quick_move(addr));
        }
        if let Some(addr) = search_frame.copy_across {
            self.status = Some(self.copy_across(addr));
        }
        if let Some(addr) = search_frame.extract {
            self.status = Some(self.extract_socketed(addr));
        }
        if let Some(addr) = search_frame.jump {
            self.jump_to_stored(addr);
        }
        if search_frame.leave {
            self.view = MainView::Panes;
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
        self.persist_ui_state(ui.ctx());
    }
}

/// The Inventory view's exclusive sub-tab: the doll, or one sack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
enum InventoryTab {
    #[default]
    Equipment,
    Sack(usize),
}

enum PaneAction {
    MoveToStore,
    MoveAllToStore,
    CopyAllToStore,
    MoveSackToStore(usize),
    CopySackToStore(usize),
    MoveToFile,
    PreviewRespec(RespecKind),
    OpenSearch,
    ImportVault,
    ExportBucket,
    ExportAll,
}

/// Whether a bulk send drains the source or leaves it untouched.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BulkMode {
    Move,
    Copy,
}

/// Tail for a bulk-send toast when the duplicate box turned items
/// away; empty when it turned none away or was never set.
fn skipped_note(skipped: usize) -> String {
    match skipped {
        0 => String::new(),
        1 => "; 1 skipped as a duplicate".to_string(),
        many => format!("; {many} skipped as duplicates"),
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
    addr: ItemAddr,
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
            if plate_button(ui, pane_chrome, true, "Import vault…")
                .on_hover_text(
                    "Read a TQVaultAE vault file into the store — the file itself \
                     is never changed",
                )
                .clicked()
            {
                requested = pick_file("Vault", &["json", "vault"], self.dialog_start_dir());
            }
            if plate_button(
                ui,
                pane_chrome,
                matches!(self.game, GameStatus::Loaded(_)),
                "Search store…",
            )
            .on_hover_text(
                "One filterable table of everything in the store (⌘F / Ctrl+F). \
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
                 as tabs, and your item store is already open on the right, \
                 sorted into type tabs.",
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
            let now = Instant::now();
            let health = &self.watch_health;
            let color = if health.stalled_now(now) {
                theme::GOLD
            } else {
                theme::TEXT_WEAK
            };
            ui.label(
                egui::RichText::new(health.summary(now))
                    .color(color)
                    .size(11.0),
            )
            .on_hover_text(
                "Auto-refresh watches the open files every 2 s, but only while the \
                 window is drawing — macOS stops that for a fully hidden window, so \
                 a long pause here means the app was covered, not that a file was \
                 missed. Panes catch up on the first frame after it is visible again.",
            );
        });
    }

    /// Advances the drag: adopts a newly started one, paints the item
    /// at the pointer, and commits or cancels on release.
    fn update_drag(&mut self, ctx: &egui::Context, frame: DragFrame) {
        self.editing_gold = frame.editing_gold;
        if self.drag.is_none() {
            self.drag = frame.begin;
        }
        let Some(state) = self.drag.clone() else {
            return;
        };

        let db = loaded_db(&self.game);
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
                if let Some(target) = candidate.combine_with {
                    self.status = Some(self.perform_combine(&state, target));
                } else if let Some(target) = candidate.socket_into {
                    self.status = Some(self.perform_socket(&state, target));
                } else if candidate.equips {
                    self.status = Some(self.perform_equip(&state, candidate.target));
                } else if candidate.fits {
                    let unmoved = match (candidate.target, state.source) {
                        (DropTarget::Grid(grid), ItemAddr::Grid { grid: source, .. }) => {
                            grid == source && candidate.cell == state.item.position
                        }
                        // The store has no cells: a stored item
                        // dropped back into it would not move at all.
                        (DropTarget::Store, ItemAddr::Stored(_)) => true,
                        (DropTarget::Grid(_) | DropTarget::Store, _) => false,
                    };
                    if !unmoved {
                        self.status = Some(self.perform_drop(&state, candidate));
                    }
                }
            }
            self.drag = None;
            ctx.request_repaint();
        }
    }

    /// Removes the item at `addr` from whatever holds it.
    fn take_at(&mut self, addr: ItemAddr) -> Result<Item, String> {
        match addr {
            ItemAddr::Grid { grid, index } => self.take_from_left(grid, index),
            ItemAddr::Stored(id) => self
                .store
                .as_mut()
                .ok_or("no item store open")?
                .store
                .remove(id)
                .ok_or_else(|| "item moved under the drag — drop ignored".to_string()),
        }
    }

    /// The item at `addr`, mutable in place.
    fn item_mut(&mut self, addr: ItemAddr) -> Option<&mut Item> {
        let (grid, index) = match addr {
            ItemAddr::Stored(id) => return self.store.as_mut()?.store.get_mut(id),
            ItemAddr::Grid { grid, index } => (grid, index),
        };
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
        }
    }

    /// Drops a partial relic/charm onto a matching partial: shards
    /// pour into the target up to completion, the remainder stays in
    /// the source, and a completed piece opens the bonus picker.
    fn perform_combine(&mut self, state: &DragState, target: ItemAddr) -> Result<String, String> {
        let needed = loaded_db(&self.game)
            .and_then(|db| db.completed_relic_level(&state.item.base))
            .ok_or("no completion data for this item")?;

        let mut source = self.take_at(state.source)?;
        if source.base != state.item.base {
            let origin = state.item.position;
            self.restore_dropped(state.source, source, origin)?;
            return Err("item moved under the drag — drop ignored".to_string());
        }
        let origin = source.position;
        let target = shift_after_take(target, state.source);
        let outcome = match self.item_mut(target) {
            Some(held) if held.base == source.base => {
                transfer::combine_shards(held, &mut source, needed)
            }
            Some(_) | None => {
                self.restore_dropped(state.source, source, origin)?;
                return Err("combine target moved — drop ignored".to_string());
            }
        };
        if !outcome.source_emptied {
            self.restore_dropped(state.source, source, origin)?;
        }
        self.mark_addr_dirty(state.source);
        self.mark_addr_dirty(target);
        self.clear_selections();
        let label = self
            .caches
            .names
            .record_name(loaded_db(&self.game), &state.item.base);
        if outcome.target_completed {
            self.begin_bonus_pick(target, state.item.base.clone(), None);
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
    fn perform_socket(&mut self, state: &DragState, target: ItemAddr) -> Result<String, String> {
        let piece = self.take_at(state.source)?;
        if piece.base != state.item.base {
            let origin = state.item.position;
            self.restore_dropped(state.source, piece, origin)?;
            return Err("item moved under the drag — drop ignored".to_string());
        }
        let origin = piece.position;
        let target = shift_after_take(target, state.source);
        let db = loaded_db(&self.game);
        let allowed = self
            .item_at(target)
            .is_ok_and(|held| transfer::can_socket(db, &piece, &held));
        if !allowed {
            self.restore_dropped(state.source, piece, origin)?;
            return Err("socket target moved — drop ignored".to_string());
        }
        let piece_label = self.caches.names.record_name(db, &piece.base);
        let target_label = self.item_at(target).map_or_else(
            |_| "item".to_string(),
            |held| self.caches.names.record_name(db, &held.base),
        );
        match self.item_mut(target) {
            Some(held) => transfer::socket_relic(held, piece),
            None => return Err("socket target moved — drop ignored".to_string()),
        }
        self.mark_addr_dirty(state.source);
        self.mark_addr_dirty(target);
        self.clear_selections();
        Ok(format!("socketed {piece_label} into {target_label}"))
    }

    /// Drops the dragged item into an empty, type-matching paper-doll
    /// slot: it comes off its container and onto the character.
    fn perform_equip(&mut self, state: &DragState, target: DropTarget) -> Result<String, String> {
        let DropTarget::Grid(GridId::Equipment(slot)) = target else {
            return Err("not an equipment slot".to_string());
        };
        let item = self.take_at(state.source)?;
        if item.base != state.item.base {
            let origin = state.item.position;
            self.restore_dropped(state.source, item, origin)?;
            return Err("item moved under the drag — drop ignored".to_string());
        }
        let origin = item.position;
        let db = loaded_db(&self.game);
        if !transfer::can_equip(db, &item, slot) {
            self.restore_dropped(state.source, item, origin)?;
            return Err("that item cannot be worn there".to_string());
        }
        let label = self.caches.names.item_label(db, &item);
        let pane = self.character.as_mut().ok_or("no character loaded")?;
        match transfer::equip(&mut pane.character, item, slot) {
            Ok(()) => {
                self.mark_addr_dirty(state.source);
                self.mark_left_dirty(GridId::Equipment(slot));
                self.clear_selections();
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
    fn begin_bonus_pick(&mut self, addr: ItemAddr, base: RecordId, current: Option<RecordId>) {
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
            addr,
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
    fn request_bonus_edit(&mut self, addr: ItemAddr) {
        let Ok(item) = self.item_at(addr) else {
            return;
        };
        match transfer::bonus_pick(loaded_db(&self.game), &item) {
            transfer::BonusPick::Ready => {
                self.begin_bonus_pick(addr, item.base.clone(), item.relic_bonus.clone());
            }
            transfer::BonusPick::Incomplete { have, needed } => {
                self.status = Some(Err(format!(
                    "complete the piece first ({have}/{needed} shards)"
                )));
            }
            transfer::BonusPick::NoTable => {
                self.status = Some(Err("this item has no bonus table".to_string()));
            }
            transfer::BonusPick::NotAPiece => {}
        }
    }

    /// Writes the chosen completion bonus (or none) onto the
    /// completed piece.
    fn apply_bonus(&mut self, choice: Option<RecordId>) -> Result<String, String> {
        let pending = self.pending_bonus.take().ok_or("no bonus pending")?;
        let stale = "the completed piece moved — bonus not applied";
        match self.item_mut(pending.addr) {
            Some(item) if item.base == pending.base => {
                item.relic_bonus.clone_from(&choice);
            }
            Some(_) | None => return Err(stale.to_string()),
        }
        self.mark_addr_dirty(pending.addr);
        let db = loaded_db(&self.game);
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
        let db = loaded_db(&self.game);
        let label = self.caches.names.item_label(db, &state.item);

        let taken = self.take_at(state.source)?;

        if taken.base != state.item.base {
            let origin = state.item.position;
            self.restore_dropped(state.source, taken, origin)?;
            return Err("item moved under the drag — drop ignored".to_string());
        }
        let origin = taken.position;

        let db = loaded_db(&self.game);
        let grid = match target.target {
            // The store has no cells: a dropped item is simply filed
            // by its own type, wherever the pointer let go.
            DropTarget::Store => {
                let bucket = univault_core::store::bucket_of(db, &taken);
                let Some(pane) = self.store.as_mut() else {
                    return Err("no item store open".to_string());
                };
                pane.store.add(taken);
                pane.dirty = true;
                self.mark_addr_dirty(state.source);
                self.clear_selections();
                self.search.mark_data_changed();
                return Ok(format!("{label} → {}", bucket.label()));
            }
            DropTarget::Grid(grid) => grid,
        };
        let placed = match grid {
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
        };

        match placed {
            Ok(()) => {
                self.mark_addr_dirty(state.source);
                self.mark_left_dirty(grid);
                self.clear_selections();
                Ok(format!(
                    "{label} → {} ({}, {})",
                    left_label(grid),
                    target.cell.x,
                    target.cell.y
                ))
            }
            Err(rejected) => {
                let reason = rejected.reason;
                self.restore_dropped(state.source, *rejected.item, origin)?;
                Err(format!("{reason}; item returned"))
            }
        }
    }

    /// Puts a taken item back where it came from: its original cell
    /// (guaranteed free) for a grid, falling back to any open spot —
    /// or straight back into the store, which always has room.
    fn restore_dropped(
        &mut self,
        source: ItemAddr,
        item: Item,
        position: univault_core::chr::GridPos,
    ) -> Result<(), String> {
        let db = loaded_db(&self.game);
        let lost = "item could not be returned — reload without saving".to_string();
        let source = match source {
            ItemAddr::Stored(_) => {
                let pane = self.store.as_mut().ok_or(lost)?;
                pane.store.add(item);
                return Ok(());
            }
            ItemAddr::Grid { grid, .. } => grid,
        };
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
        }
    }

    /// Marks whichever document `addr` belongs to as needing a save.
    fn mark_addr_dirty(&mut self, addr: ItemAddr) {
        match addr {
            ItemAddr::Grid { grid, .. } => self.mark_left_dirty(grid),
            ItemAddr::Stored(_) => self.mark_store_dirty(),
        }
    }

    fn mark_left_dirty(&mut self, grid: GridId) {
        // Any mutation can move rows the search table points at.
        self.search.mark_data_changed();
        let dirty = match grid {
            GridId::Sack(_) | GridId::Equipment(_) => {
                self.character.as_mut().map(|pane| &mut pane.dirty)
            }
            GridId::Bank => self.bank.as_mut().map(|pane| &mut pane.dirty),
            GridId::Shared => self.shared.as_mut().map(|pane| &mut pane.dirty),
            GridId::Relic => self.relics.as_mut().map(|pane| &mut pane.dirty),
        };
        if let Some(dirty) = dirty {
            *dirty = true;
        }
    }

    fn mark_store_dirty(&mut self) {
        self.search.mark_data_changed();
        if let Some(pane) = self.store.as_mut() {
            pane.dirty = true;
        }
    }

    /// Drops both panes' selections — what every completed move does,
    /// since the moved item is no longer where either pointed.
    fn clear_selections(&mut self) {
        self.left_selected = None;
        if let Some(pane) = self.store.as_mut() {
            pane.selected = None;
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

    /// Default file name for an export: the bucket's own name, or the
    /// whole store's.
    fn export_start_name(&self, whole_store: bool) -> String {
        if whole_store {
            return "UniVault Export.json".to_string();
        }
        self.store.as_ref().map_or_else(
            || "Export.json".to_string(),
            |pane| format!("{}.json", pane.view.bucket.label()),
        )
    }

    /// Shows a stored item at home in the store pane, switching to
    /// its family and type first.
    fn jump_to_stored(&mut self, addr: ItemAddr) {
        let Some(id) = addr.stored_id() else { return };
        let db = loaded_db(&self.game);
        let Some(pane) = self.store.as_mut() else {
            return;
        };
        if let Some(item) = pane.store.get(id) {
            pane.view.bucket = univault_core::store::bucket_of(db, item);
            pane.selected = Some(addr);
        }
        self.view = MainView::Panes;
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

    fn show_panes(
        &mut self,
        ui: &mut egui::Ui,
    ) -> (Option<PaneAction>, DragFrame, search::SearchFrame) {
        let dirty = self.any_dirty();
        let caches = &mut self.caches;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let drag = self.drag.clone();
        let mut frame = DragFrame::default();
        let mut search_frame = search::SearchFrame::default();
        let mut action = None;
        let has_left = self.character.is_some()
            || self.bank.is_some()
            || self.shared.is_some()
            || self.relics.is_some();
        let can_move = has_left && self.store.is_some();
        let panel = self.tabbed_panel.clone();
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
                if !view.loaded(*view.active_tab)
                    && let Some(fallback) = LeftTab::ALL.into_iter().find(|tab| view.loaded(*tab))
                {
                    *view.active_tab = fallback;
                    *view.selected = None;
                }
                let (tabs, selected) = left_tabs(&view);
                let response = panel.show(&mut columns[0], &tabs, selected, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.set_min_height(ui.available_height());
                    show_left_column(
                        ui,
                        &mut view,
                        &panel,
                        db,
                        caches,
                        can_move,
                        drag.as_ref(),
                        &mut frame,
                    )
                });
                if let Some(chosen) = response.inner {
                    action = Some(chosen);
                }
                if let Some(index) = response.clicked
                    && LeftTab::ALL[index] != *view.active_tab
                {
                    *view.active_tab = LeftTab::ALL[index];
                    *view.selected = None;
                }
            } else {
                columns[0].weak("No game file loaded.");
            }
            if self.view == MainView::Search {
                let tabs = [tabbed_panel::Tab::new("Search the store")];
                panel.show(&mut columns[1], &tabs, 0, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.set_min_height(ui.available_height());
                    search_frame = search::show_search_pane(
                        ui,
                        &mut self.search,
                        self.store.as_ref(),
                        db,
                        caches,
                        dirty,
                    );
                });
            } else if let Some(pane) = &mut self.store {
                if let Some(chosen) = show_store_column(
                    &mut columns[1],
                    pane,
                    &panel,
                    db,
                    caches,
                    can_move,
                    drag.as_ref(),
                    &mut frame,
                ) {
                    action = Some(chosen);
                }
            } else {
                columns[1].weak("No item store open.");
            }
        });
        (action, frame, search_frame)
    }
}

/// Allocates the doll's canvas centered in the pane, scaled to fill
/// the available width.
fn allocate_doll_canvas(ui: &mut egui::Ui) -> (egui::Rect, egui::Response, f32) {
    let cell = fit_cell_size(
        ui.available_size() - egui::vec2(0.0, 12.0),
        (DOLL_CELLS.0 + 1, DOLL_CELLS.1 + 1),
    );
    let pad = cell * 0.5;
    let size = egui::vec2(
        cells_at(DOLL_CELLS.0, cell) + 2.0 * pad,
        cells_at(DOLL_CELLS.1, cell) + 2.0 * pad,
    );
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

/// Native size of one grid cell — the textures' 32 pixels; grids
/// scale their on-screen cell up from this to fill their pane.
const CELL_SIZE: f32 = 32.0;

/// The on-screen cell size that fits `dims` into the available
/// space with no scrolling: bounded by width and height, capped at
/// 2× native so icons stay crisp, floored so cells stay integral.
#[allow(clippy::cast_precision_loss)] // grid dimensions are tiny
fn fit_cell_size(available: egui::Vec2, dims: (i32, i32)) -> f32 {
    let by_width = (available.x - 4.0) / dims.0.max(1) as f32;
    let by_height = (available.y - 4.0) / dims.1.max(1) as f32;
    by_width.min(by_height).floor().clamp(20.0, CELL_SIZE * 2.0)
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

/// Which game-side container an item lives in — all of them
/// positional, so `(GridId, index)` addresses a slot in one. Equipment
/// carries its slot in the id (the paper doll has no cell grid); the
/// index half is 0 there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GridId {
    Sack(usize),
    Equipment(EquipSlot),
    Bank,
    Shared,
    Relic,
}

/// Where an item is, addressably: a cell in a game-owned container,
/// or the unified store — which is identity-addressed, never
/// positional, so an address stays valid however the view reorders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemAddr {
    Grid { grid: GridId, index: usize },
    Stored(StoredItemId),
}

impl ItemAddr {
    fn grid(grid: GridId, index: usize) -> Self {
        Self::Grid { grid, index }
    }

    fn stored_id(self) -> Option<StoredItemId> {
        match self {
            Self::Stored(id) => Some(id),
            Self::Grid { .. } => None,
        }
    }
}

/// What a drop lands in: a cell of a game-side container, or the
/// store — which has no cells, so an item dropped there is simply
/// filed by type.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DropTarget {
    Grid(GridId),
    Store,
}

/// An in-flight drag: where the item came from and how it was
/// grabbed. The item stays in its container (painted dimmed) until
/// the drop commits.
#[derive(Clone)]
struct DragState {
    source: ItemAddr,
    item: Item,
    /// Pointer offset from the item's top-left, in points, so the
    /// item hangs where it was grabbed.
    grab: egui::Vec2,
}

/// Where a drop would land, computed by whichever surface the pointer
/// is over this frame. `cell` is meaningless for the store, which
/// files by type rather than position.
#[derive(Clone, Copy)]
struct DropCandidate {
    target: DropTarget,
    cell: univault_core::chr::GridPos,
    fits: bool,
    /// A matching partial relic/charm under the pointer: dropping
    /// combines into that item instead of placing.
    combine_with: Option<ItemAddr>,
    /// Socketable gear under the pointer: dropping sockets the
    /// dragged relic/charm into that item.
    socket_into: Option<ItemAddr>,
    /// An empty paper-doll slot the dragged item may be worn in
    /// (`target` is that `Equipment` slot): dropping equips.
    equips: bool,
}

/// What the grids reported back this frame.
#[derive(Default)]
struct DragFrame {
    begin: Option<DragState>,
    candidate: Option<DropCandidate>,
    /// A Shift+Click asking for the item to be duplicated into its
    /// own container.
    duplicate: Option<ItemAddr>,
    /// A right-click asking for the item to be sent straight to the
    /// other pane.
    quick_move: Option<ItemAddr>,
    /// A Shift+Right-click asking for a copy of the item to be placed
    /// in the other pane.
    copy_across: Option<ItemAddr>,
    /// A double-click asking to (re)pick the completion bonus of a
    /// completed relic/charm.
    edit_bonus: Option<ItemAddr>,
    /// An Alt+Click asking to split the socketed relic/charm out of
    /// the item — both survive.
    extract: Option<ItemAddr>,
    /// The gold field held keyboard focus this frame: the character
    /// must not be reloaded under a half-typed value.
    editing_gold: bool,
}

/// Paints a container as its actual cell grid, with items at their
/// positions (icon when decodable, initial letter otherwise), click
/// selection, a name tooltip on hover, and drag-and-drop with a
/// green/red footprint preview.
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn grid_view(
    ui: &mut egui::Ui,
    dims: (i32, i32),
    entries: &[(ItemAddr, &Item)],
    grid: GridId,
    selected: &mut Option<ItemAddr>,
    db: Option<&GameCache>,
    caches: &mut Caches,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) {
    let cell = fit_cell_size(ui.available_size(), dims);
    let size = egui::vec2(cells_at(dims.0, cell), cells_at(dims.1, cell));
    let pad = ((ui.available_width() - size.x) / 2.0).max(0.0);
    let (rect, response) = ui
        .horizontal(|ui| {
            ui.add_space(pad);
            ui.allocate_exact_size(size, egui::Sense::click_and_drag())
        })
        .inner;
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();
    let grid_chrome = caches.chrome(ui.ctx(), db);
    paint_grid_background(&painter, grid_chrome.as_ref(), rect, dims, cell);

    // The grab position decides which item a starting drag lifts —
    // the pointer may already have moved past egui's drag threshold.
    let press_origin = ui.ctx().input(|input| input.pointer.press_origin());

    let mut hovered: Option<&Item> = None;
    for (addr, item) in entries {
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
        let is_selected = *selected == Some(*addr);
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
        if drag.is_some_and(|state| state.source == *addr) {
            painter.rect_filled(item_rect, 2.0, egui::Color32::from_black_alpha(140));
        }

        if response.drag_started()
            && drag.is_none()
            && frame.begin.is_none()
            && press_origin.is_some_and(|origin| item_rect.contains(origin))
        {
            frame.begin = Some(DragState {
                source: *addr,
                item: (*item).clone(),
                grab: press_origin.map_or(egui::Vec2::ZERO, |origin| origin - item_rect.min),
            });
        }

        if drag.is_none()
            && let Some(pointer) = response.hover_pos()
            && item_rect.contains(pointer)
        {
            hovered = Some(item);
            item_gestures(ui, &response, *addr, selected, frame);
        }
    }
    if let Some(item) = hovered {
        egui::Tooltip::for_widget(&response)
            .at_pointer()
            .show(|ui| item_tooltip(ui, item, db, caches));
    }

    if let Some(state) = drag
        && let Some(pointer) = ui.ctx().pointer_latest_pos()
        && rect.contains(pointer)
    {
        frame.candidate = Some(paint_drop_preview(
            &painter,
            rect,
            cell,
            dims,
            entries,
            DropTarget::Grid(grid),
            state,
            pointer,
            db,
            caches,
        ));
    }
}

/// The grid's cell art (or painted fallback lines) behind items.
fn paint_grid_background(
    painter: &egui::Painter,
    grid_chrome: Option<&chrome::Chrome>,
    rect: egui::Rect,
    dims: (i32, i32),
    cell: f32,
) {
    if let Some(grid_chrome) = grid_chrome {
        for row in 0..dims.1 {
            for column in 0..dims.0 {
                let tile = egui::Rect::from_min_size(
                    rect.min + egui::vec2(cells_at(column, cell), cells_at(row, cell)),
                    egui::vec2(cell, cell),
                );
                grid_chrome.grid_cell(painter, tile);
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
    paint_socket_pips(painter, item_rect, item);
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

/// One relic-orange pip per filled socket, bottom-left — the only
/// sign on the grid that gear carries a relic or charm. Sits clear of
/// the stack count in the opposite corner.
fn paint_socket_pips(painter: &egui::Painter, item_rect: egui::Rect, item: &Item) {
    const RADIUS: f32 = 2.5;
    let pip = game_color(style::style_color(style::ItemStyle::Relic));
    let mut center = item_rect.left_bottom() + egui::vec2(RADIUS + 2.0, -(RADIUS + 2.0));
    for _ in transfer::socketed_slots(item) {
        painter.circle(
            center,
            RADIUS,
            pip,
            egui::Stroke::new(1.0, egui::Color32::from_black_alpha(200)),
        );
        center.x += 2.0f32.mul_add(RADIUS, 2.0);
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
    entries: &[(ItemAddr, &Item)],
    target: DropTarget,
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
    for (addr, item) in entries {
        if state.source == *addr {
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
                target,
                cell,
                fits: false,
                combine_with: combine.then_some(*addr),
                socket_into: socket.then_some(*addr),
                equips: false,
            };
        }
    }
    // The store has no cells to collide in: anywhere in the pane is a
    // valid drop, and the item files itself by type.
    if target == DropTarget::Store {
        return DropCandidate {
            target,
            cell,
            fits: true,
            combine_with: None,
            socket_into: None,
            equips: false,
        };
    }
    let occupied: Vec<univault_core::grid::CellRect> = entries
        .iter()
        .filter(|(addr, _)| *addr != state.source)
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
        target,
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
            if let Some(details) = &details {
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
            }
            tooltip_gestures(ui, item, db, caches);
        });
    if let Some(pane_chrome) = pane_chrome {
        pane_chrome.tooltip_frame(ui.painter(), response.response.rect);
    }
}

/// The footer naming what the hovered item can do beyond being
/// moved, and the gesture that reaches it — the app's socket
/// operations are otherwise invisible. Silent for ordinary items.
fn tooltip_gestures(ui: &mut egui::Ui, item: &Item, db: Option<&GameCache>, caches: &mut Caches) {
    let affordances = transfer::affordances(db, item);
    if affordances.is_empty() {
        return;
    }
    ui.add(egui::Separator::default().spacing(6.0));
    for affordance in affordances {
        let hint = match affordance {
            transfer::Affordance::ExtractSocketed { piece } => {
                let piece = caches.names.record_name(db, piece);
                format!("Alt+Click to remove {piece} — both are kept")
            }
            transfer::Affordance::SocketIntoGear => {
                "Drag onto gear its type allows to socket it".to_string()
            }
            transfer::Affordance::CombineShards => {
                "Drag onto a matching partial to pour in its shards".to_string()
            }
            transfer::Affordance::PickBonus => {
                "Double-click to pick its completion bonus".to_string()
            }
        };
        ui.label(
            egui::RichText::new(hint)
                .color(theme::TEXT_WEAK)
                .size(11.0)
                .italics(),
        );
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
    address: ItemAddr,
    selected: &mut Option<ItemAddr>,
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
            target: DropTarget::Grid(GridId::Equipment(slot)),
            cell: univault_core::chr::GridPos { x: 0, y: 0 },
            fits: false,
            combine_with: None,
            socket_into: sockets.then(|| ItemAddr::grid(GridId::Equipment(slot), 0)),
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
    selected: &mut Option<ItemAddr>,
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

    let origin = rect.min + egui::vec2(cell * 0.5, cell * 0.5);
    for slot in EquipSlot::ALL {
        let (x, y, w, h) = doll_geometry(slot);
        let box_rect = egui::Rect::from_min_size(
            origin + egui::vec2(cells_at(x, cell), cells_at(y, cell)),
            egui::vec2(cells_at(w, cell), cells_at(h, cell)),
        )
        .shrink(1.0);
        let addr = ItemAddr::grid(GridId::Equipment(slot), 0);
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
                let is_selected = *selected == Some(addr);
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
                if drag.is_some_and(|state| state.source == addr) {
                    painter.rect_filled(item_rect, 2.0, egui::Color32::from_black_alpha(140));
                }
                if response.drag_started()
                    && drag.is_none()
                    && frame.begin.is_none()
                    && press_origin.is_some_and(|origin| box_rect.contains(origin))
                {
                    frame.begin = Some(DragState {
                        source: addr,
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
                    item_gestures(ui, &response, addr, selected, frame);
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
            && state.source != addr
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
        egui::Tooltip::for_widget(&response)
            .at_pointer()
            .show(|ui| item_tooltip(ui, item, db, caches));
    }
}

#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn show_character_section(
    ui: &mut egui::Ui,
    pane: &mut CharacterPane,
    panel: &TabbedPanel,
    db: Option<&GameCache>,
    caches: &mut Caches,
    can_move: bool,
    inventory_tab: &mut InventoryTab,
    selected: &mut Option<ItemAddr>,
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
        frame.editing_gold = gold.has_focus();
        let selection_here = matches!(
            *selected,
            Some(ItemAddr::Grid {
                grid: GridId::Sack(_) | GridId::Equipment(_),
                ..
            })
        );
        if plate_button(ui, pane_chrome, can_move && selection_here, "→ Store").clicked() {
            action = Some(PaneAction::MoveToStore);
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
    let tabs = inventory_tabs(pane);
    let selected_index = match *inventory_tab {
        InventoryTab::Equipment => 0,
        InventoryTab::Sack(index) => index + 1,
    };
    let response = panel.show(ui, &tabs, selected_index, |ui| {
        ui.set_min_width(ui.available_width());
        ui.set_min_height(ui.available_height());
        show_inventory_body(
            ui,
            pane,
            db,
            caches,
            *inventory_tab,
            can_move,
            selected,
            drag,
            frame,
        )
    });
    if let Some(chosen) = response.inner {
        action = Some(chosen);
    }
    let target = response.clicked.or(if drag.is_some() {
        response.hovered
    } else {
        None
    });
    if let Some(index) = target {
        let tab = match index {
            0 => InventoryTab::Equipment,
            sack => InventoryTab::Sack(sack - 1),
        };
        if tab != *inventory_tab {
            *inventory_tab = tab;
            *selected = None;
        }
    }
    action
}

/// Tab strip inputs for the Inventory sub-tabs: the doll first,
/// then one plate per sack with its item count.
fn inventory_tabs(pane: &CharacterPane) -> Vec<tabbed_panel::Tab> {
    std::iter::once(tabbed_panel::Tab::new("Equipment"))
        .chain(
            pane.character
                .sacks
                .iter()
                .enumerate()
                .map(|(index, sack)| {
                    tabbed_panel::Tab::new(if index == 0 {
                        format!("Main Sack ({})", sack.items.len())
                    } else {
                        format!("Sack {} ({})", index, sack.items.len())
                    })
                }),
        )
        .collect()
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
    can_move: bool,
    selected: &mut Option<ItemAddr>,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) -> Option<PaneAction> {
    let mut action = None;
    match inventory_tab {
        InventoryTab::Equipment => {
            ui.add_space(12.0);
            show_equipment_doll(
                ui,
                &pane.character.equipment,
                selected,
                db,
                caches,
                drag,
                frame,
            );
            ui.add_space(12.0);
        }
        InventoryTab::Sack(index) => {
            if let Some(sack) = pane.character.sacks.get(index) {
                let pane_chrome = caches.chrome(ui.ctx(), db);
                let pane_chrome = pane_chrome.as_ref();
                let has_items = !sack.items.is_empty();
                ui.horizontal(|ui| {
                    if plate_button(ui, pane_chrome, can_move && has_items, "Move all → Store")
                        .on_hover_text(
                            "Move every item from this sack into the store, \
                             each filed under its own type",
                        )
                        .clicked()
                    {
                        action = Some(PaneAction::MoveSackToStore(index));
                    }
                    if plate_button(ui, pane_chrome, can_move && has_items, "Copy all → Store")
                        .on_hover_text("The same, as copies — every item stays in this sack")
                        .clicked()
                    {
                        action = Some(PaneAction::CopySackToStore(index));
                    }
                });
                let entries: Vec<(ItemAddr, &Item)> = sack
                    .items
                    .iter()
                    .enumerate()
                    .map(|(index_in_sack, item)| {
                        (ItemAddr::grid(GridId::Sack(index), index_in_sack), item)
                    })
                    .collect();
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
    action
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
    selected: &'a mut Option<ItemAddr>,
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

/// Tab strip inputs for the left pane — one plate per document,
/// unloaded ones disabled with their hint — plus the active index.
fn left_tabs(view: &LeftView<'_>) -> (Vec<tabbed_panel::Tab>, usize) {
    let tabs = LeftTab::ALL
        .into_iter()
        .map(|tab| {
            if view.loaded(tab) {
                tabbed_panel::Tab::new(tab.title())
            } else {
                tabbed_panel::Tab::disabled(tab.title(), tab.missing_hint())
            }
        })
        .collect();
    let selected = LeftTab::ALL
        .iter()
        .position(|tab| tab == view.active_tab)
        .unwrap_or(0);
    (tabs, selected)
}

/// How many of the store's items fall in each bucket — the counts
/// the family and sub-type tabs wear. One pass over the store, since
/// classification is a lookup per item.
fn bucket_counts(store: &VaultStore, db: Option<&GameCache>) -> HashMap<Bucket, usize> {
    let mut counts = HashMap::new();
    for entry in store.entries() {
        *counts
            .entry(univault_core::store::bucket_of(db, &entry.item))
            .or_insert(0) += 1;
    }
    counts
}

/// Tab strip inputs for the store's family plates. Names only: the
/// strip lays out on one unwrapped row, and six labelled counts
/// overflow the half-width column (silently — the plates just run off
/// the pane, taking Misc with them). Counts live one level down, on
/// the sub-type strip, with the store's total in the header.
fn family_tabs() -> Vec<tabbed_panel::Tab> {
    Family::ALL
        .into_iter()
        .map(|family| tabbed_panel::Tab::new(family.label()))
        .collect()
}

/// Sub-type strip inputs for the open family: one plate per bucket,
/// with its item count.
fn bucket_tabs(family: Family, counts: &HashMap<Bucket, usize>) -> Vec<tabbed_panel::Tab> {
    family
        .buckets()
        .iter()
        .map(|bucket| {
            let count = counts.get(bucket).copied().unwrap_or(0);
            tabbed_panel::Tab::new(if count == 0 {
                bucket.label().to_string()
            } else {
                format!("{} ({count})", bucket.label())
            })
        })
        .collect()
}

/// Lays a bucket's items out in reading order — left to right,
/// wrapping into shelves as tall as their tallest item — and returns
/// the rows used. The store keeps no positions, so this scratch
/// layout is recomputed each frame and never serialized.
fn shelve_items(footprints: &[(i32, i32)], width: i32) -> (Vec<univault_core::chr::GridPos>, i32) {
    let mut positions = Vec::with_capacity(footprints.len());
    let (mut x, mut y, mut shelf) = (0, 0, 0);
    for (item_width, item_height) in footprints {
        if x + item_width > width && x > 0 {
            y += shelf;
            x = 0;
            shelf = 0;
        }
        positions.push(univault_core::chr::GridPos { x, y });
        x += item_width;
        shelf = shelf.max(*item_height);
    }
    (positions, y + shelf)
}

/// The left column: the active document's section (the strip above
/// it is the pane's [`TabbedPanel`]).
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn show_left_column(
    ui: &mut egui::Ui,
    view: &mut LeftView<'_>,
    panel: &TabbedPanel,
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
                panel,
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
    selected: &mut Option<ItemAddr>,
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
        let selection_here =
            matches!(*selected, Some(ItemAddr::Grid { grid: current, .. }) if current == grid);
        if plate_button(ui, pane_chrome, can_move && selection_here, "→ Store").clicked() {
            action = Some(PaneAction::MoveToStore);
        }
        let has_items = !pane.stash.items.is_empty();
        if plate_button(ui, pane_chrome, can_move && has_items, "Move all → Store")
            .on_hover_text(
                "Move every item in this bank into the store, \
                 each filed under its own type",
            )
            .clicked()
        {
            action = Some(PaneAction::MoveAllToStore);
        }
        if plate_button(ui, pane_chrome, can_move && has_items, "Copy all → Store")
            .on_hover_text("The same, as copies — every item stays in the bank")
            .clicked()
        {
            action = Some(PaneAction::CopyAllToStore);
        }
    });
    ui.label(theme::path_text(pane.path.display().to_string()));
    let entries: Vec<(ItemAddr, &Item)> = pane
        .stash
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| (ItemAddr::grid(grid, index), item))
        .collect();
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

/// The store column: family plates over a sub-type strip over the
/// open bucket's grid. Both strips switch on click or mid-drag hover,
/// so a dragged item can be carried to any type without dropping it.
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn show_store_column(
    ui: &mut egui::Ui,
    pane: &mut StorePane,
    panel: &TabbedPanel,
    db: Option<&GameCache>,
    caches: &mut Caches,
    can_move: bool,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) -> Option<PaneAction> {
    let counts = bucket_counts(&pane.store, db);
    let tabs = family_tabs();
    let selected = Family::ALL
        .iter()
        .position(|family| *family == pane.view.family())
        .unwrap_or(0);
    let response = panel.show(ui, &tabs, selected, |ui| {
        ui.set_min_width(ui.available_width());
        ui.set_min_height(ui.available_height());
        show_store_pane(ui, pane, panel, &counts, db, caches, can_move, drag, frame)
    });
    let target = response.clicked.or(if drag.is_some() {
        response.hovered
    } else {
        None
    });
    if let Some(index) = target
        && let Some(family) = Family::ALL.get(index).copied()
        && family != pane.view.family()
    {
        pane.view.bucket = family.buckets()[0];
        pane.selected = None;
    }
    response.inner
}

#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn show_store_pane(
    ui: &mut egui::Ui,
    pane: &mut StorePane,
    panel: &TabbedPanel,
    counts: &HashMap<Bucket, usize>,
    db: Option<&GameCache>,
    caches: &mut Caches,
    can_move: bool,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) -> Option<PaneAction> {
    let mut action = None;
    let pane_chrome = caches.chrome(ui.ctx(), db);
    let pane_chrome = pane_chrome.as_ref();
    let total = pane.store.len();
    ui.horizontal_wrapped(|ui| {
        pane_heading(ui, pane_chrome, "Item store");
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
            .on_hover_text("Search and filter the whole store (⌘F / Ctrl+F)")
            .clicked()
        {
            action = Some(PaneAction::OpenSearch);
        }
        let sort = egui::ComboBox::from_id_salt("store-sort")
            .selected_text(pane.view.sort.key.label())
            .width(110.0);
        sort.show_ui(ui, |ui| {
            for key in StoreSortKey::ALL {
                if ui
                    .selectable_label(pane.view.sort.key == key, key.label())
                    .clicked()
                    && pane.view.sort.key != key
                {
                    pane.view.sort = StoreSort::by(key);
                }
            }
        });
        if plate_button(ui, pane_chrome, true, pane.view.sort.direction.arrow())
            .on_hover_text(pane.view.sort.direction.flip_hint())
            .clicked()
        {
            pane.view.sort.direction = pane.view.sort.direction.flipped();
        }
    });
    ui.horizontal_wrapped(|ui| {
        if plate_button(ui, pane_chrome, true, "Import vault…")
            .on_hover_text("Read a TQVaultAE vault file into the store; the file is never changed")
            .clicked()
        {
            action = Some(PaneAction::ImportVault);
        }
        if plate_button(ui, pane_chrome, total > 0, "Export type…")
            .on_hover_text("Write this type's items out as a TQVaultAE-readable vault")
            .clicked()
        {
            action = Some(PaneAction::ExportBucket);
        }
        if plate_button(ui, pane_chrome, total > 0, "Export all…")
            .on_hover_text("Write the whole store out as a TQVaultAE-readable vault")
            .clicked()
        {
            action = Some(PaneAction::ExportAll);
        }
        ui.weak(format!("{} stored", count_items(total)));
        ui.checkbox(&mut pane.view.skip_duplicate_seeds, "Skip duplicates")
            .on_hover_text(
                "Bulk sends only: pass over an item whose seed is already stored in its \
                 type. Single sends, right-click sends, and drops always land.",
            );
    });

    let family = pane.view.family();
    let buckets = family.buckets();
    let sub_tabs = bucket_tabs(family, counts);
    let selected = buckets
        .iter()
        .position(|bucket| *bucket == pane.view.bucket)
        .unwrap_or(0);
    let response = panel.show(ui, &sub_tabs, selected, |ui| {
        ui.set_min_width(ui.available_width());
        ui.set_min_height(ui.available_height());
        if takes_slot_filter(pane.view.bucket) {
            show_slot_filter(ui, &mut pane.view.slot_filter, &mut pane.selected);
        }
        show_bucket_grid(ui, pane, counts, db, caches, drag, frame);
    });
    let target = response.clicked.or(if drag.is_some() {
        response.hovered
    } else {
        None
    });
    if let Some(index) = target
        && let Some(bucket) = buckets.get(index).copied()
        && bucket != pane.view.bucket
    {
        pane.view.bucket = bucket;
        pane.selected = None;
    }
    action
}

/// The Relic and Charm buckets' family chips: one toggle per
/// equipment family, any number lit at once, "Any slot" to clear.
/// A change drops the selection, which may just have left the view.
fn show_slot_filter(ui: &mut egui::Ui, filter: &mut SlotFilter, selected: &mut Option<ItemAddr>) {
    let before = filter.clone();
    ui.horizontal_wrapped(|ui| {
        ui.label("Fits into:");
        if ui.selectable_label(filter.is_empty(), "Any slot").clicked() {
            filter.clear();
        }
        for slot in style::GearSlot::ALL {
            let label = univault_core::query::ItemCategory::Gear(slot).label();
            if ui.selectable_label(filter.contains(slot), label).clicked() {
                filter.toggle(slot);
            }
        }
    });
    if *filter != before {
        *selected = None;
    }
}

/// The open bucket's items, sorted and shelf-packed into a scrolling
/// grid. Positions here are scratch: the store files by type, so a
/// bucket is a list, and this layout is only how it is read.
fn show_bucket_grid(
    ui: &mut egui::Ui,
    pane: &mut StorePane,
    counts: &HashMap<Bucket, usize>,
    db: Option<&GameCache>,
    caches: &mut Caches,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) {
    let bucket = pane.view.bucket;
    let sort = pane.view.sort;
    let narrowing = takes_slot_filter(bucket) && !pane.view.slot_filter.is_empty();
    let mut ids: Vec<StoredItemId> = pane
        .store
        .entries()
        .filter(|entry| univault_core::store::bucket_of(db, &entry.item) == bucket)
        .filter(|entry| !narrowing || pane.view.slot_filter.admits(db, &entry.item))
        .map(univault_core::store::StoredEntry::id)
        .collect();
    if narrowing {
        let total = counts.get(&bucket).copied().unwrap_or(0);
        ui.weak(format!("{} of {} fit", ids.len(), total));
    }
    sort_stored(&mut ids, &pane.store, db, sort, &mut caches.names);

    let footprints: Vec<(i32, i32)> = ids
        .iter()
        .filter_map(|id| pane.store.get(*id))
        .map(|item| caches.footprint(db, item))
        .collect();
    let (positions, rows) = shelve_items(&footprints, STORE_COLUMNS);
    for (id, position) in ids.iter().zip(&positions) {
        if let Some(item) = pane.store.get_mut(*id) {
            item.position = *position;
        }
    }
    let entries: Vec<(ItemAddr, &Item)> = ids
        .iter()
        .filter_map(|id| {
            pane.store
                .get(*id)
                .map(|item| (ItemAddr::Stored(*id), item))
        })
        .collect();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            grid_view_store(
                ui,
                (STORE_COLUMNS, rows.max(STORE_MIN_ROWS)),
                &entries,
                &mut pane.selected,
                db,
                caches,
                drag,
                frame,
            );
        });
}

/// The bucket grid keeps the game vault tab's own footprint as its
/// minimum, so a thinly stocked type still paints a full grid instead
/// of a few tiles stranded on bare panel — and an empty one is a
/// plain empty grid that takes drops like any other. A well-stocked
/// type grows past the minimum and scrolls.
const STORE_COLUMNS: i32 = univault_core::vault::TAB_WIDTH;
const STORE_MIN_ROWS: i32 = univault_core::vault::TAB_HEIGHT;

/// Orders a bucket's ids for display. Ranks are always built
/// ascending and oriented once by the pane's direction; name is the
/// tiebreak everywhere, so the order is total and stable frame to
/// frame.
fn sort_stored(
    ids: &mut [StoredItemId],
    store: &VaultStore,
    db: Option<&GameCache>,
    sort: StoreSort,
    names: &mut NameCache,
) {
    let mut keyed: Vec<((i32, String), StoredItemId)> = Vec::with_capacity(ids.len());
    for id in ids.iter() {
        let Some(item) = store.get(*id) else {
            keyed.push(((0, String::new()), *id));
            continue;
        };
        let name = names.item_label(db, item).to_lowercase();
        let rank = match sort.key {
            StoreSortKey::Name => 0,
            StoreSortKey::Rarity => db.map_or(0, |db| {
                i32::from(style::item_style(Some(db), item).rarity_rank())
            }),
            StoreSortKey::Level => db.map_or(0, |db| {
                stats::item_requirements(db, item)
                    .into_iter()
                    .find(|(key, _)| *key == stats::Requirement::Level)
                    .map_or(0, |(_, value)| value)
            }),
        };
        keyed.push(((rank, name), *id));
    }
    keyed.sort_by(|a, b| sort.direction.apply(a.0.cmp(&b.0)));
    for (slot, (_, id)) in ids.iter_mut().zip(keyed) {
        *slot = id;
    }
}

/// The store's grid: the same painting and gestures as a container
/// grid, but every drop lands in the store rather than a cell.
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn grid_view_store(
    ui: &mut egui::Ui,
    dims: (i32, i32),
    entries: &[(ItemAddr, &Item)],
    selected: &mut Option<ItemAddr>,
    db: Option<&GameCache>,
    caches: &mut Caches,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) {
    // Sized off the *minimum* grid, so the cell stays put whether the
    // bucket is half empty or scrolling well past the fold; a taller
    // bucket grows downward at the same scale instead of shrinking to
    // fit. Centered like every other grid — hugging the top-left
    // reads as a layout slip.
    let cell = fit_cell_size(ui.available_size(), (dims.0, STORE_MIN_ROWS));
    let size = egui::vec2(cells_at(dims.0, cell), cells_at(dims.1, cell));
    let pad = ((ui.available_width() - size.x) / 2.0).max(0.0);
    let (rect, response) = ui
        .horizontal(|ui| {
            ui.add_space(pad);
            ui.allocate_exact_size(size, egui::Sense::click_and_drag())
        })
        .inner;
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();
    let grid_chrome = caches.chrome(ui.ctx(), db);
    paint_grid_background(&painter, grid_chrome.as_ref(), rect, dims, cell);

    let press_origin = ui.ctx().input(|input| input.pointer.press_origin());
    let mut hovered: Option<&Item> = None;
    for (addr, item) in entries {
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
        paint_item_tile(
            ui,
            &painter,
            item_rect,
            item,
            *selected == Some(*addr),
            &visuals,
            db,
            caches,
        );
        if drag.is_some_and(|state| state.source == *addr) {
            painter.rect_filled(item_rect, 2.0, egui::Color32::from_black_alpha(140));
        }
        if response.drag_started()
            && drag.is_none()
            && frame.begin.is_none()
            && press_origin.is_some_and(|origin| item_rect.contains(origin))
        {
            frame.begin = Some(DragState {
                source: *addr,
                item: (*item).clone(),
                grab: press_origin.map_or(egui::Vec2::ZERO, |origin| origin - item_rect.min),
            });
        }
        if drag.is_none()
            && let Some(pointer) = response.hover_pos()
            && item_rect.contains(pointer)
        {
            hovered = Some(item);
            item_gestures(ui, &response, *addr, selected, frame);
        }
    }
    if let Some(item) = hovered {
        egui::Tooltip::for_widget(&response)
            .at_pointer()
            .show(|ui| item_tooltip(ui, item, db, caches));
    }
    if let Some(state) = drag
        && let Some(pointer) = ui.ctx().pointer_latest_pos()
        && rect.contains(pointer)
    {
        frame.candidate = Some(paint_drop_preview(
            &painter,
            rect,
            cell,
            dims,
            entries,
            DropTarget::Store,
            state,
            pointer,
            db,
            caches,
        ));
    }
}

fn pick_file(description: &str, extensions: &[&str], start: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().add_filter(description, extensions);
    if let Some(start) = start {
        dialog = dialog.set_directory(start);
    }
    dialog.pick_file()
}

/// Where an export is written — the save dialog, pre-named.
fn save_file(description: &str, extension: &str, suggested: String) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .add_filter(description, &[extension])
        .set_file_name(suggested);
    if let Some(dir) = vaults_dir() {
        let _ = std::fs::create_dir_all(&dir);
        dialog = dialog.set_directory(dir);
    }
    dialog.save_file()
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
    fn a_failed_reload_is_reported_only_once_it_persists() {
        let mut tracker = RefreshTracker::default();
        let path = Path::new("watched");
        for _ in 1..RELOAD_PATIENCE {
            assert_eq!(tracker.reload_failed(path), ReloadFailure::Transient);
            // Every attempt reopens the file, which forgets the
            // pending observation but must not forget the count.
            tracker.forget(path);
        }
        assert_eq!(tracker.reload_failed(path), ReloadFailure::Persisting);
        assert_eq!(tracker.reload_failed(path), ReloadFailure::Persisting);
        tracker.reload_succeeded(path);
        assert_eq!(tracker.reload_failed(path), ReloadFailure::Transient);
        assert_eq!(
            tracker.reload_failed(Path::new("other")),
            ReloadFailure::Transient
        );
    }

    #[test]
    fn a_backlog_of_polls_settles_without_waiting_for_more() {
        // What a hidden window accumulates: the same changed stamp
        // observed over and over. Consuming the backlog in order must
        // settle it, which is why drive_refresh feeds every queued
        // poll rather than only the newest.
        let mut tracker = RefreshTracker::default();
        let path = Path::new("watched");
        let ours = stamp(10, 100);
        let changed = stamp(20, 200);
        let backlog = [&changed; 30];
        let settles = backlog
            .iter()
            .filter(|observed| tracker.settled(path, Some(observed), Some(&ours)))
            .count();
        assert_eq!(settles, backlog.len() - 1, "only the first arms the check");
    }

    #[test]
    fn an_emptied_bank_is_held_back_but_never_forever() {
        let mut tracker = RefreshTracker::default();
        let shared = Path::new("Sys/winsys.dxb");
        let bank = Path::new("_Sif/winsys.dxb");
        for attempt in 1..=EMPTY_PATIENCE {
            assert!(tracker.defer_empty(shared), "held on attempt {attempt}");
        }
        // A twin that never catches up must not pin the pane forever.
        assert!(!tracker.defer_empty(shared), "patience is bounded");

        // Deferral is per file: one bank emptying says nothing about
        // another that emptied in the same poll.
        assert!(tracker.defer_empty(bank));

        // Once adopted, a later emptying is questioned afresh rather
        // than waved through on the spent budget.
        tracker.empty_settled(shared);
        assert!(tracker.defer_empty(shared));
    }

    #[test]
    fn watch_health_reports_a_stall_after_it_ends() {
        let start = Instant::now();
        let mut health = WatchHealth::default();
        assert_eq!(health.summary(start), "watching: starting…");
        assert!(health.stalled_now(start), "nothing checked yet reads stale");

        health.checked(start);
        assert!(!health.stalled_now(start + Duration::from_secs(2)));
        assert_eq!(
            health.summary(start + Duration::from_secs(2)),
            "watching: checked 2s ago"
        );

        // A gap the paint loop stopped through, then a resume: the
        // pause is still reported once polls are landing again.
        let resumed = start + STALL_THRESHOLD + Duration::from_secs(65);
        assert!(health.stalled_now(resumed));
        health.checked(resumed);
        assert_eq!(
            health.summary(resumed),
            "watching: checked 0s ago · longest pause 1m15s (window hidden?)"
        );
        // Ordinary cadence afterwards neither clears nor worsens it.
        health.checked(resumed + Duration::from_secs(2));
        assert!(
            health
                .summary(resumed + Duration::from_secs(2))
                .contains("1m15s")
        );
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
    fn the_store_grid_sorts_both_ways() {
        let mut store = VaultStore::new();
        let ids: Vec<StoredItemId> = ["charlie", "alpha", "bravo"]
            .into_iter()
            .map(|stem| {
                store.add(Item::bare(
                    RecordId::parse(format!("records\\{stem}.dbr")).unwrap(),
                    chr::ItemSeed::new(1),
                ))
            })
            .collect();
        let mut names = NameCache::default();
        let ordered = |direction, names: &mut NameCache| {
            let mut ids = ids.clone();
            sort_stored(
                &mut ids,
                &store,
                None,
                StoreSort {
                    key: StoreSortKey::Name,
                    direction,
                },
                names,
            );
            ids.iter()
                .filter_map(|id| store.get(*id))
                .map(|item| names.item_label(None, item))
                .collect::<Vec<String>>()
        };
        assert_eq!(
            ordered(SortDirection::Ascending, &mut names),
            ["alpha", "bravo", "charlie"]
        );
        assert_eq!(
            ordered(SortDirection::Descending, &mut names),
            ["charlie", "bravo", "alpha"]
        );
    }

    fn seeded(stem: &str, seed: i32) -> Item {
        Item::bare(
            RecordId::parse(format!("records\\{stem}.dbr")).unwrap(),
            chr::ItemSeed::new(seed),
        )
    }

    fn guard_over(stored: &[(&str, i32)]) -> DuplicateGuard {
        let mut store = VaultStore::new();
        for (stem, seed) in stored {
            store.add(seeded(stem, *seed));
        }
        DuplicateGuard::over(&store, None)
    }

    #[test]
    fn a_guarded_move_leaves_duplicates_where_they_are() {
        let mut guard = guard_over(&[("helm", 41823)]);
        let mut source = vec![seeded("helm", 41823), seeded("helm", 90210)];
        let taken = drain_or_clone(&mut source, BulkMode::Move, None, Some(&mut guard));
        assert_eq!(
            taken
                .iter()
                .map(|item| item.seed.value())
                .collect::<Vec<_>>(),
            [90210]
        );
        assert_eq!(
            source
                .iter()
                .map(|item| item.seed.value())
                .collect::<Vec<_>>(),
            [41823],
            "a skipped duplicate is never drained out of the source"
        );
        assert_eq!(guard.skipped(), 1);
    }

    #[test]
    fn a_guarded_copy_takes_fresh_seeds_and_never_touches_the_source() {
        let mut guard = guard_over(&[("helm", 41823)]);
        let mut source = vec![seeded("helm", 41823), seeded("helm", 90210)];
        let taken = drain_or_clone(&mut source, BulkMode::Copy, None, Some(&mut guard));
        assert_eq!(taken.len(), 1);
        assert_eq!(source.len(), 2);
        assert_eq!(guard.skipped(), 1);
    }

    #[test]
    fn one_batch_cannot_duplicate_within_itself() {
        let mut guard = guard_over(&[]);
        let mut source = vec![seeded("helm", 7), seeded("helm", 7), seeded("helm", 7)];
        let taken = drain_or_clone(&mut source, BulkMode::Move, None, Some(&mut guard));
        assert_eq!(taken.len(), 1);
        assert_eq!(source.len(), 2);
        assert_eq!(guard.skipped(), 2);
    }

    #[test]
    fn without_the_box_a_bulk_send_still_takes_everything() {
        let mut source = vec![seeded("helm", 7), seeded("helm", 7)];
        let taken = drain_or_clone(&mut source, BulkMode::Move, None, None);
        assert_eq!(taken.len(), 2);
        assert!(source.is_empty());
        let mut source = vec![seeded("helm", 7), seeded("helm", 7)];
        let copied = drain_or_clone(&mut source, BulkMode::Copy, None, None);
        assert_eq!(copied.len(), 2);
        assert_eq!(source.len(), 2);
    }

    #[test]
    fn the_skipped_note_stays_grammatical() {
        assert_eq!(skipped_note(0), "");
        assert_eq!(skipped_note(1), "; 1 skipped as a duplicate");
        assert_eq!(skipped_note(4), "; 4 skipped as duplicates");
    }

    #[test]
    fn a_fresh_store_key_opens_in_its_natural_direction() {
        assert_eq!(StoreSortKey::Name.natural(), SortDirection::Ascending);
        assert_eq!(StoreSortKey::Rarity.natural(), SortDirection::Descending);
        assert_eq!(StoreSortKey::Level.natural(), SortDirection::Descending);
        assert_eq!(StoreSort::default().direction, SortDirection::Ascending);
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
    fn the_store_lives_under_the_config_dir() {
        let path = store_path().expect("a config dir on a supported platform");
        assert!(path.ends_with("vault-store.json"), "{path:?}");
        assert!(
            path.starts_with(univault_core::platform::config_dir().unwrap()),
            "{path:?}"
        );
    }

    #[test]
    fn shelves_wrap_in_reading_order_and_report_their_rows() {
        // Two 2×5 swords then a 1×1 potion across 4 columns: the
        // third item shares the first shelf, the fourth wraps.
        let (positions, rows) = shelve_items(&[(2, 5), (2, 5), (1, 1), (3, 2)], 4);
        assert_eq!(
            positions,
            [
                univault_core::chr::GridPos { x: 0, y: 0 },
                univault_core::chr::GridPos { x: 2, y: 0 },
                univault_core::chr::GridPos { x: 0, y: 5 },
                univault_core::chr::GridPos { x: 1, y: 5 },
            ]
        );
        assert_eq!(rows, 7);
    }

    #[test]
    fn an_oversized_item_still_gets_its_own_shelf() {
        let (positions, rows) = shelve_items(&[(9, 3)], 4);
        assert_eq!(positions, [univault_core::chr::GridPos { x: 0, y: 0 }]);
        assert_eq!(rows, 3);
    }

    #[test]
    fn taking_an_item_re_aims_only_later_targets_in_the_same_grid() {
        let grid = GridId::Bank;
        let source = ItemAddr::grid(grid, 2);
        // Later in the same container: shifts down one.
        assert_eq!(
            shift_after_take(ItemAddr::grid(grid, 5), source),
            ItemAddr::grid(grid, 4)
        );
        // Earlier, another container, and the store: untouched.
        assert_eq!(
            shift_after_take(ItemAddr::grid(grid, 1), source),
            ItemAddr::grid(grid, 1)
        );
        assert_eq!(
            shift_after_take(ItemAddr::grid(GridId::Shared, 5), source),
            ItemAddr::grid(GridId::Shared, 5)
        );
        let stored = ItemAddr::Stored(VaultStore::new().add(Item::bare(
            RecordId::parse("records\\a.dbr".to_string()).unwrap(),
            univault_core::chr::ItemSeed::new(1),
        )));
        assert_eq!(shift_after_take(stored, source), stored);
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

    #[test]
    fn slot_filter_keeps_family_order_whatever_the_click_order() {
        use style::GearSlot;
        let mut filter = SlotFilter::default();
        assert!(filter.is_empty());
        filter.toggle(GearSlot::Staff);
        filter.toggle(GearSlot::Head);
        filter.toggle(GearSlot::Ring);
        assert_eq!(
            filter.slots,
            vec![GearSlot::Head, GearSlot::Ring, GearSlot::Staff]
        );
        filter.toggle(GearSlot::Ring);
        assert_eq!(filter.slots, vec![GearSlot::Head, GearSlot::Staff]);
        assert!(filter.contains(GearSlot::Head));
        assert!(!filter.contains(GearSlot::Ring));
        filter.clear();
        assert!(filter.is_empty());
    }

    #[test]
    fn an_empty_slot_filter_admits_everything_and_a_lit_one_needs_the_database() {
        let relic = seeded("relic", 1);
        let mut filter = SlotFilter::default();
        assert!(filter.admits(None, &relic));
        filter.toggle(style::GearSlot::Head);
        assert!(!filter.admits(None, &relic));
    }

    #[test]
    fn only_the_socketable_buckets_take_the_slot_filter() {
        use univault_core::query::ItemCategory;
        assert!(takes_slot_filter(Bucket::Category(ItemCategory::Relic)));
        assert!(takes_slot_filter(Bucket::Category(ItemCategory::Charm)));
        assert!(!takes_slot_filter(Bucket::Category(ItemCategory::Gear(
            style::GearSlot::Head
        ))));
        assert!(!takes_slot_filter(Bucket::Unknown));
    }

    #[test]
    fn the_default_store_view_opens_on_helmets() {
        let view = StoreView::default();
        assert_eq!(view.family(), Family::Armor);
        assert_eq!(view.bucket, Family::Armor.buckets()[0]);
        assert!(view.slot_filter.is_empty());
        assert!(!view.skip_duplicate_seeds);
    }
}
