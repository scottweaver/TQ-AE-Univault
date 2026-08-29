//! The store search view: everything in the unified store as one
//! filtered, sorted table (icon | name | rarity | req | type |
//! stats), shown in the store pane's place so the game-file pane
//! stays live beside it. Rows carry the item's [`ItemAddr`], so the
//! standard gestures act on them — right-click sends to the active
//! left tab, Shift+Click duplicates, Alt+Click extracts a socketed
//! piece, double-click jumps to the item's type in the store pane.

use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::BTreeSet;
use univault_core::cache::GameCache;
use univault_core::chr::Item;

use univault_core::query::{self, Expansion, Filter, ItemCategory, ValueBounds};
use univault_core::stats::{self, Requirement};
use univault_core::store::VaultStore;
use univault_core::style::{self, ItemStyle};

use crate::theme;
use crate::{App, ItemAddr, MainView, StorePane, game_color, item_tooltip};

/// The search view's whole state; lives on [`App`] so filters and
/// sort survive a switch back to the store pane.
#[derive(Default)]
pub(crate) struct SearchState {
    pub(crate) stale: bool,
    /// The suggestion vocabularies lag behind `stale`: they rebuild
    /// only when item data changed, never on a filter keystroke.
    vocab_stale: bool,
    vocab_stats: Vec<String>,
    vocab_affixes: Vec<String>,
    draft: FilterDraft,
    sort: SortSpec,
    rows: Vec<SearchRow>,
    total: usize,
    selected: Option<ItemAddr>,
}

impl SearchState {
    /// Item data changed (edit, reload, import): rows and suggestion
    /// vocabularies both need a rebuild.
    pub(crate) fn mark_data_changed(&mut self) {
        self.stale = true;
        self.vocab_stale = true;
    }
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

impl FilterDraft {
    /// Nothing is being filtered — inert criterion rows (the bar
    /// always shows at least one empty row) don't count.
    fn is_clear(&self) -> bool {
        let texts = [
            &self.name,
            &self.req_level,
            &self.req_strength,
            &self.req_dexterity,
            &self.req_intelligence,
            &self.set,
        ];
        texts.iter().all(|text| text.trim().is_empty())
            && self.criteria.iter().all(CriterionDraft::is_empty)
            && !self.set_only
            && self.style.is_none()
            && self.category.is_none()
            && self.socketed.is_none()
            && self.origin == OriginDraft::Any
    }
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

impl CriterionDraft {
    fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.min.trim().is_empty() && self.max.trim().is_empty()
    }
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
    Bucket,
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
    addr: ItemAddr,
    item: Item,
    name: String,
    name_sort: String,
    item_style: ItemStyle,
    rarity_rank: u8,
    req_level: Option<i32>,
    /// The computed type bucket the item files under — its address in
    /// the store pane, and the table's location column.
    bucket: String,
    details: stats::ItemDetails,
    height: f32,
}

/// Row gestures reported back to the main loop, which routes them
/// through the same handlers the panes use. `rescan` defers the
/// folder re-list to the main loop, which owns the pane state the
/// rescan reconciles against.
#[derive(Default)]
pub(crate) struct SearchFrame {
    pub(crate) duplicate: Option<ItemAddr>,
    pub(crate) quick_move: Option<ItemAddr>,
    pub(crate) copy_across: Option<ItemAddr>,
    pub(crate) extract: Option<ItemAddr>,
    pub(crate) jump: Option<ItemAddr>,
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

/// The suggestion vocabularies over the store: distinct stat-line
/// templates (numbers replaced by `#`) and affix names.
fn collect_vocab(db: &GameCache, store: &VaultStore) -> (Vec<String>, Vec<String>) {
    let mut stats = BTreeSet::new();
    let mut affixes = BTreeSet::new();
    for entry in store.entries() {
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
    (stats.into_iter().collect(), affixes.into_iter().collect())
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
            SortKey::Bucket => a.bucket.cmp(&b.bucket).then(by_name),
        };
        if sort.ascending {
            ordering
        } else {
            ordering.reverse()
        }
    });
}

