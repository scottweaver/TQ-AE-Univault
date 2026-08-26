//! The all-vaults search view: every vault file in the vaults folder
//! flattened into one filtered, sorted table (icon | name | rarity |
//! req | vault | stats). Rows are addressable through [`GridId`], so
//! the standard gestures act on them — right-click sends to the
//! active left tab, Shift+Click duplicates in place, Alt+Click
//! extracts a socketed piece, double-click jumps to the item in the
//! vault pane. Vaults loaded here ride the same autosave/refresh/
//! conflict rails as the panes ([`DocId::SearchVault`]).

use std::path::{Path, PathBuf};

use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::BTreeSet;
use univault_core::cache::{GameCache, SourceStamp};
use univault_core::chr::Item;

use univault_core::query::{self, Expansion, Filter, ItemCategory, ValueBounds};
use univault_core::stats::{self, Requirement};
use univault_core::style::{self, ItemStyle};
use univault_core::vault::Vault;

use crate::{
    App, DocId, GameStatus, GridId, MainView, VaultPane, game_color, item_tooltip, stamp_of,
    vaults_dir,
};

/// One vault file loaded by the search view (never the one open in
/// the vault pane — that one contributes its in-memory state).
pub(crate) struct SearchDoc {
    pub(crate) path: PathBuf,
    pub(crate) vault: Vault,
    pub(crate) dirty: bool,
    pub(crate) disk_stamp: Option<SourceStamp>,
}

/// The search view's whole state; lives on [`App`] so loaded vaults
/// stay warm (and autosaving) across view switches.
#[derive(Default)]
pub(crate) struct SearchState {
    pub(crate) docs: Vec<SearchDoc>,
    pub(crate) stale: bool,
    /// The suggestion vocabularies lag behind `stale`: they rebuild
    /// only when item data changed, never on a filter keystroke.
    vocab_stale: bool,
    vocab_stats: Vec<String>,
    vocab_affixes: Vec<String>,
    draft: FilterDraft,
    sort: SortSpec,
    source_filter: Option<RowSource>,
    rows: Vec<SearchRow>,
    total: usize,
    selected: Option<(GridId, usize)>,
}

impl SearchState {
    /// Item data changed (edit, reload, rescan, adoption): rows and
    /// suggestion vocabularies both need a rebuild.
    pub(crate) fn mark_data_changed(&mut self) {
        self.stale = true;
        self.vocab_stale = true;
    }
}

/// Which vault a row came from: the open pane or a loaded doc.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowSource {
    Open,
    Doc(usize),
}

/// Widget state of the filter bar; empty text fields (and
/// unparseable numbers) mean "no filter".
#[derive(Default, Clone, PartialEq)]
struct FilterDraft {
    name: String,
    /// The dynamic stat/affix criteria — as many rows as the user
    /// adds, each its own conjunct.
    criteria: Vec<CriterionDraft>,
    req_level: String,
    req_strength: String,
    req_dexterity: String,
    req_intelligence: String,
    set: String,
    set_only: bool,
    style: Option<ItemStyle>,
    category: Option<ItemCategory>,
    socketed: Option<bool>,
    origin: OriginDraft,
}

/// One criterion row: what to match, where, and the value window. A
/// row with nothing filled in is inert.
#[derive(Default, Clone, PartialEq)]
struct CriterionDraft {
    scope: CriterionScope,
    text: String,
    min: String,
    max: String,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum CriterionScope {
    /// Any stat line on the item (base, affix, relic, bonus).
    #[default]
    AnyStat,
    /// Only lines an affix grants.
    AffixStat,
    /// The affix's name itself (presence — no value window).
    AffixName,
}

impl CriterionScope {
    const ALL: [Self; 3] = [Self::AnyStat, Self::AffixStat, Self::AffixName];

    fn label(self) -> &'static str {
        match self {
            Self::AnyStat => "Any stat",
            Self::AffixStat => "Affix stat",
            Self::AffixName => "Affix name",
        }
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum OriginDraft {
    #[default]
    Any,
    BaseGame,
    Expansion(Expansion),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Name,
    Rarity,
    ReqLevel,
    Location,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SortSpec {
    key: SortKey,
    ascending: bool,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            ascending: true,
        }
    }
}

/// One table row, with everything display and sorting need cached at
/// rebuild time.
struct SearchRow {
    grid: GridId,
    index: usize,
    item: Item,
    name: String,
    name_sort: String,
    item_style: ItemStyle,
    rarity_rank: u8,
    req_level: Option<i32>,
    location: String,
    details: stats::ItemDetails,
    height: f32,
}

/// Row gestures reported back to the main loop, which routes them
/// through the same handlers the panes use.
#[derive(Default)]
pub(crate) struct SearchFrame {
    pub(crate) duplicate: Option<(GridId, usize)>,
    pub(crate) quick_move: Option<(GridId, usize)>,
    pub(crate) copy_across: Option<(GridId, usize)>,
    pub(crate) extract: Option<(GridId, usize)>,
    pub(crate) jump: Option<(GridId, usize)>,
    pub(crate) leave: bool,
}

fn filter_field(ui: &mut egui::Ui, value: &mut String, hint: &str, width: f32) {
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(width),
    );
}

