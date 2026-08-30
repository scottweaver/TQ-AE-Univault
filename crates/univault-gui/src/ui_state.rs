//! View state that survives a restart: which tabs are open, the
//! store pane's ordering and filters, the search bar. One small
//! self-describing JSON file (`ui-state.json`, format tag
//! `univault-ui-state`) under the config directory, beside the
//! store. It is a convenience, never data: an unreadable, foreign,
//! or newer file is ignored and the app starts from defaults.
//!
//! Writes are debounced — the state is snapshotted every frame and
//! written once it has held still for [`QUIET`], so typing into the
//! search bar does not hit the disk per keystroke — and flushed on
//! exit.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::search::SearchSettings;
use crate::{App, InventoryTab, LeftTab, MainView, StoreView};

const FORMAT: &str = "univault-ui-state";
const VERSION: u32 = 1;
const FILE_NAME: &str = "ui-state.json";

/// How long the state must hold still before it is written.
const QUIET: Duration = Duration::from_secs(1);

/// Everything the file holds. Fields default individually, so a
/// file from an older build that lacks one still restores the rest.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct UiState {
    pub(crate) left_tab: LeftTab,
    pub(crate) inventory_tab: InventoryTab,
    pub(crate) view: MainView,
    pub(crate) store: StoreView,
    pub(crate) search: SearchSettings,
}

/// The on-disk shape: the identity tags around the state.
#[derive(Serialize, Deserialize)]
struct FileDto {
    format: String,
    version: u32,
    #[serde(flatten)]
    state: UiState,
}

impl UiState {
    fn from_json(text: &str) -> Option<Self> {
        let dto: FileDto = serde_json::from_str(text).ok()?;
        (dto.format == FORMAT && dto.version <= VERSION).then_some(dto.state)
    }

    fn to_json(&self) -> String {
        let dto = FileDto {
            format: FORMAT.to_string(),
            version: VERSION,
            state: self.clone(),
        };
        serde_json::to_string_pretty(&dto).unwrap_or_default()
    }
}

/// The file and what it holds, plus the pending-change clock the
/// debounce runs on.
pub(crate) struct UiStateFile {
    file: Option<PathBuf>,
    on_disk: UiState,
    /// When the live state first diverged from `on_disk`; cleared by
    /// a write or by the state returning to what is on disk.
    pending_since: Option<Instant>,
}

impl UiStateFile {
    /// Opens the file under the config directory, restoring whatever
    /// it validly holds.
    pub(crate) fn load() -> Self {
        Self::at(univault_core::platform::config_dir().map(|dir| dir.join(FILE_NAME)))
    }

    fn at(file: Option<PathBuf>) -> Self {
        let on_disk = file
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| UiState::from_json(&text))
            .unwrap_or_default();
        Self {
            file,
            on_disk,
            pending_since: None,
        }
    }

    /// The state restored at load (or last written).
    pub(crate) fn on_disk(&self) -> &UiState {
        &self.on_disk
    }

    /// Feeds this frame's live state. A change is written once it
    /// has held still for [`QUIET`]; until then the wait remaining
    /// is returned so the caller can schedule the next look.
    pub(crate) fn observe(&mut self, current: UiState, now: Instant) -> Option<Duration> {
        if current == self.on_disk {
            self.pending_since = None;
            return None;
        }
        let since = *self.pending_since.get_or_insert(now);
        let elapsed = now.saturating_duration_since(since);
        if elapsed < QUIET {
            return Some(QUIET.saturating_sub(elapsed));
        }
        self.write(current);
        None
    }

    /// Writes any pending change now — the exit path.
    pub(crate) fn flush(&mut self, current: UiState) {
        if current != self.on_disk {
            self.write(current);
        }
    }

    fn write(&mut self, current: UiState) {
        if let Some(file) = &self.file {
            if let Some(parent) = file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(file, current.to_json());
        }
        self.on_disk = current;
        self.pending_since = None;
    }
}