impl App {
    /// Switches to the search view.
    pub(crate) fn enter_search(&mut self) {
        self.view = MainView::Search;
        self.search.mark_data_changed();
    }
}

/// Rebuilds the filtered row set (and, when item data changed, the
/// suggestion vocabularies) from the store.
fn rebuild_rows(search: &mut SearchState, db: Option<&GameCache>, pane: Option<&StorePane>) {
    search.stale = false;
    search.total = 0;
    let (Some(db), Some(pane)) = (db, pane) else {
        search.rows.clear();
        return;
    };
    if search.vocab_stale {
        let (stats, affixes) = collect_vocab(db, &pane.store);
        search.vocab_stats = stats;
        search.vocab_affixes = affixes;
        search.vocab_stale = false;
    }
    let filters = draft_filters(&search.draft);
    let mut rows = Vec::new();
    for entry in pane.store.entries() {
        let item = &entry.item;
        search.total += 1;
        if !query::matches(db, item, &filters) {
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
            addr: ItemAddr::Stored(entry.id()),
            item: item.clone(),
            name_sort: name.to_lowercase(),
            name,
            rarity_rank: style_rank(item_style),
            item_style,
            req_level,
            bucket: univault_core::store::bucket_of(Some(db), item)
                .label()
                .to_string(),
            height: row_height(&details),
            details,
        });
    }
    sort_rows(&mut rows, search.sort);
    search.rows = rows;
}

/// The search surface in the vault pane's place: action row, filter
/// bar, results table. `dirty` mirrors the autosave indicator; the
/// caller routes the returned frame's gestures and requests.
pub(crate) fn show_search_pane(
    ui: &mut egui::Ui,
    search: &mut SearchState,
    pane: Option<&StorePane>,
    db: Option<&GameCache>,
    caches: &mut crate::Caches,
    dirty: bool,
) -> SearchFrame {
    let mut frame = SearchFrame::default();
    if search.stale {
        rebuild_rows(search, db, pane);
    }
    let pane_chrome = caches.chrome(ui.ctx(), db);
    let chrome_ref = pane_chrome.as_ref();
    ui.horizontal_wrapped(|ui| {
        if crate::plate_button(ui, chrome_ref, true, "← Store")
            .on_hover_text("Show the store again (Esc)")
            .clicked()
        {
            frame.leave = true;
        }
        let filtering = !search.draft.is_clear();
        if crate::plate_button(ui, chrome_ref, filtering, "Clear all")
            .on_hover_text("Reset every filter — show everything")
            .clicked()
        {
            search.draft = FilterDraft::default();
            search.stale = true;
        }
        ui.label(format!("{} of {} items", search.rows.len(), search.total));
        if dirty {
            ui.weak("Saving…");
        }
    });
    if ui.ctx().input(|input| input.key_pressed(egui::Key::Escape))
        && ui.ctx().memory(|memory| memory.focused().is_none())
    {
        frame.leave = true;
    }
    let draft_before = search.draft.clone();
    show_filter_bar(ui, search);
    if search.draft != draft_before {
        search.stale = true;
    }
    if search.stale {
        rebuild_rows(search, db, pane);
    }
    ui.separator();
    show_table(ui, search, caches, db, &mut frame);
    frame
}

fn show_filter_bar(ui: &mut egui::Ui, search: &mut SearchState) {
    filter_text_row(ui, &mut search.draft);
    show_criteria_rows(ui, search);
    show_filter_choice_row(ui, search);
}

/// The dynamic criteria list: one row per stat/affix conjunct,
/// each with a scope, an autocompleting text, and a min–max
/// value window; rows are added and removed freely.
fn show_criteria_rows(ui: &mut egui::Ui, search: &mut SearchState) {
    let SearchState {
        draft,
        vocab_stats,
        vocab_affixes,
        ..
    } = search;
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
                200.0,
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

fn show_filter_choice_row(ui: &mut egui::Ui, search: &mut SearchState) {
    let SearchState { draft, .. } = search;
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
    });
    ui.horizontal_wrapped(|ui| {
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
    });
    ui.horizontal_wrapped(|ui| {
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
    });
}