fn filter_text_row(ui: &mut egui::Ui, draft: &mut FilterDraft) {
    ui.horizontal_wrapped(|ui| {
        filter_field(ui, &mut draft.name, "Name contains…", 170.0);
        filter_field(ui, &mut draft.set, "Set name…", 140.0);
        ui.checkbox(&mut draft.set_only, "set items only");
    });
}

/// How many suggestions the autocomplete popup offers at once.
const SUGGESTION_LIMIT: usize = 8;

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// A text field with an autocomplete popup fed from `vocab`. The
/// popup also renders on the frame focus is lost so a click on a
/// suggestion lands before it closes.
fn suggesting_field(
    ui: &mut egui::Ui,
    id_salt: (&str, usize),
    value: &mut String,
    hint: &str,
    width: f32,
    vocab: &[String],
) {
    let response = ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(width),
    );
    if !(response.has_focus() || response.lost_focus()) {
        return;
    }
    let matching: Vec<&String> = vocab
        .iter()
        .filter(|entry| contains_ci(entry, value))
        .take(SUGGESTION_LIMIT)
        .collect();
    if matching.is_empty() || (matching.len() == 1 && *matching[0] == *value) {
        return;
    }
    egui::Area::new(egui::Id::new(id_salt).with("suggestions"))
        .order(egui::Order::Foreground)
        .fixed_pos(response.rect.left_bottom() + egui::vec2(0.0, 4.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(response.rect.width());
                for entry in matching {
                    if ui.selectable_label(false, entry).clicked() {
                        value.clone_from(entry);
                    }
                }
            });
        });
}

/// The suggestion vocabularies over every loaded vault: distinct
/// stat-line templates (numbers replaced by `#`) and affix names.
fn collect_vocab<'a>(
    db: &GameCache,
    vaults: impl Iterator<Item = &'a Vault>,
) -> (Vec<String>, Vec<String>) {
    let mut stats = BTreeSet::new();
    let mut affixes = BTreeSet::new();
    for vault in vaults {
        for sack in &vault.sacks {
            for entry in &sack.items {
                let item = &entry.item;
                for line in query::stat_lines(db, item) {
                    if !line.text.trim().is_empty() {
                        stats.insert(query::stat_template(&line.text));
                    }
                }
                for affix in [item.prefix.as_ref(), item.suffix.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    affixes.insert(query::record_name(Some(db), affix));
                }
            }
        }
    }
    (stats.into_iter().collect(), affixes.into_iter().collect())
}