impl App {
    /// The live view state, as it would be written.
    pub(crate) fn ui_snapshot(&self) -> UiState {
        UiState {
            left_tab: self.active_tab,
            inventory_tab: self.inventory_tab,
            view: self.view,
            store: self.store.as_ref().map_or_else(
                || self.ui_state.on_disk().store.clone(),
                |pane| pane.view.clone(),
            ),
            search: self.search.settings(),
        }
    }

    /// Persists the view state once it has held still; called at the
    /// end of every frame.
    pub(crate) fn persist_ui_state(&mut self, ctx: &eframe::egui::Context) {
        let current = self.ui_snapshot();
        if let Some(wait) = self.ui_state.observe(current, Instant::now()) {
            ctx.request_repaint_after(wait);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SlotFilter, StoreSort, StoreSortKey};
    use univault_core::query::ItemCategory;
    use univault_core::store::Bucket;
    use univault_core::style::GearSlot;

    fn changed() -> UiState {
        let mut slot_filter = SlotFilter::default();
        slot_filter.toggle(GearSlot::Ring);
        slot_filter.toggle(GearSlot::Head);
        UiState {
            left_tab: LeftTab::Shared,
            inventory_tab: InventoryTab::Sack(2),
            view: MainView::Search,
            store: StoreView {
                bucket: Bucket::Category(ItemCategory::Charm),
                sort: StoreSort::by(StoreSortKey::Rarity),
                skip_duplicate_seeds: true,
                slot_filter,
            },
            search: SearchSettings::default(),
        }
    }

    #[test]
    fn state_round_trips_through_its_file_shape() {
        let state = changed();
        let json = state.to_json();
        assert!(json.contains("\"format\": \"univault-ui-state\""), "{json}");
        assert!(json.contains("\"version\": 1"), "{json}");
        assert_eq!(UiState::from_json(&json), Some(state));
    }

    #[test]
    fn foreign_or_newer_files_are_ignored() {
        assert_eq!(
            UiState::from_json(r#"{"format":"univault-store","version":1}"#),
            None
        );
        assert_eq!(
            UiState::from_json(r#"{"format":"univault-ui-state","version":2}"#),
            None
        );
        assert_eq!(UiState::from_json("not json"), None);
    }

    #[test]
    fn missing_fields_restore_the_rest() {
        let partial = r#"{"format":"univault-ui-state","version":1,"leftTab":"Relic"}"#;
        let state = UiState::from_json(partial).expect("parses");
        assert_eq!(state.left_tab, LeftTab::Relic);
        assert_eq!(state.store, StoreView::default());
        assert_eq!(state.view, MainView::Panes);
    }

    #[test]
    fn a_change_is_written_only_after_it_holds_still() {
        let dir = std::env::temp_dir().join(format!("univault-ui-state-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        let mut file = UiStateFile::at(Some(path.clone()));
        let start = Instant::now();
        assert_eq!(file.observe(UiState::default(), start), None);
        assert!(!path.exists());

        let wait = file.observe(changed(), start).expect("pending");
        assert_eq!(wait, QUIET);
        assert!(!path.exists());
        let wait = file
            .observe(changed(), start + QUIET / 2)
            .expect("still pending");
        assert_eq!(wait, QUIET / 2);

        assert_eq!(file.observe(changed(), start + QUIET), None);
        let reloaded = UiStateFile::at(Some(path.clone()));
        assert_eq!(*reloaded.on_disk(), changed());

        // Reverting to what is on disk cancels the pending write.
        let mut stale = changed();
        stale.left_tab = LeftTab::Bank;
        assert!(file.observe(stale, start + QUIET).is_some());
        assert_eq!(file.observe(changed(), start + QUIET * 3), None);
        assert_eq!(*UiStateFile::at(Some(path.clone())).on_disk(), changed());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn flush_writes_without_waiting() {
        let dir = std::env::temp_dir().join(format!("univault-ui-flush-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        let mut file = UiStateFile::at(Some(path.clone()));
        file.flush(changed());
        assert_eq!(*UiStateFile::at(Some(path)).on_disk(), changed());
        let _ = std::fs::remove_dir_all(dir);
    }
}