fn show_table(
    ui: &mut egui::Ui,
    search: &mut SearchState,
    caches: &mut crate::Caches,
    db: Option<&GameCache>,
    frame: &mut SearchFrame,
) {
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
        .column(Column::initial(170.0).at_least(120.0).clip(true))
        .column(Column::initial(70.0).clip(true))
        .column(Column::exact(40.0))
        .column(Column::initial(110.0).clip(true))
        .column(Column::remainder().clip(true))
        .header(22.0, |mut header| {
            header.col(|_| {});
            header.col(|ui| sort_button(ui, "Name", SortKey::Name, &mut sort));
            header.col(|ui| sort_button(ui, "Rarity", SortKey::Rarity, &mut sort));
            header.col(|ui| sort_button(ui, "Lv", SortKey::ReqLevel, &mut sort));
            header.col(|ui| sort_button(ui, "Type", SortKey::Bucket, &mut sort));
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

/// One table row: icon, colored name, rarity, level requirement,
/// vault, and the full colored stat lines — plus the shared item
/// gestures (select / duplicate / extract / send / copy / jump).
fn show_row(
    table_row: &mut egui_extras::TableRow<'_, '_>,
    row: &SearchRow,
    selected: &mut Option<ItemAddr>,
    caches: &mut crate::Caches,
    db: Option<&GameCache>,
    frame: &mut SearchFrame,
) {
    table_row.set_selected(*selected == Some(row.addr));
    let (_, icon_response) = table_row.col(|ui| {
        if let Some(texture) = caches.icon(ui.ctx(), db, &row.item) {
            ui.image((texture.id(), egui::vec2(ICON_SIZE, ICON_SIZE)));
        } else {
            ui.label(row.name.chars().next().unwrap_or('?').to_string());
        }
    });
    // The game-style tooltip only on the icon — on the whole row it
    // would shadow the stats column at every pointer move. Must be
    // `for_enabled`: `for_widget` carries no open condition in egui
    // 0.36 and would render every row's tooltip unconditionally.
    egui::Tooltip::for_enabled(&icon_response)
        .at_pointer()
        .show(|ui| item_tooltip(ui, &row.item, db, caches));
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
                .color(theme::TEXT_WEAK)
                .size(12.0),
        );
    });
    table_row.col(|ui| {
        if let Some(level) = row.req_level {
            ui.label(level.to_string());
        }
    });
    table_row.col(|ui| {
        ui.label(egui::RichText::new(&row.bucket).size(12.0));
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
    let address = row.addr;
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
    fn clear_detection_ignores_inert_criterion_rows() {
        assert!(FilterDraft::default().is_clear());
        let with_empty_rows = FilterDraft {
            criteria: vec![
                criterion(CriterionScope::AffixStat, "", "", ""),
                criterion(CriterionScope::AffixName, "  ", "", ""),
            ],
            ..FilterDraft::default()
        };
        assert!(with_empty_rows.is_clear());
        let filtering = FilterDraft {
            criteria: vec![criterion(CriterionScope::AnyStat, "", "5", "")],
            ..FilterDraft::default()
        };
        assert!(!filtering.is_clear());
        let set_only = FilterDraft {
            set_only: true,
            ..FilterDraft::default()
        };
        assert!(!set_only.is_clear());
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
            addr: ItemAddr::Stored(sample_id()),
            item: sample_item(),
            name: name.to_string(),
            name_sort: name.to_lowercase(),
            item_style: ItemStyle::Common,
            rarity_rank: rarity,
            req_level: level,
            bucket: String::new(),
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
        Item::bare(
            univault_core::chr::RecordId::parse("records\\a.dbr".to_string()).unwrap(),
            univault_core::chr::ItemSeed::new(1),
        )
    }

    fn sample_id() -> univault_core::store::StoredItemId {
        let mut store = VaultStore::new();
        store.add(sample_item())
    }
}