/// A vault file's display name: its file stem.
pub(crate) fn vault_label(path: &Path) -> String {
    path.file_stem().map_or_else(
        || path.display().to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

/// Reads one vault file for the search view.
pub(crate) fn load_search_doc(path: &Path) -> Result<SearchDoc, String> {
    let disk_stamp = stamp_of(path);
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let vault = Vault::from_json(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(SearchDoc {
        path: path.to_path_buf(),
        vault,
        dirty: false,
        disk_stamp,
    })
}

/// The vault files in the vaults folder, sorted by name.
fn list_vault_files() -> Vec<PathBuf> {
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

fn draft_filters(draft: &FilterDraft) -> Vec<Filter> {
    let text = |field: &str| {
        let trimmed = field.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    };
    let number = |field: &str| field.trim().parse::<f32>().ok();
    let mut filters = Vec::new();
    if let Some(name) = text(&draft.name) {
        filters.push(Filter::NameContains(name));
    }
    for criterion in &draft.criteria {
        let trimmed = criterion.text.trim().to_string();
        let bounds = ValueBounds {
            min: number(&criterion.min),
            max: number(&criterion.max),
        };
        let inert = trimmed.is_empty() && bounds.is_unbounded();
        match criterion.scope {
            CriterionScope::AffixName => {
                if !trimmed.is_empty() {
                    filters.push(Filter::HasAffix(trimmed));
                }
            }
            CriterionScope::AnyStat => {
                if !inert {
                    filters.push(Filter::StatContains {
                        text: trimmed,
                        bounds,
                    });
                }
            }
            CriterionScope::AffixStat => {
                if !inert {
                    filters.push(Filter::AffixStat {
                        text: trimmed,
                        bounds,
                    });
                }
            }
        }
    }
    for (field, requirement) in [
        (&draft.req_level, Requirement::Level),
        (&draft.req_strength, Requirement::Strength),
        (&draft.req_dexterity, Requirement::Dexterity),
        (&draft.req_intelligence, Requirement::Intelligence),
    ] {
        if let Ok(cap) = field.trim().parse::<i32>() {
            filters.push(Filter::RequirementAtMost(requirement, cap));
        }
    }
    if let Some(set) = text(&draft.set) {
        filters.push(Filter::InSet(set));
    } else if draft.set_only {
        filters.push(Filter::InSet(String::new()));
    }
    if let Some(item_style) = draft.style {
        filters.push(Filter::Style(item_style));
    }
    if let Some(category) = draft.category {
        filters.push(Filter::Category(category));
    }
    if let Some(socketed) = draft.socketed {
        filters.push(Filter::Socketed(socketed));
    }
    match draft.origin {
        OriginDraft::Any => {}
        OriginDraft::BaseGame => filters.push(Filter::Origin(None)),
        OriginDraft::Expansion(expansion) => filters.push(Filter::Origin(Some(expansion))),
    }
    filters
}

/// Ascending rarity order for the sortable column.
fn style_rank(item_style: ItemStyle) -> u8 {
    match item_style {
        ItemStyle::Broken => 0,
        ItemStyle::Mundane => 1,
        ItemStyle::Common => 2,
        ItemStyle::Rare => 3,
        ItemStyle::Epic => 4,
        ItemStyle::Legendary => 5,
        ItemStyle::Quest => 6,
        ItemStyle::Potion => 7,
        ItemStyle::Scroll => 8,
        ItemStyle::Parchment => 9,
        ItemStyle::Relic => 10,
        ItemStyle::Formulae => 11,
        ItemStyle::Artifact => 12,
    }
}

/// Height budget per stat line: a size-12 galley (~15px) plus the
/// 4px cell spacing, with slack — over-estimating pads, while
/// under-estimating clips the cell.
const LINE_HEIGHT: f32 = 20.0;
const BLOCK_GAP: f32 = 5.0;
const ROW_PAD: f32 = 8.0;
const ICON_SIZE: f32 = 32.0;

// Line and block counts are tiny; f32 represents them exactly.
#[allow(clippy::cast_precision_loss)]
fn row_height(details: &stats::ItemDetails) -> f32 {
    let lines: usize = details.blocks.iter().map(Vec::len).sum();
    let gaps = details.blocks.len().saturating_sub(1);
    (lines as f32)
        .mul_add(LINE_HEIGHT, (gaps as f32) * BLOCK_GAP + ROW_PAD)
        .max(ICON_SIZE + ROW_PAD)
}

fn sort_rows(rows: &mut [SearchRow], sort: SortSpec) {
    rows.sort_by(|a, b| {
        let by_name = a.name_sort.cmp(&b.name_sort);
        let ordering = match sort.key {
            SortKey::Name => by_name,
            SortKey::Rarity => a.rarity_rank.cmp(&b.rarity_rank).then(by_name),
            SortKey::ReqLevel => a
                .req_level
                .unwrap_or(0)
                .cmp(&b.req_level.unwrap_or(0))
                .then(by_name),
            SortKey::Location => a.location.cmp(&b.location).then(by_name),
        };
        if sort.ascending {
            ordering
        } else {
            ordering.reverse()
        }
    });
}

#[allow(clippy::too_many_arguments)] // one call surface, view-internal
fn collect_rows(
    db: &GameCache,
    filters: &[Filter],
    vault: &Vault,
    source: RowSource,
    label: &str,
    total: &mut usize,
    rows: &mut Vec<SearchRow>,
) {
    for (tab, sack) in vault.sacks.iter().enumerate() {
        for (index, entry) in sack.items.iter().enumerate() {
            let item = &entry.item;
            *total += 1;
            if !query::matches(db, item, filters) {
                continue;
            }
            let details = stats::item_details(db, item);
            let item_style = style::item_style(Some(db), item);
            let name = query::item_name(Some(db), item);
            let req_level = stats::item_requirements(db, item)
                .into_iter()
                .find(|(key, _)| *key == Requirement::Level)
                .map(|(_, value)| value);
            rows.push(SearchRow {
                grid: match source {
                    RowSource::Open => GridId::VaultTab(tab),
                    RowSource::Doc(doc) => GridId::SearchDoc { doc, tab },
                },
                index,
                item: item.clone(),
                name_sort: name.to_lowercase(),
                name,
                rarity_rank: style_rank(item_style),
                item_style,
                req_level,
                location: format!("{label} · tab {}", tab + 1),
                height: row_height(&details),
                details,
            });
        }
    }
}

impl App {
    /// Switches to the search view, re-listing the vaults folder.
    pub(crate) fn enter_search(&mut self) {
        self.view = MainView::Search;
        self.rescan_search_docs();
    }

    /// Re-lists the vaults folder: keeps the in-memory state of
    /// already-loaded files, loads new ones, drops clean docs whose
    /// file vanished (dirty ones stay — autosave recreates them).
    /// The vault open in the pane is excluded; its rows come from
    /// the pane's live model.
    pub(crate) fn rescan_search_docs(&mut self) {
        let mut kept = std::mem::take(&mut self.search.docs);
        let mut docs = Vec::new();
        let mut failures = Vec::new();
        for path in list_vault_files() {
            if self.right.as_ref().is_some_and(|pane| pane.path == path) {
                continue;
            }
            if let Some(position) = kept.iter().position(|doc| doc.path == path) {
                docs.push(kept.swap_remove(position));
            } else {
                match load_search_doc(&path) {
                    Ok(doc) => docs.push(doc),
                    Err(error) => failures.push(error),
                }
            }
        }
        docs.extend(kept.into_iter().filter(|doc| doc.dirty));
        self.search.docs = docs;
        self.search.source_filter = None;
        self.search.mark_data_changed();
        if !failures.is_empty() {
            self.status = Some(Err(format!(
                "some vaults could not be read: {}",
                failures.join("; ")
            )));
        }
    }

    /// Moves search doc `doc` into the vault pane (and the pane's
    /// previous vault into the doc list), preserving unsaved edits
    /// and stamps on both sides — no disk round-trip.
    pub(crate) fn adopt_search_doc(
        &mut self,
        doc: usize,
        open_tab: usize,
        selected: Option<(GridId, usize)>,
    ) {
        if doc >= self.search.docs.len() {
            return;
        }
        let adopted = self.search.docs.remove(doc);
        let absorbed_index = self.right.take().map(|pane| {
            self.search.docs.push(SearchDoc {
                path: pane.path,
                vault: pane.vault,
                dirty: pane.dirty,
                disk_stamp: pane.disk_stamp,
            });
            self.search.docs.len() - 1
        });
        self.right = Some(VaultPane {
            path: adopted.path,
            vault: adopted.vault,
            dirty: adopted.dirty,
            selected,
            disk_stamp: adopted.disk_stamp,
            open_tab,
        });
        for conflict in &mut self.conflicts {
            *conflict = match *conflict {
                DocId::Character => DocId::Character,
                DocId::Stash(slot) => DocId::Stash(slot),
                DocId::Vault => absorbed_index.map_or(DocId::Vault, DocId::SearchVault),
                DocId::SearchVault(index) => match index.cmp(&doc) {
                    std::cmp::Ordering::Equal => DocId::Vault,
                    std::cmp::Ordering::Greater => DocId::SearchVault(index - 1),
                    std::cmp::Ordering::Less => DocId::SearchVault(index),
                },
            };
        }
        self.search.mark_data_changed();
    }

    /// Double-click on a row: show the item at home in the vault
    /// pane, adopting its vault there first when needed.
    pub(crate) fn jump_to_search_row(&mut self, grid: GridId, index: usize) {
        match grid {
            GridId::VaultTab(tab) => {
                if let Some(pane) = self.right.as_mut() {
                    pane.open_tab = tab;
                    pane.selected = Some((GridId::VaultTab(tab), index));
                }
            }
            GridId::SearchDoc { doc, tab } => {
                self.adopt_search_doc(doc, tab, Some((GridId::VaultTab(tab), index)));
            }
            GridId::Sack(_)
            | GridId::Equipment(_)
            | GridId::Bank
            | GridId::Shared
            | GridId::Relic => {}
        }
        self.view = MainView::Panes;
    }

    fn rebuild_search_rows(&mut self) {
        self.search.stale = false;
        self.search.total = 0;
        let GameStatus::Loaded(db) = &self.game else {
            self.search.rows.clear();
            return;
        };
        if self.search.vocab_stale {
            let vaults = self
                .right
                .iter()
                .map(|pane| &pane.vault)
                .chain(self.search.docs.iter().map(|doc| &doc.vault));
            let (stats, affixes) = collect_vocab(db, vaults);
            self.search.vocab_stats = stats;
            self.search.vocab_affixes = affixes;
            self.search.vocab_stale = false;
        }
        let filters = draft_filters(&self.search.draft);
        let wanted = self.search.source_filter;
        let mut total = 0;
        let mut rows = Vec::new();
        if let Some(pane) = &self.right
            && wanted.is_none_or(|source| source == RowSource::Open)
        {
            collect_rows(
                db,
                &filters,
                &pane.vault,
                RowSource::Open,
                &vault_label(&pane.path),
                &mut total,
                &mut rows,
            );
        }
        for (index, doc) in self.search.docs.iter().enumerate() {
            if wanted.is_none_or(|source| source == RowSource::Doc(index)) {
                collect_rows(
                    db,
                    &filters,
                    &doc.vault,
                    RowSource::Doc(index),
                    &vault_label(&doc.path),
                    &mut total,
                    &mut rows,
                );
            }
        }
        sort_rows(&mut rows, self.search.sort);
        self.search.rows = rows;
        self.search.total = total;
    }

    /// The whole search surface: header, filter bar, results table.
    pub(crate) fn show_search_ui(&mut self, ui: &mut egui::Ui) -> SearchFrame {
        let mut frame = SearchFrame::default();
        if self.search.stale {
            self.rebuild_search_rows();
        }
        ui.horizontal(|ui| {
            if ui
                .button("← Back")
                .on_hover_text("Return to the panes (Esc)")
                .clicked()
            {
                frame.leave = true;
            }
            if ui
                .button("Rescan")
                .on_hover_text("Re-list the vaults folder for new or removed files")
                .clicked()
            {
                self.rescan_search_docs();
            }
            ui.heading("Search all vaults");
            ui.label(format!(
                "{} of {} items",
                self.search.rows.len(),
                self.search.total
            ));
            if self.any_dirty() {
                ui.weak("Saving…");
            }
        });
        if ui.ctx().input(|input| input.key_pressed(egui::Key::Escape))
            && ui.ctx().memory(|memory| memory.focused().is_none())
        {
            frame.leave = true;
        }
        let draft_before = self.search.draft.clone();
        let source_before = self.search.source_filter;
        self.show_filter_bar(ui);
        if self.search.draft != draft_before || self.search.source_filter != source_before {
            self.search.stale = true;
        }
        if self.search.stale {
            self.rebuild_search_rows();
        }
        ui.separator();
        self.show_search_table(ui, &mut frame);
        frame
    }

    fn show_filter_bar(&mut self, ui: &mut egui::Ui) {
        filter_text_row(ui, &mut self.search.draft);
        self.show_criteria_rows(ui);
        self.show_filter_choice_row(ui);
    }

    /// The dynamic criteria list: one row per stat/affix conjunct,
    /// each with a scope, an autocompleting text, and a min–max
    /// value window; rows are added and removed freely.
    fn show_criteria_rows(&mut self, ui: &mut egui::Ui) {
        let SearchState {
            draft,
            vocab_stats,
            vocab_affixes,
            ..
        } = &mut self.search;
        if draft.criteria.is_empty() {
            draft.criteria.push(CriterionDraft::default());
        }
        let mut remove = None;
        for (index, criterion) in draft.criteria.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(("criterion-scope", index))
                    .selected_text(criterion.scope.label())
                    .width(96.0)
                    .show_ui(ui, |ui| {
                        for scope in CriterionScope::ALL {
                            ui.selectable_value(&mut criterion.scope, scope, scope.label());
                        }
                    });
                let vocab: &[String] = match criterion.scope {
                    CriterionScope::AnyStat | CriterionScope::AffixStat => vocab_stats,
                    CriterionScope::AffixName => vocab_affixes,
                };
                suggesting_field(
                    ui,
                    ("criterion-text", index),
                    &mut criterion.text,
                    "type or pick…",
                    260.0,
                    vocab,
                );
                match criterion.scope {
                    CriterionScope::AffixName => {}
                    CriterionScope::AnyStat | CriterionScope::AffixStat => {
                        filter_field(ui, &mut criterion.min, "min", 44.0);
                        ui.label("–");
                        filter_field(ui, &mut criterion.max, "max", 44.0);
                    }
                }
                if ui
                    .button("✕")
                    .on_hover_text("Remove this criterion")
                    .clicked()
                {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            draft.criteria.remove(index);
        }
        if ui.button("＋ Add stat / affix criterion").clicked() {
            draft.criteria.push(CriterionDraft::default());
        }
    }

    fn show_filter_choice_row(&mut self, ui: &mut egui::Ui) {
        let draft = &mut self.search.draft;
        ui.horizontal_wrapped(|ui| {
            ui.label("Wearable at ≤");
            ui.label("Lv");
            filter_field(ui, &mut draft.req_level, "–", 36.0);
            ui.label("Str");
            filter_field(ui, &mut draft.req_strength, "–", 44.0);
            ui.label("Dex");
            filter_field(ui, &mut draft.req_dexterity, "–", 44.0);
            ui.label("Int");
            filter_field(ui, &mut draft.req_intelligence, "–", 44.0);
            egui::ComboBox::from_id_salt("search-rarity")
                .selected_text(draft.style.map_or("Any rarity", ItemStyle::label))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.style, None, "Any rarity");
                    for item_style in RARITY_CHOICES {
                        ui.selectable_value(&mut draft.style, Some(item_style), item_style.label());
                    }
                });
            egui::ComboBox::from_id_salt("search-category")
                .selected_text(draft.category.map_or("Any type", ItemCategory::label))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.category, None, "Any type");
                    for category in ItemCategory::ALL {
                        ui.selectable_value(&mut draft.category, Some(category), category.label());
                    }
                });
            egui::ComboBox::from_id_salt("search-socketed")
                .selected_text(match draft.socketed {
                    None => "Socketed or not",
                    Some(true) => "With relic/charm",
                    Some(false) => "Without relic/charm",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.socketed, None, "Socketed or not");
                    ui.selectable_value(&mut draft.socketed, Some(true), "With relic/charm");
                    ui.selectable_value(&mut draft.socketed, Some(false), "Without relic/charm");
                });
            egui::ComboBox::from_id_salt("search-origin")
                .selected_text(match draft.origin {
                    OriginDraft::Any => "Any origin",
                    OriginDraft::BaseGame => "Base game",
                    OriginDraft::Expansion(expansion) => expansion.label(),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.origin, OriginDraft::Any, "Any origin");
                    ui.selectable_value(&mut draft.origin, OriginDraft::BaseGame, "Base game");
                    for expansion in Expansion::ALL {
                        ui.selectable_value(
                            &mut draft.origin,
                            OriginDraft::Expansion(expansion),
                            expansion.label(),
                        );
                    }
                });
            let source_label = |source: Option<RowSource>| -> String {
                match source {
                    None => "All vaults".to_string(),
                    Some(RowSource::Open) => self.right.as_ref().map_or_else(
                        || "(open vault)".to_string(),
                        |pane| vault_label(&pane.path),
                    ),
                    Some(RowSource::Doc(index)) => self
                        .search
                        .docs
                        .get(index)
                        .map_or_else(|| "(gone)".to_string(), |doc| vault_label(&doc.path)),
                }
            };
            let mut source = self.search.source_filter;
            egui::ComboBox::from_id_salt("search-source")
                .selected_text(source_label(source))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut source, None, "All vaults");
                    if self.right.is_some() {
                        ui.selectable_value(
                            &mut source,
                            Some(RowSource::Open),
                            source_label(Some(RowSource::Open)),
                        );
                    }
                    for index in 0..self.search.docs.len() {
                        ui.selectable_value(
                            &mut source,
                            Some(RowSource::Doc(index)),
                            source_label(Some(RowSource::Doc(index))),
                        );
                    }
                });
            self.search.source_filter = source;
        });
    }

    fn show_search_table(&mut self, ui: &mut egui::Ui, frame: &mut SearchFrame) {
        let App {
            search,
            caches,
            game,
            ..
        } = self;
        let db = match game {
            GameStatus::Loaded(data) => Some(&*data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        if db.is_none() {
            ui.weak("Search needs imported game data — use 'Import game data…' first.");
            return;
        }
        let mut sort = search.sort;
        let sort_button = |ui: &mut egui::Ui, label: &str, key: SortKey, sort: &mut SortSpec| {
            let arrow = if sort.key == key {
                if sort.ascending { " ▲" } else { " ▼" }
            } else {
                ""
            };
            if ui
                .add(egui::Button::new(format!("{label}{arrow}")).frame(false))
                .clicked()
            {
                *sort = SortSpec {
                    key,
                    ascending: if sort.key == key {
                        !sort.ascending
                    } else {
                        true
                    },
                };
            }
        };
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .column(Column::exact(ICON_SIZE + 8.0))
            .column(Column::initial(230.0).at_least(140.0).clip(true))
            .column(Column::initial(90.0).clip(true))
            .column(Column::exact(48.0))
            .column(Column::initial(150.0).clip(true))
            .column(Column::remainder())
            .header(22.0, |mut header| {
                header.col(|_| {});
                header.col(|ui| sort_button(ui, "Name", SortKey::Name, &mut sort));
                header.col(|ui| sort_button(ui, "Rarity", SortKey::Rarity, &mut sort));
                header.col(|ui| sort_button(ui, "Lv", SortKey::ReqLevel, &mut sort));
                header.col(|ui| sort_button(ui, "Vault", SortKey::Location, &mut sort));
                header.col(|ui| {
                    ui.strong("Stats");
                });
            })
            .body(|body| {
                let SearchState { rows, selected, .. } = &mut *search;
                let heights: Vec<f32> = rows.iter().map(|row| row.height).collect();
                body.heterogeneous_rows(heights.into_iter(), |mut table_row| {
                    let Some(row) = rows.get(table_row.index()) else {
                        return;
                    };
                    show_row(&mut table_row, row, selected, caches, db, frame);
                });
            });
        if sort != search.sort {
            search.sort = sort;
            sort_rows(&mut search.rows, sort);
        }
    }
}

/// One table row: icon, colored name, rarity, level requirement,
/// vault, and the full colored stat lines — plus the shared item
/// gestures (select / duplicate / extract / send / copy / jump).
fn show_row(
    table_row: &mut egui_extras::TableRow<'_, '_>,
    row: &SearchRow,
    selected: &mut Option<(GridId, usize)>,
    caches: &mut crate::Caches,
    db: Option<&GameCache>,
    frame: &mut SearchFrame,
) {
    table_row.set_selected(*selected == Some((row.grid, row.index)));
    let (_, icon_response) = table_row.col(|ui| {
        if let Some(texture) = caches.icon(ui.ctx(), db, &row.item) {
            ui.image((texture.id(), egui::vec2(ICON_SIZE, ICON_SIZE)));
        } else {
            ui.label(row.name.chars().next().unwrap_or('?').to_string());
        }
    });
    // The game-style tooltip only on the icon — on the whole row it
    // would shadow the stats column at every pointer move.
    icon_response.on_hover_ui(|ui| item_tooltip(ui, &row.item, db, caches));
    table_row.col(|ui| {
        ui.label(
            egui::RichText::new(&row.name)
                .color(game_color(style::style_color(row.item_style)))
                .size(13.0),
        );
    });
    table_row.col(|ui| {
        ui.label(
            egui::RichText::new(row.item_style.label())
                .color(egui::Color32::from_gray(150))
                .size(12.0),
        );
    });
    table_row.col(|ui| {
        if let Some(level) = row.req_level {
            ui.label(level.to_string());
        }
    });
    table_row.col(|ui| {
        ui.label(egui::RichText::new(&row.location).size(12.0));
    });
    table_row.col(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        for (block_index, block) in row.details.blocks.iter().enumerate() {
            if block_index > 0 {
                ui.add_space(BLOCK_GAP);
            }
            for line in block {
                if line.text.trim().is_empty() {
                    ui.add_space(LINE_HEIGHT / 2.0);
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
    let response = table_row.response();
    let address = (row.grid, row.index);
    if response.double_clicked() {
        frame.jump = Some(address);
    } else if response.clicked() {
        let modifiers = response.ctx.input(|input| input.modifiers);
        if modifiers.shift {
            frame.duplicate = Some(address);
        } else if modifiers.alt {
            frame.extract = Some(address);
        } else {
            *selected = Some(address);
        }
    }
    if response.secondary_clicked() {
        if response.ctx.input(|input| input.modifiers.shift) {
            frame.copy_across = Some(address);
        } else {
            frame.quick_move = Some(address);
        }
    }
}

/// Every style, offered in the rarity dropdown.
const RARITY_CHOICES: [ItemStyle; 13] = [
    ItemStyle::Broken,
    ItemStyle::Mundane,
    ItemStyle::Common,
    ItemStyle::Rare,
    ItemStyle::Epic,
    ItemStyle::Legendary,
    ItemStyle::Quest,
    ItemStyle::Relic,
    ItemStyle::Potion,
    ItemStyle::Scroll,
    ItemStyle::Parchment,
    ItemStyle::Formulae,
    ItemStyle::Artifact,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_draft_builds_no_filters() {
        assert!(draft_filters(&FilterDraft::default()).is_empty());
    }

    fn criterion(scope: CriterionScope, text: &str, min: &str, max: &str) -> CriterionDraft {
        CriterionDraft {
            scope,
            text: text.into(),
            min: min.into(),
            max: max.into(),
        }
    }

    #[test]
    fn text_fields_trim_and_numbers_parse() {
        let draft = FilterDraft {
            name: "  hecate  ".into(),
            criteria: vec![criterion(CriterionScope::AffixStat, "fire", " 12.5 ", "")],
            req_level: "20".into(),
            req_strength: "not a number".into(),
            ..FilterDraft::default()
        };
        let filters = draft_filters(&draft);
        assert!(filters.contains(&Filter::NameContains("hecate".into())));
        assert!(filters.contains(&Filter::AffixStat {
            text: "fire".into(),
            bounds: ValueBounds {
                min: Some(12.5),
                max: None,
            },
        }));
        assert!(filters.contains(&Filter::RequirementAtMost(Requirement::Level, 20)));
        assert!(
            !filters.iter().any(|filter| matches!(
                filter,
                Filter::RequirementAtMost(Requirement::Strength, _)
            ))
        );
    }

    #[test]
    fn each_criterion_row_becomes_its_own_conjunct() {
        let draft = FilterDraft {
            criteria: vec![
                criterion(CriterionScope::AnyStat, "pierce resistance", "20", ""),
                criterion(CriterionScope::AnyStat, "poison resistance", "10", "40"),
                criterion(CriterionScope::AnyStat, "burn damage", "", ""),
                criterion(CriterionScope::AffixName, "of Thorns", "", ""),
                criterion(CriterionScope::AnyStat, "", "", ""),
            ],
            ..FilterDraft::default()
        };
        assert_eq!(
            draft_filters(&draft),
            vec![
                Filter::StatContains {
                    text: "pierce resistance".into(),
                    bounds: ValueBounds {
                        min: Some(20.0),
                        max: None,
                    },
                },
                Filter::StatContains {
                    text: "poison resistance".into(),
                    bounds: ValueBounds {
                        min: Some(10.0),
                        max: Some(40.0),
                    },
                },
                Filter::StatContains {
                    text: "burn damage".into(),
                    bounds: ValueBounds::default(),
                },
                Filter::HasAffix("of Thorns".into()),
            ]
        );
    }

    #[test]
    fn affix_name_rows_ignore_the_value_window() {
        let draft = FilterDraft {
            criteria: vec![criterion(CriterionScope::AffixName, "", "5", "10")],
            ..FilterDraft::default()
        };
        assert!(draft_filters(&draft).is_empty());
    }

    #[test]
    fn bare_minimum_values_still_filter() {
        let draft = FilterDraft {
            criteria: vec![criterion(CriterionScope::AnyStat, "", "100", "")],
            ..FilterDraft::default()
        };
        assert_eq!(
            draft_filters(&draft),
            vec![Filter::StatContains {
                text: String::new(),
                bounds: ValueBounds {
                    min: Some(100.0),
                    max: None,
                },
            }]
        );
    }

    #[test]
    fn set_checkbox_means_any_set_unless_text_given() {
        let checkbox_only = FilterDraft {
            set_only: true,
            ..FilterDraft::default()
        };
        assert_eq!(
            draft_filters(&checkbox_only),
            vec![Filter::InSet(String::new())]
        );
        let with_text = FilterDraft {
            set_only: true,
            set: "olympus".into(),
            ..FilterDraft::default()
        };
        assert_eq!(
            draft_filters(&with_text),
            vec![Filter::InSet("olympus".into())]
        );
    }

    #[test]
    fn rows_sort_by_key_with_name_tiebreak() {
        let row = |name: &str, rarity: u8, level: Option<i32>| SearchRow {
            grid: GridId::VaultTab(0),
            index: 0,
            item: sample_item(),
            name: name.to_string(),
            name_sort: name.to_lowercase(),
            item_style: ItemStyle::Common,
            rarity_rank: rarity,
            req_level: level,
            location: String::new(),
            details: stats::ItemDetails {
                quality: None,
                style_word: None,
                blocks: Vec::new(),
            },
            height: 40.0,
        };
        let mut rows = vec![
            row("Bravo", 2, Some(30)),
            row("Alpha", 5, None),
            row("Charlie", 2, Some(10)),
        ];
        sort_rows(
            &mut rows,
            SortSpec {
                key: SortKey::Rarity,
                ascending: true,
            },
        );
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["Bravo", "Charlie", "Alpha"]);
        sort_rows(
            &mut rows,
            SortSpec {
                key: SortKey::ReqLevel,
                ascending: false,
            },
        );
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["Bravo", "Charlie", "Alpha"]);
    }

    fn sample_item() -> Item {
        let json =
            r#"{"sacks":[{"items":[{"baseName":"records\\a.dbr","stackSize":1,"seed":1}]}]}"#;
        Vault::from_json(json).unwrap().sacks[0].items[0]
            .item
            .clone()
    }
}
