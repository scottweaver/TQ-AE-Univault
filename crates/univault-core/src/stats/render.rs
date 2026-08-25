//! The import-time stat renderer: turns one database record into the
//! display lines, counts, and requirement data the tooltip needs —
//! `TQVaultAE`'s `ItemProvider.GetAttributesFromRecord` /
//! `ConvertOffenseAttributesToString` and their satellite renderers
//! (MIT), re-shaped to run once per record at import.
//!
//! Known deviations from the reference, chosen deliberately:
//! `attributeScalePercent` scales only its own record's lines (the
//! reference leaks the base item's scale into relic sections), and
//! it applies to every group regardless of sort position.

use serde::{Deserialize, Serialize};

use crate::arz::{DbRecord, DbValues, DbVariable};
use crate::chr::RecordId;
use crate::gamedata::GameData;
use crate::stats::dictionary::{
    self, AttributeData, EffectType, attribute_data, attribute_text_tag, unknown_attribute,
};
use crate::stats::format::{
    Arg, FormatSpec, leading_color, parse_format, strip_color_tags, wrap_words,
};

/// One rendered tooltip line: a palette color tag (the game's
/// `{^X}` letters) and plain text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatLine {
    pub color: char,
    pub text: String,
}

impl StatLine {
    fn new(color: char, text: impl Into<String>) -> Self {
        Self {
            color,
            text: text.into(),
        }
    }
}

/// Palette letters for the styles the engine colors lines with.
pub(crate) const COLOR_MUNDANE: char = 'W';
pub(crate) const COLOR_MAGICAL: char = 'B';
pub(crate) const COLOR_UNKNOWN: char = 'P';
pub(crate) const COLOR_RELIC: char = 'O';
pub(crate) const COLOR_RARE: char = 'G';
pub(crate) const COLOR_COMMON: char = 'Y';
pub(crate) const COLOR_BROKEN: char = 'D';

/// A requirement key — the four the game formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Requirement {
    Level,
    Strength,
    Dexterity,
    Intelligence,
}

impl Requirement {
    fn from_variable(name: &str) -> Option<Self> {
        let upper = name.to_uppercase();
        match upper.as_str() {
            "LEVELREQUIREMENT" => Some(Self::Level),
            "STRENGTHREQUIREMENT" => Some(Self::Strength),
            "DEXTERITYREQUIREMENT" => Some(Self::Dexterity),
            "INTELLIGENCEREQUIREMENT" => Some(Self::Intelligence),
            _ => None,
        }
    }

    fn from_equation_key(key: &str) -> Option<Self> {
        match key.to_uppercase().as_str() {
            "LEVEL" => Some(Self::Level),
            "STRENGTH" => Some(Self::Strength),
            "DEXTERITY" => Some(Self::Dexterity),
            "INTELLIGENCE" => Some(Self::Intelligence),
            _ => None,
        }
    }

    /// The localization tag naming this requirement.
    pub(crate) fn text_tag(self) -> &'static str {
        match self {
            Self::Level => "LevelRequirement",
            Self::Strength => "Strength",
            Self::Dexterity => "Dexterity",
            Self::Intelligence => "Intelligence",
        }
    }
}

/// Artifact details shown on a formula.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FormulaExtras {
    pub(crate) artifact_name: String,
    pub(crate) artifact_class: Option<String>,
    pub(crate) attr: Vec<StatLine>,
    pub(crate) requirements: Vec<(Requirement, i32)>,
}

/// Everything the tooltip needs about one record, rendered at import.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub(crate) struct StatBlock {
    /// Display lines per variable slot — index is the relic/charm
    /// shard count minus one; non-relic records have exactly one.
    pub(crate) attr: Vec<Vec<StatLine>>,
    /// This record's contribution to `totalAttCount`.
    pub(crate) attribute_count: i32,
    /// Explicit requirement variables on this record.
    pub(crate) requirements: Vec<(Requirement, i32)>,
    /// Requirement equations from the item-cost record (base gear
    /// only), evaluated at display time with `totalAttCount`.
    pub(crate) equations: Vec<(Requirement, String)>,
    pub(crate) item_level: i32,
    /// Translated `itemStyleTag` text: a name particle for gear, the
    /// flavor text for potions/relics/scrolls/parchment/quest items.
    pub(crate) style_text: Option<String>,
    /// Translated `itemQualityTag` name particle.
    pub(crate) quality_text: Option<String>,
    pub(crate) set_lines: Vec<StatLine>,
    pub(crate) artifact_class: Option<String>,
    pub(crate) formula: Option<FormulaExtras>,
}

/// How deep nested renders (granted skill → buff → pet) may go.
const MAX_DEPTH: u32 = 4;

/// How many shard slots a relic render covers at most.
const MAX_RELIC_SLOTS: i32 = 12;

pub(crate) struct Renderer<'a> {
    pub(crate) data: &'a GameData,
}

/// The role facts of the record whose lines are being rendered —
/// stands in for the reference's `itm.Is*`/`recordId == BaseItemId`
/// checks, which collapse to record-shape facts at import time.
#[derive(Clone, Copy)]
struct RecordRole<'a> {
    /// The gear record the render started from (`itm.baseItemInfo`).
    base: &'a DbRecord,
    /// Whether the record currently being rendered is `base` itself.
    is_base: bool,
    variable_number: usize,
}

impl Renderer<'_> {
    fn tag(&self, tag: &str) -> Option<&str> {
        self.data.tag_text(tag)
    }

    fn tag_spec(&self, tag: &str) -> Option<FormatSpec> {
        self.tag(tag).map(parse_format)
    }

    fn record(&self, id: &str) -> Option<DbRecord> {
        let with_extension = if id.to_uppercase().ends_with(".DBR") {
            id.to_string()
        } else {
            format!("{id}.dbr")
        };
        let record_id = RecordId::parse(with_extension)?;
        self.data.record(&record_id)?.ok()
    }

    /// Renders the full stat block for one item-classed record.
    pub(crate) fn stat_block(&self, record: &DbRecord) -> StatBlock {
        let slots = if is_relic_class(&record.record_type) {
            record
                .integer("completedRelicLevel")
                .unwrap_or(1)
                .clamp(1, MAX_RELIC_SLOTS)
        } else {
            1
        };
        let attr = (0..slots)
            .map(|slot| {
                let role = RecordRole {
                    base: record,
                    is_base: true,
                    variable_number: slot as usize,
                };
                let mut lines = Vec::new();
                self.render_record(record, role, &mut lines, 0);
                lines
            })
            .collect();
        let mut requirements = explicit_requirements(record);
        let formula = self.formula_extras(record);
        if let Some(extras) = &formula {
            // A formula also carries its artifact's requirements.
            for (key, value) in &extras.requirements {
                match requirements
                    .iter_mut()
                    .find(|(existing, _)| existing == key)
                {
                    Some((_, existing_value)) => *existing_value = (*existing_value).max(*value),
                    None => requirements.push((*key, *value)),
                }
            }
        }
        StatBlock {
            attr,
            attribute_count: attribute_count(record),
            requirements,
            equations: self.cost_equations(record),
            item_level: record.integer("itemLevel").unwrap_or(0),
            style_text: self.style_text(record),
            quality_text: self.translated(record, "itemQualityTag"),
            set_lines: self.set_lines(record),
            artifact_class: record
                .string("artifactClassification")
                .map(|class| self.artifact_class_label(class)),
            formula: self.formula_extras(record),
        }
    }

    fn translated(&self, record: &DbRecord, variable: &str) -> Option<String> {
        let tag = record.string(variable).filter(|tag| !tag.is_empty())?;
        let text = self.tag(tag)?;
        Some(strip_color_tags(text))
    }

    /// The style/flavor text source per record kind — `Info`'s
    /// `styleVar` dispatch: one-shots, quest items, relics/charms and
    /// quest equipment carry flavor in `itemText`; gear carries its
    /// style particle in `itemStyleTag`; artifacts and formulae have
    /// neither.
    fn style_text(&self, record: &DbRecord) -> Option<String> {
        let class = record.record_type.to_uppercase();
        if class.starts_with("ONESHOT")
            || class == "QUESTITEM"
            || class == "ITEMEQUIPMENT"
            || class == "ITEMRELIC"
            || class == "ITEMCHARM"
        {
            self.translated(record, "itemText")
        } else if class.starts_with("ITEMARTIFACT") {
            None
        } else {
            self.translated(record, "itemStyleTag")
        }
    }

    fn artifact_class_label(&self, classification: &str) -> String {
        let tag = match classification.to_uppercase().as_str() {
            "LESSER" => "xtagArtifactClass01",
            "GREATER" => "xtagArtifactClass02",
            "DIVINE" => "xtagArtifactClass03",
            _ => return "Unknown Artifact Class".to_string(),
        };
        self.tag(tag)
            .map_or_else(|| "Unknown Artifact Class".to_string(), strip_color_tags)
    }

    fn set_lines(&self, record: &DbRecord) -> Vec<StatLine> {
        let Some(set_id) = record.string("itemSetName").filter(|id| !id.is_empty()) else {
            return Vec::new();
        };
        let Some(set_record) = self.record(set_id) else {
            return Vec::new();
        };
        let Some(DbValues::Strings(members)) = set_record.variable("setMembers").map(|v| &v.values)
        else {
            return Vec::new();
        };
        let set_name = set_record
            .string("setName")
            .and_then(|tag| self.tag(tag))
            .map_or_else(|| "Unknown Set".to_string(), strip_color_tags);
        let mut lines = vec![StatLine::new(COLOR_RARE, set_name)];
        for member in members {
            let name = self
                .record(member)
                .and_then(|record| {
                    record
                        .string("description")
                        .or_else(|| record.string("itemNameTag"))
                        .and_then(|tag| self.tag(tag))
                        .map(strip_color_tags)
                })
                .unwrap_or_else(|| "?? Missing database info ??".to_string());
            lines.push(StatLine::new(COLOR_COMMON, format!("    {name}")));
        }
        lines
    }

    fn formula_extras(&self, record: &DbRecord) -> Option<FormulaExtras> {
        if !record
            .record_type
            .eq_ignore_ascii_case("ItemArtifactFormula")
        {
            return None;
        }
        let artifact_id = record.string("artifactName").filter(|id| !id.is_empty())?;
        let artifact = self.record(artifact_id)?;
        let artifact_name = artifact
            .string("description")
            .and_then(|tag| self.tag(tag))
            .map_or_else(|| "?Unknown Artifact Name?".to_string(), strip_color_tags);
        let artifact_class = artifact
            .string("artifactClassification")
            .map(|class| self.artifact_class_label(class));
        let role = RecordRole {
            base: &artifact,
            is_base: true,
            variable_number: 0,
        };
        let mut attr = Vec::new();
        self.render_record(&artifact, role, &mut attr, 0);
        Some(FormulaExtras {
            artifact_name,
            artifact_class,
            attr,
            requirements: explicit_requirements(&artifact),
        })
    }

    /// The requirement equations of the record's item-cost table —
    /// `GetDynamicRequirementsFromRecord`, minus the evaluation that
    /// needs the assembled item.
    fn cost_equations(&self, record: &DbRecord) -> Vec<(Requirement, String)> {
        if record.integer("itemLevel").is_none() && record.float("itemLevel").is_none() {
            return Vec::new();
        }
        let Some(prefix) = requirement_equation_prefix(&record.record_type) else {
            return Vec::new();
        };
        let cost_record = record
            .string("itemCostName")
            .filter(|id| !id.is_empty())
            .and_then(|id| self.record(id))
            .or_else(|| self.record("records/game/itemcost.dbr"));
        let Some(cost_record) = cost_record else {
            return Vec::new();
        };
        let mut equations = Vec::new();
        for variable in cost_record.variables() {
            if variable.name.len() < prefix.len()
                || !variable.name[..prefix.len()].eq_ignore_ascii_case(prefix)
            {
                continue;
            }
            let key = variable.name[prefix.len()..].replace("Equation", "");
            let Some(requirement) = Requirement::from_equation_key(&key) else {
                continue;
            };
            let Some(DbValues::Strings(values)) = Some(&variable.values) else {
                continue;
            };
            if let Some(expression) = values.first().filter(|expr| !expr.is_empty()) {
                equations.push((requirement, expression.clone()));
            }
        }
        equations
    }

    /// `GetAttributesFromRecord`: group, sort, and convert every
    /// displayable variable of one record into lines.
    fn render_record(
        &self,
        record: &DbRecord,
        role: RecordRole<'_>,
        results: &mut Vec<StatLine>,
        depth: u32,
    ) {
        struct Group<'r> {
            key: String,
            vars: Vec<&'r DbVariable>,
            spurious_global: bool,
            xor_global: bool,
        }
        if depth > MAX_DEPTH {
            return;
        }
        let mut groups: Vec<Group<'_>> = Vec::new();
        for variable in displayable_variables(record) {
            let data = attribute_data(&variable.name)
                .cloned()
                .unwrap_or_else(|| unknown_attribute(&variable.name));
            let key = effect_group(&data);
            match groups.iter_mut().find(|group| group.key == key) {
                Some(group) => group.vars.push(variable),
                None => groups.push(Group {
                    key,
                    vars: vec![variable],
                    spurious_global: false,
                    xor_global: false,
                }),
            }
        }

        let is_armor_or_shield =
            is_armor_class(&record.record_type) || is_shield_class(&record.record_type);
        let order_of = |vars: &[&DbVariable]| group_order(vars, is_armor_or_shield);
        let mut ordered: Vec<usize> = (0..groups.len()).collect();
        ordered.sort_by_key(|&index| (order_of(&groups[index].vars), index));
        let groups_sorted: Vec<usize> = ordered;

        // Global-chance groups apply to the group that follows them:
        // XOR there means "one of the following", and a global chance
        // with no global group after it is spurious.
        for position in 0..groups_sorted.len() {
            let index = groups_sorted[position];
            let is_global_chance = {
                let first = groups[index].vars[0];
                let data = attribute_data(&first.name)
                    .cloned()
                    .unwrap_or_else(|| unknown_attribute(&first.name));
                data.effect.eq_ignore_ascii_case("offensiveGlobalChance")
                    || data.effect.eq_ignore_ascii_case("retaliationGlobalChance")
            };
            if !is_global_chance {
                continue;
            }
            let next = groups_sorted.get(position + 1).copied();
            let (spurious, xor) = match next {
                Some(next_index) => {
                    let next_vars = &groups[next_index].vars;
                    if group_has_variable(next_vars, "Global") {
                        (false, group_has_variable(next_vars, "XOR"))
                    } else {
                        (true, false)
                    }
                }
                None => (true, false),
            };
            groups[index].spurious_global = spurious;
            groups[index].xor_global = xor;
        }

        for index in groups_sorted {
            let group = &groups[index];
            let mut vars = group.vars.clone();
            if group_effect_type(&vars) == EffectType::DamageQualifier {
                vars.sort_by_key(|variable| {
                    attribute_data(&variable.name).map_or(3_000_000, |data| {
                        ((1000 * (1 + data.effect_type.order())) + data.suborder) * 10
                    })
                });
            }
            self.convert_group(
                record,
                role,
                &vars,
                group.spurious_global,
                group.xor_global,
                results,
                depth,
            );
        }
    }

    /// `ConvertOffenseAttributesToString`: one sorted effect group to
    /// its line(s).
    #[allow(clippy::too_many_arguments)] // mirrors the reference signature
    fn convert_group(
        &self,
        record: &DbRecord,
        role: RecordRole<'_>,
        vars: &[&DbVariable],
        spurious_global: bool,
        xor_global: bool,
        results: &mut Vec<StatLine>,
        depth: u32,
    ) {
        let data = attribute_data(&vars[0].name)
            .cloned()
            .unwrap_or_else(|| unknown_attribute(&vars[0].name));

        let mut variable_number = if role.is_base {
            role.variable_number
        } else {
            0
        };
        if record.record_type.to_uppercase().starts_with("SKILL") {
            variable_number = self.triggered_skill_level(role.base, record, variable_number);
        }

        let scale = record
            .variable("attributeScalePercent")
            .map_or(1.0, |variable| 1.0 + float_at(variable, 0) / 100.0);

        let mut min_var = None;
        let mut max_var = None;
        let mut min_dur_var = None;
        let mut max_dur_var = None;
        let mut chance_var = None;
        let mut modifier_var = None;
        let mut duration_modifier_var = None;
        let mut modifier_chance_var = None;
        let mut damage_ratio_var: Option<(&DbVariable, AttributeData)> = None;
        for variable in vars {
            let attribute = attribute_data(&variable.name)
                .cloned()
                .unwrap_or_else(|| unknown_attribute(&variable.name));
            match attribute.variable.to_uppercase().as_str() {
                "MIN" | "DRAINMIN" => min_var = Some(*variable),
                "MAX" | "DRAINMAX" => max_var = Some(*variable),
                "DURATIONMIN" => min_dur_var = Some(*variable),
                "DURATIONMAX" => max_dur_var = Some(*variable),
                "CHANCE" => chance_var = Some(*variable),
                "MODIFIER" => modifier_var = Some(*variable),
                "MODIFIERCHANCE" => modifier_chance_var = Some(*variable),
                "DURATIONMODIFIER" => duration_modifier_var = Some(*variable),
                "DAMAGERATIO" => damage_ratio_var = Some((*variable, attribute)),
                _ => {}
            }
        }

        let is_global = group_has_variable(vars, "Global");
        let global_indent = "    ";

        // Label.
        let (mut label, label_color, label_leading_blank) = self.label_for(&data, record, role);

        // Amount.
        let duration_scales = min_dur_var.filter(|_| duration_reliant(&data.effect));
        let amount = {
            let values_differ = match (min_var, max_var) {
                (Some(min), Some(max)) => {
                    float_at_capped(min, variable_number) != float_at_capped(max, variable_number)
                }
                _ => false,
            };
            let value_of = |variable: &DbVariable| {
                let mut value = float_at_capped(variable, variable_number) * scale;
                if let Some(duration) = duration_scales {
                    value *= float_at(duration, duration.len().saturating_sub(1));
                }
                value
            };
            if values_differ {
                let (min, max) = (min_var.unwrap(), max_var.unwrap());
                let spec = if label.as_ref().is_some_and(FormatSpec::has_args) {
                    label.take().unwrap()
                } else {
                    self.tag_spec(range_format_tag(&data.effect))
                        .unwrap_or_else(|| parse_format("{%.0f0}..{%.0f1}"))
                };
                Some(spec.format(&[Arg::Number(value_of(min)), Arg::Number(value_of(max))]))
            } else {
                min_var.or(max_var).map(|variable| {
                    let spec = if label.as_ref().is_some_and(FormatSpec::has_args) {
                        label.take().unwrap()
                    } else {
                        self.tag_spec(single_format_tag(&data.effect))
                            .unwrap_or_else(|| parse_format("{%.0f0}"))
                    };
                    spec.format(&[Arg::Number(value_of(variable))])
                })
            }
        };

        // Duration.
        let duration = match (min_dur_var, max_dur_var) {
            (Some(min), Some(max))
                if float_at(min, min.len().saturating_sub(1))
                    != float_at(max, max.len().saturating_sub(1)) =>
            {
                self.tag_spec("DamageRangeFormatTime").map(|spec| {
                    spec.format(&[
                        Arg::Number(float_at_capped(min, variable_number)),
                        Arg::Number(float_at_capped(max, variable_number)),
                    ])
                })
            }
            (min, max) => min.or(max).and_then(|variable| {
                self.tag_spec("DamageSingleFormatTime").map(|spec| {
                    spec.format(&[Arg::Number(float_at_capped(variable, variable_number))])
                })
            }),
        };

        // Damage ratio.
        let damage_ratio = damage_ratio_var.map(|(variable, ratio_data)| {
            let full = &ratio_data.full_attribute;
            let middle = &full[9.min(full.len())..full.len().saturating_sub(11)];
            let tag = format!("Damage{middle}Ratio");
            let spec = self
                .tag_spec(&tag)
                .unwrap_or_else(|| parse_format(&format!("{{%.1f0}}% ?{full}?")));
            spec.format(&[Arg::Number(float_at_capped(variable, variable_number))])
        });

        // Chance.
        let chance = chance_var.and_then(|variable| {
            self.tag_spec("ChanceOfTag")
                .map(|spec| spec.format(&[Arg::Number(float_at_capped(variable, variable_number))]))
        });

        let mut amount_used = false;
        if amount.is_some() || duration.is_some() {
            amount_used = true;
            let label_text = label.as_ref().map(|spec| spec.format(&[]));
            let parts: Vec<&str> = [
                chance.as_deref(),
                amount.as_deref(),
                label_text.as_deref(),
                duration.as_deref(),
                damage_ratio.as_deref(),
            ]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect();
            let text = parts.join(" ");
            let color = if !is_global
                && (chance.is_none() || data.effect.eq_ignore_ascii_case("defensiveBlock"))
                && role.is_base
                && duration.is_none()
                && amount.is_some()
                && mundane_base_stat(record, &data.effect)
            {
                COLOR_MUNDANE
            } else {
                label_color.unwrap_or(COLOR_MAGICAL)
            };
            let was_first = results.is_empty();
            let indented = if is_global {
                format!("{global_indent}{text}")
            } else {
                text
            };
            push_line(results, StatLine::new(color, indented));
            if label_leading_blank && was_first {
                results.push(StatLine::new(COLOR_MUNDANE, ""));
            }
        }

        // Modifier line.
        let modifier = modifier_var.map(|variable| {
            let tag = attribute_text_tag(&data);
            let spec = self
                .tag_spec(&tag)
                .unwrap_or_else(|| parse_format(&format!("{{%.1f0}}% ?{}?", data.full_attribute)));
            spec.format(&[Arg::Number(
                float_at_capped(variable, variable_number) * scale,
            )])
        });
        let duration_modifier = duration_modifier_var.and_then(|variable| {
            self.tag_spec("ImprovedTimeFormat")
                .map(|spec| spec.format(&[Arg::Number(float_at_capped(variable, variable_number))]))
        });
        let modifier_chance = modifier_chance_var.and_then(|variable| {
            self.tag_spec("ChanceOfTag")
                .map(|spec| spec.format(&[Arg::Number(float_at_capped(variable, variable_number))]))
        });
        let modifier_used = modifier.is_some();
        if let Some(modifier_text) = &modifier {
            let parts: Vec<&str> = [
                modifier_chance.as_deref(),
                Some(modifier_text.as_str()),
                duration_modifier.as_deref(),
            ]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect();
            let mut text = parts.join(" ");
            if is_global {
                text = format!("{global_indent}{text}");
            }
            push_line(results, StatLine::new(COLOR_MAGICAL, text));
        }

        // Everything the amount/modifier lines did not consume.
        let mut qualifier_title_shown = false;
        for variable in vars {
            let attribute = attribute_data(&variable.name)
                .cloned()
                .unwrap_or_else(|| unknown_attribute(&variable.name));
            let sub = attribute.variable.to_uppercase();
            let consumed = (amount_used
                && matches!(sub.as_str(), "MIN" | "MAX" | "DRAINMIN" | "DRAINMAX"))
                || (amount_used
                    && duration.is_some()
                    && matches!(sub.as_str(), "DURATIONMIN" | "DURATIONMAX"))
                || (amount_used && chance.is_some() && sub == "CHANCE")
                || (modifier_used && sub == "MODIFIER")
                || (modifier_used && duration_modifier.is_some() && sub == "DURATIONMODIFIER")
                || (modifier_used && modifier_chance.is_some() && sub == "MODIFIERCHANCE")
                || (amount_used && damage_ratio.is_some() && sub == "DAMAGERATIO")
                || sub == "GLOBAL"
                || (sub == "XOR" && is_global);
            if consumed {
                continue;
            }
            self.convert_leftover(
                record,
                role,
                &data,
                &attribute,
                variable,
                variable_number,
                is_global,
                spurious_global,
                xor_global,
                &mut qualifier_title_shown,
                results,
                depth,
            );
        }
    }

    /// The label tag, its color override, and whether a blank line
    /// precedes it (`GetLabelAndColorFromTag`).
    fn label_for(
        &self,
        data: &AttributeData,
        record: &DbRecord,
        role: RecordRole<'_>,
    ) -> (Option<FormatSpec>, Option<char>, bool) {
        let mut tag = attribute_text_tag(data);
        let mut color = None;
        let mut leading_blank = false;
        if tag.is_empty() {
            return (
                Some(parse_format(&format!("?{}?", data.full_attribute))),
                Some(COLOR_UNKNOWN),
                false,
            );
        }
        if tag.eq_ignore_ascii_case("DefenseAbsorptionProtection") {
            if !is_armor_class(&record.record_type) || !role.is_base {
                tag = "DefenseAbsorptionProtectionBonus".to_string();
                color = Some(COLOR_MAGICAL);
            } else {
                color = Some(COLOR_MUNDANE);
                leading_blank = true;
            }
        }
        match self.tag(&tag) {
            Some(text) => {
                let text_color = leading_color(text);
                (
                    Some(parse_format(text)),
                    text_color.or(color),
                    leading_blank,
                )
            }
            None => (
                Some(parse_format(&format!("?{tag}?"))),
                Some(COLOR_UNKNOWN),
                false,
            ),
        }
    }

    /// The leftover-variable special cases from the tail of
    /// `ConvertOffenseAttributesToString`.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // mirrors one reference loop body
    fn convert_leftover(
        &self,
        record: &DbRecord,
        role: RecordRole<'_>,
        group_data: &AttributeData,
        attribute: &AttributeData,
        variable: &DbVariable,
        variable_number: usize,
        is_global: bool,
        spurious_global: bool,
        xor_global: bool,
        qualifier_title_shown: &mut bool,
        results: &mut Vec<StatLine>,
        depth: u32,
    ) {
        let full_upper = attribute.full_attribute.to_uppercase();
        let is_formula = record
            .record_type
            .eq_ignore_ascii_case("ItemArtifactFormula");
        let mut color = None;
        let mut line: Option<String> = None;

        if full_upper == "CHARACTERBASEATTACKSPEEDTAG" {
            if is_weapon_class(&record.record_type) && role.is_base {
                color = Some(COLOR_MUNDANE);
                line = first_string(variable)
                    .and_then(|tag| self.tag(tag))
                    .map(strip_color_tags);
            } else {
                line = Some(String::new());
            }
        } else if full_upper.ends_with("GLOBALCHANCE") {
            line = Some(if spurious_global {
                String::new()
            } else {
                let tag = if xor_global {
                    "GlobalPercentChanceOfOneTag"
                } else {
                    "GlobalPercentChanceOfAllTag"
                };
                color = Some(COLOR_MAGICAL);
                self.tag_spec(tag)
                    .unwrap_or_else(|| parse_format("{%.0f0}% ?Chance?"))
                    .format(&[Arg::Number(float_at_capped(variable, variable_number))])
            });
        } else if full_upper.starts_with("RACIALBONUS") {
            line = self.racial_bonus(
                record,
                attribute,
                variable,
                variable_number,
                results,
                &mut color,
            );
        } else if full_upper == "AUGMENTALLLEVEL" {
            color = Some(COLOR_MAGICAL);
            line = Some(
                self.tag_spec("ItemAllSkillIncrement")
                    .unwrap_or_else(|| parse_format("?+{%d0} to all skills?"))
                    .format(&[Arg::Number(float_at_capped(variable, variable_number))]),
            );
        } else if full_upper.starts_with("AUGMENTMASTERYLEVEL") {
            line = Some(self.augment_mastery(record, attribute, variable, &mut color));
        } else if full_upper.starts_with("AUGMENTSKILLLEVEL") {
            line = self.augment_skill(record, attribute, variable, &mut color);
        } else if is_formula && role.is_base && full_upper.starts_with("REAGENT") {
            line = self.reagent_name(variable, &mut color);
        } else if is_formula && role.is_base && full_upper == "ARTIFACTCREATIONCOST" {
            color = Some(COLOR_RARE);
            results.push(StatLine::new(COLOR_MUNDANE, ""));
            line = Some(
                self.tag_spec("xtagArtifactCost")
                    .unwrap_or_else(|| parse_format("Gold Cost: {%t0}"))
                    .format(&[Arg::Text(thousands(integer_at(variable, 0)))]),
            );
        } else if full_upper == "ITEMSKILLNAME" {
            line = self.granted_skill(record, variable, &mut color, results, depth);
        } else if full_upper == "PETBONUSNAME" {
            let header = self
                .tag("xtagPetBonusNameAllPets")
                .map_or_else(|| "?Bonus to All Pets:?".to_string(), strip_color_tags);
            color = Some(COLOR_RELIC);
            results.push(StatLine::new(COLOR_MUNDANE, ""));
            line = Some(header);
        } else if (role.is_base && full_upper == "ATTRIBUTESCALEPERCENT")
            || full_upper == "SKILLNAME"
        {
            line = Some(String::new());
        } else if attribute.effect_type == EffectType::SkillEffect {
            line = Some(self.skill_effect(
                group_data,
                attribute,
                variable,
                variable_number,
                &mut color,
            ));
        } else if full_upper.ends_with("DAMAGEQUALIFIER") {
            if !*qualifier_title_shown {
                let title = self
                    .tag("tagDamageAbsorptionTitle")
                    .map_or_else(|| "Protects Against :".to_string(), strip_color_tags);
                results.push(StatLine::new(COLOR_MUNDANE, title));
                *qualifier_title_shown = true;
            }
            let full = &attribute.full_attribute;
            let damage = first_upper(&full[..full.len().saturating_sub(15)]);
            let damage_type = self
                .tag(&format!("tagQualifyingDamage{damage}"))
                .map_or_else(|| damage.clone(), strip_color_tags);
            color = Some(COLOR_MUNDANE);
            line = Some(
                self.tag_spec("formatQualifyingDamage")
                    .unwrap_or_else(|| parse_format("{%s0}"))
                    .format(&[Arg::Text(damage_type)]),
            );
        }

        let line = line.unwrap_or_else(|| {
            self.raw_attribute(attribute, variable, variable_number, &mut color)
        });
        if !line.is_empty() {
            let mut text = line;
            if !attribute.variable.is_empty() {
                text = format!("{text} {}", attribute.variable);
                color = color.or(Some(COLOR_UNKNOWN));
            }
            if is_global || (is_formula && full_upper.starts_with("REAGENT")) {
                text = format!("    {text}");
            }
            for split in text.split('\n') {
                push_line(
                    results,
                    StatLine::new(color.unwrap_or(COLOR_UNKNOWN), split.to_string()),
                );
            }
        }

        if full_upper == "PETBONUSNAME"
            && let Some(bonus_record) = record
                .string("petBonusName")
                .filter(|id| !id.is_empty())
                .and_then(|id| self.record(id))
        {
            let mut nested = Vec::new();
            self.render_record(
                &bonus_record,
                RecordRole {
                    base: role.base,
                    is_base: false,
                    variable_number: 0,
                },
                &mut nested,
                depth + 1,
            );
            for line in nested {
                results.push(StatLine::new(line.color, format!("    {}", line.text)));
            }
            results.push(StatLine::new(COLOR_MUNDANE, ""));
        }
        let is_scroll_or_potion = matches!(
            record.record_type.to_uppercase().as_str(),
            "ONESHOT_SCROLL"
                | "ONESHOT_POTIONHEALTH"
                | "ONESHOT_POTIONMANA"
                | "ONESHOT_SCROLL_ETERNAL"
        );
        if full_upper == "ITEMSKILLNAME" || (is_scroll_or_potion && full_upper == "SKILLNAME") {
            self.skill_description_and_effects(record, role, variable, results, depth);
        }
    }

    fn racial_bonus(
        &self,
        record: &DbRecord,
        attribute: &AttributeData,
        variable: &DbVariable,
        variable_number: usize,
        results: &mut Vec<StatLine>,
        color: &mut Option<char>,
    ) -> Option<String> {
        let DbValues::Strings(races) = &record.variable("racialBonusRace")?.values else {
            return None;
        };
        let mut line: Option<String> = None;
        for race in races {
            let race_name = self
                .tag(&format!("racialBonusRace{race}"))
                .map_or_else(|| race.clone(), strip_color_tags);
            let format_tag = first_upper(&attribute.full_attribute);
            let spec = self
                .tag_spec(&format_tag)
                .unwrap_or_else(|| parse_format(&format!("{format_tag} {{%.0f0}} {{%s1}}")));
            if let Some(previous) = line.take() {
                results.push(StatLine::new(COLOR_MAGICAL, previous));
            }
            line = Some(spec.format(&[
                Arg::Number(float_at_capped(variable, variable_number)),
                Arg::Text(race_name),
            ]));
            *color = Some(COLOR_MAGICAL);
        }
        line
    }

    fn augment_mastery(
        &self,
        record: &DbRecord,
        attribute: &AttributeData,
        variable: &DbVariable,
        color: &mut Option<char>,
    ) -> String {
        let digit = attribute.full_attribute.chars().nth(19).unwrap_or('1');
        let mastery_id = record
            .string(&format!("augmentMasteryName{digit}"))
            .unwrap_or_default()
            .to_string();
        let skill_name = self
            .record(&mastery_id)
            .and_then(|skill| {
                skill
                    .string("skillDisplayName")
                    .filter(|tag| !tag.is_empty())
                    .and_then(|tag| self.tag(tag))
                    .map(strip_color_tags)
            })
            .unwrap_or_else(|| {
                *color = Some(COLOR_UNKNOWN);
                file_stem(&mastery_id)
            });
        if color.is_none() {
            *color = Some(COLOR_MAGICAL);
        }
        self.tag_spec("ItemMasteryIncrement")
            .unwrap_or_else(|| parse_format("?+{%d0} to skills in {%s1}?"))
            .format(&[Arg::Number(float_at(variable, 0)), Arg::Text(skill_name)])
    }

    fn augment_skill(
        &self,
        record: &DbRecord,
        attribute: &AttributeData,
        variable: &DbVariable,
        color: &mut Option<char>,
    ) -> Option<String> {
        let digit = attribute.full_attribute.chars().nth(17).unwrap_or('1');
        let skill_id = record
            .string(&format!("augmentSkillName{digit}"))
            .filter(|id| !id.is_empty())?
            .to_string();
        let skill_name = self
            .skill_display_name(&skill_id, 0)
            .unwrap_or_else(|| file_stem(&skill_id));
        *color = Some(COLOR_MAGICAL);
        Some(
            self.tag_spec("ItemSkillIncrement")
                .unwrap_or_else(|| parse_format("?+{%d0} to skill {%s1}?"))
                .format(&[Arg::Number(float_at(variable, 0)), Arg::Text(skill_name)]),
        )
    }

    /// A skill's display name, following buff and pet-modifier
    /// redirections like the reference.
    fn skill_display_name(&self, skill_id: &str, depth: u32) -> Option<String> {
        if depth > MAX_DEPTH {
            return None;
        }
        let skill = self.record(skill_id)?;
        if let Some(buff_id) = skill.string("buffSkillName").filter(|id| !id.is_empty()) {
            let buff_id = buff_id.to_string();
            return self.skill_display_name(&buff_id, depth + 1);
        }
        if let Some(tag) = skill
            .string("skillDisplayName")
            .filter(|tag| !tag.is_empty())
        {
            return self.tag(tag).map(strip_color_tags);
        }
        if skill.record_type.contains("PetModifier")
            && let Some(pet_skill) = skill.string("petSkillName").filter(|id| !id.is_empty())
        {
            let pet_skill = pet_skill.to_string();
            return self.skill_display_name(&pet_skill, depth + 1);
        }
        None
    }

    fn reagent_name(&self, variable: &DbVariable, color: &mut Option<char>) -> Option<String> {
        let reagent_id = first_string(variable)?;
        let reagent = self.record(reagent_id)?;
        let tag = reagent
            .string("description")
            .filter(|tag| !tag.is_empty())?;
        let name = self.tag(tag).map(strip_color_tags)?;
        *color = Some(COLOR_COMMON);
        Some(name)
    }

    fn granted_skill(
        &self,
        record: &DbRecord,
        variable: &DbVariable,
        color: &mut Option<char>,
        results: &mut Vec<StatLine>,
        _depth: u32,
    ) -> Option<String> {
        let skill_id = first_string(variable)?;
        let skill = self.record(skill_id)?;
        results.push(StatLine::new(COLOR_MUNDANE, ""));
        let grant_label = self
            .tag("tagItemGrantSkill")
            .map_or_else(|| "Grants Skill :".to_string(), strip_color_tags);
        results.push(StatLine::new(COLOR_MUNDANE, grant_label));
        let skill_name = self
            .skill_display_name(skill_id, 0)
            .unwrap_or_else(|| file_stem(skill_id));
        let activation = record
            .string("itemSkillAutoController")
            .filter(|id| !id.is_empty())
            .and_then(|id| self.record(id))
            .and_then(|controller| controller.string("triggerType").map(ToString::to_string))
            .and_then(|trigger| {
                let tag = match trigger.to_uppercase().as_str() {
                    "LOWHEALTH" => "xtagAutoSkillCondition01",
                    "LOWMANA" => "xtagAutoSkillCondition02",
                    "HITBYENEMY" => "xtagAutoSkillCondition03",
                    "HITBYMELEE" => "xtagAutoSkillCondition04",
                    "HITBYPROJECTILE" => "xtagAutoSkillCondition05",
                    "CASTBUFF" => "xtagAutoSkillCondition06",
                    "ATTACKENEMY" => "xtagAutoSkillCondition07",
                    "ONEQUIP" => "xtagAutoSkillCondition08",
                    _ => return None,
                };
                self.tag(tag).map(strip_color_tags)
            });
        *color = Some(if activation.is_none() {
            COLOR_MAGICAL
        } else {
            COLOR_MUNDANE
        });
        let _ = skill;
        Some(match activation {
            Some(text) => format!("{skill_name} {text}"),
            None => skill_name,
        })
    }

    fn skill_effect(
        &self,
        group_data: &AttributeData,
        attribute: &AttributeData,
        variable: &DbVariable,
        variable_number: usize,
        color: &mut Option<char>,
    ) -> String {
        let label_tag = attribute_text_tag(group_data);
        let label = if label_tag.is_empty() {
            *color = Some(COLOR_UNKNOWN);
            parse_format(&format!("?{}?", group_data.full_attribute))
        } else if let Some(text) = self.tag(&label_tag) {
            parse_format(text)
        } else {
            *color = Some(COLOR_UNKNOWN);
            parse_format(&format!("?{label_tag}?"))
        };
        let full_upper = attribute.full_attribute.to_uppercase();
        let two_param_tag = if full_upper.ends_with("COST") || full_upper.ends_with("LEVEL") {
            Some("SkillIntFormat")
        } else if full_upper.ends_with("DURATION") {
            Some("SkillSecondFormat")
        } else if full_upper.ends_with("RADIUS") {
            Some("SkillDistanceFormat")
        } else {
            None
        };
        let value = float_at_capped(variable, variable_number);
        match two_param_tag {
            None => {
                if color.is_none() {
                    *color = Some(COLOR_MAGICAL);
                }
                label.format(&[Arg::Number(value)])
            }
            Some(tag) => {
                let spec = self.tag_spec(tag).unwrap_or_else(|| {
                    *color = Some(COLOR_UNKNOWN);
                    parse_format("?{%.0f0} {%s1}?")
                });
                if color.is_none() {
                    *color = Some(COLOR_MAGICAL);
                }
                spec.format(&[Arg::Number(value), Arg::Text(label.format(&[]))])
            }
        }
    }

    fn raw_attribute(
        &self,
        attribute: &AttributeData,
        variable: &DbVariable,
        variable_number: usize,
        color: &mut Option<char>,
    ) -> String {
        let label_tag = attribute_text_tag(attribute);
        let label = if label_tag.is_empty() {
            *color = Some(COLOR_UNKNOWN);
            format!("?{}?", attribute.full_attribute)
        } else if let Some(text) = self.tag(&label_tag) {
            text.to_string()
        } else {
            *color = Some(COLOR_UNKNOWN);
            format!("?{label_tag}?")
        };
        let spec = parse_format(&label);
        let line = if spec.has_args() {
            if color.is_none() {
                *color = Some(COLOR_MAGICAL);
            }
            spec.format(&[Arg::Number(float_at_capped(variable, variable_number))])
        } else {
            format!("{}: {}", variable.name, values_display(variable))
        };
        if color.is_none() {
            *color = Some(COLOR_UNKNOWN);
        }
        line
    }

    /// `GetSkillDescriptionAndEffects`: description text, level, and
    /// the granted skill's own effects (or pet stats for summons).
    fn skill_description_and_effects(
        &self,
        record: &DbRecord,
        role: RecordRole<'_>,
        variable: &DbVariable,
        results: &mut Vec<StatLine>,
        depth: u32,
    ) {
        if depth >= MAX_DEPTH {
            return;
        }
        let Some(skill_id) = first_string(variable) else {
            return;
        };
        let is_scroll = record.record_type.eq_ignore_ascii_case("OneShot_Scroll")
            || record
                .record_type
                .eq_ignore_ascii_case("OneShot_Scroll_Eternal");
        let auto_controller = record
            .string("itemSkillAutoController")
            .filter(|id| !id.is_empty());
        if auto_controller.is_none() && !is_scroll && !is_potion_class(&record.record_type) {
            return;
        }
        let Some(skill) = self.record(skill_id) else {
            return;
        };
        let columns = auto_controller.map_or(30, |id| id.len().max(30));
        let buff_id = skill
            .string("buffSkillName")
            .filter(|id| !id.is_empty())
            .map(str::to_string);
        let description_source = match &buff_id {
            Some(id) => self.record(id),
            None => Some(skill.clone()),
        };
        if !is_scroll
            && let Some(source) = &description_source
            && let Some(description) = source
                .string("skillBaseDescription")
                .filter(|tag| !tag.is_empty())
                .and_then(|tag| self.tag(tag))
        {
            for wrapped in wrap_words(&strip_color_tags(description), columns) {
                results.push(StatLine::new(COLOR_MUNDANE, format!("    {wrapped}")));
            }
            let level = record.integer("itemSkillLevel").unwrap_or(0);
            if level > 0 {
                let text = self
                    .tag_spec("MenuLevel")
                    .unwrap_or_else(|| parse_format("Level:   {%d0}"))
                    .format(&[Arg::Number(level as f32)]);
                results.push(StatLine::new(COLOR_MUNDANE, text));
            }
        }
        let named = !skill
            .string("skillDisplayName")
            .unwrap_or_default()
            .is_empty();
        if !named && buff_id.is_none() && !is_scroll {
            return;
        }
        if !is_scroll && !is_potion_class(&record.record_type) {
            results.push(StatLine::new(COLOR_MUNDANE, ""));
        }
        if skill.record_type.eq_ignore_ascii_case("SKILL_SPAWNPET") {
            self.pet_stats(&skill, role, results, depth);
        } else {
            let effect_record = match &buff_id {
                Some(id) => self.record(id),
                None => Some(skill.clone()),
            };
            if let Some(effect_record) = effect_record {
                self.render_record(
                    &effect_record,
                    RecordRole {
                        base: role.base,
                        is_base: false,
                        variable_number: 0,
                    },
                    results,
                    depth + 1,
                );
            }
        }
    }

    /// `ConvertPetStats`: a summon's headline numbers and skills.
    fn pet_stats(
        &self,
        skill: &DbRecord,
        role: RecordRole<'_>,
        results: &mut Vec<StatLine>,
        depth: u32,
    ) {
        let mut push = |color, text: String| results.push(StatLine::new(color, text));
        let limit = skill.integer("petLimit").unwrap_or(0);
        if limit > 1 {
            let text = self
                .tag_spec("SkillPetLimit")
                .unwrap_or_else(|| parse_format("{%d0} Summon Limit"))
                .format(&[Arg::Number(limit as f32)]);
            push(COLOR_MUNDANE, text);
        }
        let Some(pet) = skill
            .string("spawnObjects")
            .filter(|id| !id.is_empty())
            .and_then(|id| self.record(id))
        else {
            return;
        };
        let pet_name = pet
            .string("description")
            .and_then(|tag| self.tag(tag))
            .map_or_else(String::new, strip_color_tags);
        let heading = self
            .tag_spec("SkillPetDescriptionHeading")
            .unwrap_or_else(|| parse_format("{%s0} Attributes:"))
            .format(&[Arg::Text(pet_name.clone())]);
        push(COLOR_MUNDANE, heading);
        let ttl = skill.float("spawnObjectsTimeToLive").unwrap_or(0.0);
        let ttl_text = self
            .tag_spec("tagSkillPetTimeToLive")
            .unwrap_or_else(|| parse_format("Life Time {%.1f0} Seconds"))
            .format(&[Arg::Number(ttl)]);
        push(COLOR_MUNDANE, ttl_text);
        let life = pet.float("characterLife").unwrap_or(0.0);
        if life != 0.0 {
            let text = self
                .tag_spec("SkillPetDescriptionHealth")
                .unwrap_or_else(|| parse_format("{%.0f0}  Health"))
                .format(&[Arg::Number(life)]);
            push(COLOR_MUNDANE, text);
        }
        let mana = pet.float("characterMana").unwrap_or(0.0);
        if mana != 0.0 {
            let text = self
                .tag_spec("SkillPetDescriptionMana")
                .unwrap_or_else(|| parse_format("{%.0f0}  Energy"))
                .format(&[Arg::Number(mana)]);
            push(COLOR_MUNDANE, text);
        }
        push(COLOR_MUNDANE, String::new());
        let abilities = self
            .tag_spec("tagSkillPetAbilities")
            .unwrap_or_else(|| parse_format("{%s0} Abilities:"))
            .format(&[Arg::Text(pet_name)]);
        push(COLOR_MUNDANE, abilities);
        let (min, max) = (
            pet.float("handHitDamageMin").unwrap_or(0.0),
            pet.float("handHitDamageMax").unwrap_or(0.0),
        );
        if min > 1.0 || max > 2.0 {
            let text = if max == 0.0 || (min - max).abs() < f32::EPSILON {
                self.tag_spec("SkillPetDescriptionDamageMinOnly")
                    .unwrap_or_else(|| parse_format("{%.0f0}  Damage"))
                    .format(&[Arg::Number(min)])
            } else {
                self.tag_spec("SkillPetDescriptionDamageMinMax")
                    .unwrap_or_else(|| parse_format("{%.0f0} - {%.0f1}  Damage"))
                    .format(&[Arg::Number(min), Arg::Number(max)])
            };
            results.push(StatLine::new(COLOR_MUNDANE, text));
        }
        for offset in 0..17 {
            let Some(pet_skill_id) = pet
                .string(&format!("skillName{offset}"))
                .filter(|id| !id.is_empty() && id.to_lowercase().starts_with("records"))
            else {
                continue;
            };
            let Some(pet_skill) = self.record(pet_skill_id) else {
                continue;
            };
            if pet_skill.record_type.eq_ignore_ascii_case("SKILL_PASSIVE") {
                continue;
            }
            let effect_record = match pet_skill
                .string("buffSkillName")
                .filter(|id| !id.is_empty())
            {
                Some(buff_id) => self.record(buff_id),
                None => Some(pet_skill.clone()),
            };
            let name = self
                .skill_display_name(pet_skill_id, 0)
                .unwrap_or_else(|| file_stem(pet_skill_id));
            results.push(StatLine::new(COLOR_MUNDANE, name));
            if let Some(effect_record) = effect_record {
                self.render_record(
                    &effect_record,
                    RecordRole {
                        base: role.base,
                        is_base: false,
                        variable_number: 0,
                    },
                    results,
                    depth + 1,
                );
            }
            results.push(StatLine::new(COLOR_MUNDANE, String::new()));
        }
    }

    /// `GetTriggeredSkillLevel`: skill records granted by the base
    /// item use the base's `itemSkillLevel` as the value index.
    fn triggered_skill_level(
        &self,
        base: &DbRecord,
        skill_record: &DbRecord,
        variable_number: usize,
    ) -> usize {
        let Some(_controller) = base
            .string("itemSkillAutoController")
            .filter(|id| !id.is_empty())
        else {
            return variable_number;
        };
        let level = base.integer("itemSkillLevel").unwrap_or(0);
        let item_skill = base.string("itemSkillName").unwrap_or_default();
        if skill_record
            .record_type
            .to_uppercase()
            .starts_with("SKILLBUFF")
        {
            let buff_matches = self
                .record(item_skill)
                .and_then(|skill| skill.string("buffSkillName").map(str::to_string))
                .is_some();
            if buff_matches {
                return (level.max(1) - 1) as usize;
            }
            variable_number
        } else if !item_skill.is_empty() {
            (level.max(1) - 1) as usize
        } else {
            variable_number
        }
    }
}

/// The variables that survive the display filters
/// (`FilterValue` + `FilterKey` + `FilterRequirements`).
fn displayable_variables(record: &DbRecord) -> impl Iterator<Item = &DbVariable> {
    record
        .variables()
        .filter(|variable| !filter_value(variable))
        .filter(|variable| !filter_key(&variable.name))
        .filter(|variable| Requirement::from_variable(&variable.name).is_none())
}

/// The record's contribution to `totalAttCount`, mirroring the
/// counting rules inside `GetAttributesFromRecord`.
fn attribute_count(record: &DbRecord) -> i32 {
    const UNCOUNTED: &[&str] = &[
        "CHARACTERBASEATTACKSPEEDTAG",
        "OFFENSIVEPHYSICALMIN",
        "OFFENSIVEPHYSICALMAX",
        "DEFENSIVEPROTECTION",
        "DEFENSIVEBLOCK",
        "BLOCKRECOVERYTIME",
        "OFFENSIVEGLOBALCHANCE",
        "RETALIATIONGLOBALCHANCE",
        "OFFENSIVEPIERCERATIOMIN",
    ];
    let mut counted_groups: Vec<String> = Vec::new();
    let mut count = 0;
    for variable in displayable_variables(record) {
        let data = attribute_data(&variable.name)
            .cloned()
            .unwrap_or_else(|| unknown_attribute(&variable.name));
        let group = effect_group(&data);
        if counted_groups.contains(&group) {
            continue;
        }
        let upper = variable.name.to_uppercase();
        if upper.contains("CHANCE") || upper.contains("DURATION") {
            continue;
        }
        if UNCOUNTED.contains(&upper.as_str()) {
            continue;
        }
        if upper.starts_with("AUGMENTSKILLLEVEL") {
            count += integer_at(variable, 0);
        } else {
            count += 1;
        }
        counted_groups.push(group);
    }
    count
}

/// `GetRequirementsFromRecord`: the explicit requirement variables,
/// max-merged.
pub(crate) fn explicit_requirements(record: &DbRecord) -> Vec<(Requirement, i32)> {
    let mut requirements: Vec<(Requirement, i32)> = Vec::new();
    for variable in record.variables() {
        if filter_value(variable) {
            continue;
        }
        let Some(key) = Requirement::from_variable(&variable.name) else {
            continue;
        };
        let value = integer_at(variable, 0);
        match requirements
            .iter_mut()
            .find(|(existing, _)| *existing == key)
        {
            Some((_, existing_value)) => *existing_value = (*existing_value).max(value),
            None => requirements.push((key, value)),
        }
    }
    requirements
}

fn effect_group(data: &AttributeData) -> String {
    if data.effect_type == EffectType::DamageQualifier {
        "DamageQualifier:DamageQualifier".to_string()
    } else {
        format!("{:?}:{}", data.effect_type, data.effect)
    }
}

fn group_effect_type(vars: &[&DbVariable]) -> EffectType {
    attribute_data(&vars[0].name).map_or(EffectType::Other, |data| data.effect_type)
}

fn group_has_variable(vars: &[&DbVariable], name: &str) -> bool {
    vars.iter().any(|variable| {
        attribute_data(&variable.name).is_some_and(|data| data.variable.eq_ignore_ascii_case(name))
    })
}

/// `ItemAttributeListCompare.CalcOrder`.
fn group_order(vars: &[&DbVariable], is_armor_or_shield: bool) -> i32 {
    const GLOBAL: i32 = 10_000_000;
    let first = vars[0];
    if first.name.eq_ignore_ascii_case("itemSkillName") {
        return 4_000_000;
    }
    let Some(data) = attribute_data(&first.name) else {
        return 3_000_000;
    };
    let base = |effect_type: EffectType, suborder: i32| {
        let mut type_order = effect_type.order();
        if is_armor_or_shield {
            if effect_type == EffectType::ShieldEffect {
                type_order = 0;
            } else if effect_type == EffectType::Defense {
                type_order = 1;
            } else if type_order < EffectType::Defense.order() {
                type_order += 1;
            }
        }
        ((1000 * (1 + type_order)) + suborder) * 10
    };
    let mut order = base(data.effect_type, data.suborder);
    if data
        .full_attribute
        .eq_ignore_ascii_case("characterBaseAttackSpeedTag")
    {
        let piercing = attribute_data("offensivePierceRatioMin")
            .expect("dictionary always contains offensivePierceRatioMin");
        order = base(piercing.effect_type, piercing.suborder) + 1;
    } else if data
        .full_attribute
        .eq_ignore_ascii_case("retaliationGlobalChance")
    {
        order = base(EffectType::Retaliation, 0) - 1 + GLOBAL;
    } else if data
        .full_attribute
        .eq_ignore_ascii_case("offensiveGlobalChance")
    {
        order = base(EffectType::Offense, 0) - 1 + GLOBAL;
    } else if is_armor_or_shield
        && data
            .full_attribute
            .eq_ignore_ascii_case("offensivePhysicalMin")
    {
        let block = attribute_data("blockRecoveryTime")
            .expect("dictionary always contains blockRecoveryTime");
        order = base(block.effect_type, block.suborder) + 1;
    }
    if group_has_variable(vars, "Global") {
        order += GLOBAL;
    }
    order
}

/// `FilterValue(variable, false)`: all-zero numerics and all strings
/// except the special display names are invisible.
fn filter_value(variable: &DbVariable) -> bool {
    match &variable.values {
        DbValues::Integers(values) => values.iter().all(|&value| value == 0),
        DbValues::Floats(values) => values.iter().all(|&value| value == 0.0),
        DbValues::Booleans(values) => values.iter().all(|&value| !value),
        DbValues::Strings(values) => {
            let allowed = [
                "characterBaseAttackSpeedTag",
                "itemSkillName",
                "skillName",
                "petBonusName",
            ]
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&variable.name))
                || dictionary::is_reagent(&variable.name);
            !(allowed && values.iter().any(|value| !value.is_empty()))
        }
    }
}

/// `FilterKey`: bookkeeping variables that never display.
fn filter_key(name: &str) -> bool {
    const UNWANTED: &[&str] = &[
        "DEFENSIVEABSORPTION",
        "MAXTRANSPARENCY",
        "SCALE",
        "CASTSSHADOWS",
        "MARKETADJUSTMENTPERCENT",
        "LOOTRANDOMIZERCOST",
        "LOOTRANDOMIZERJITTER",
        "ACTORHEIGHT",
        "ACTORRADIUS",
        "SHADOWBIAS",
        "ITEMLEVEL",
        "ITEMCOST",
        "COMPLETEDRELICLEVEL",
        "CHARACTERBASEATTACKSPEED",
        "HIDESUFFIXNAME",
        "HIDEPREFIXNAME",
        "AMULET",
        "RING",
        "HELMET",
        "GREAVES",
        "ARMBAND",
        "BODYARMOR",
        "BOW",
        "SPEAR",
        "STAFF",
        "MACE",
        "SWORD",
        "RANGEDONEHAND",
        "AXE",
        "SHIELD",
        "BRACELET",
        "BLOCKABSORPTION",
        "ITEMCOSTSCALEPERCENT",
        "ITEMSKILLLEVEL",
        "USEDELAYTIME",
        "CAMERASHAKEAMPLITUDE",
        "SKILLMAXLEVEL",
        "EXPANSIONTIME",
        "SKILLTIER",
        "CAMERASHAKEDURATIONSECS",
        "SKILLULTIMATELEVEL",
        "SKILLCONNECTIONSPACING",
        "PETBURSTSPAWN",
        "PETLIMIT",
        "ISPETDISPLAYABLE",
        "SPAWNOBJECTSTIMETOLIVE",
        "SKILLPROJECTILENUMBER",
        "SKILLMASTERYLEVELREQUIRED",
        "EXCLUDERACIALDAMAGE",
        "SKILLWEAPONTINTRED",
        "SKILLWEAPONTINTGREEN",
        "SKILLWEAPONTINTBLUE",
        "DEBUFSKILL",
        "HIDEFROMUI",
        "INSTANTCAST",
        "WAVEENDWIDTH",
        "WAVEDISTANCE",
        "WAVEDEPTH",
        "WAVESTARTWIDTH",
        "RAGDOLLAMPLIFICATION",
        "WAVETIME",
        "SPARKGAP",
        "SPARKCHANCE",
        "PROJECTILEUSESALLDAMAGE",
        "DROPOFFSET",
        "DROPHEIGHT",
        "NUMPROJECTILES",
        "QUEST",
        "CANNOTPICKUPMULTIPLE",
        "BONUSLIFEPERCENT",
        "BONUSLIFEPOINTS",
        "BONUSMANAPERCENT",
        "BONUSMANAPOINTS",
        "DISPLAYASQUESTITEM",
        "ACTORSCALE",
        "ACTORSCALETIME",
        "SPAWNOBJECTSDISTANCEINCREMENT",
        "SPAWNOBJECTSDISTANCEINNERCIRCLE",
        "SPAWNOBJECTSNUMBEROFRINGS",
        "SPAWNOBJECTSSPACINGANGLE",
        "CONTAGIONINTERVAL",
        "CONTAGIONLIMIT",
        "CONTAGIONMAXSPREAD",
        "CONTAGIONRADIUS",
        "NOHIGHLIGHTDEFAULTCOLORA",
        "FORCEIGNORERUNSPEEDCAPS",
        "LOOTRANDOMIZERSCALE",
        "PROJECTILEFRAGMENTSLAUNCHNUMBERMAX",
        "PROJECTILEFRAGMENTSLAUNCHNUMBERMIN",
        "SPAWNOBJECTSRANDOMROTATION",
        "SKILLPROJECTILETARGETGROUNDONLY",
        "OFFENSIVETOTALDAMAGEGLOBAL",
        "OFFENSIVETOTALDAMAGEXOR",
        "SKILLALLOWSWARMUP",
        "ONHITACTIVATIONCHANCE",
        "DECREMENTSTATTYPE",
        "ALLSKILLENHANCEMENT",
    ];
    let upper = name.to_uppercase();
    UNWANTED.contains(&upper.as_str())
        || upper.ends_with("SOUND")
        || upper.ends_with("MESH")
        || upper.starts_with("BODYMASK")
}

/// Effects whose magnitude ignores the duration when scaling —
/// `durationIndependentEffects`.
fn duration_reliant(effect: &str) -> bool {
    const INDEPENDENT: &[&str] = &[
        "OFFENSIVESLOWTOTALSPEED",
        "OFFENSIVESLOWATTACKSPEED",
        "OFFENSIVESLOWRUNSPEED",
        "OFFENSIVESLOWOFFENSIVEABILITY",
        "OFFENSIVESLOWDEFENSIVEABILITY",
        "OFFENSIVESLOWOFFENSIVEREDUCTION",
        "OFFENSIVESLOWDEFENSIVEREDUCTION",
        "OFFENSIVETOTALDAMAGEREDUCTIONPERCENT",
        "OFFENSIVETOTALDAMAGEREDUCTIONABSOLUTE",
        "OFFENSIVETOTALRESISTANCEREDUCTIONPERCENT",
        "OFFENSIVETOTALRESISTANCEREDUCTIONABSOLUTE",
    ];
    !INDEPENDENT.contains(&effect.to_uppercase().as_str())
}

fn range_format_tag(effect: &str) -> &'static str {
    if influence_effect(effect) {
        "DamageInfluenceRangeFormat"
    } else if effect.eq_ignore_ascii_case("defensiveBlock") {
        "DefenseBlock"
    } else {
        "DamageRangeFormat"
    }
}

fn single_format_tag(effect: &str) -> &'static str {
    if influence_effect(effect) {
        "DamageInfluenceSingleFormat"
    } else {
        "DamageSingleFormat"
    }
}

fn influence_effect(effect: &str) -> bool {
    const SUFFIXES: &[&str] = &[
        "STUN",
        "FREEZE",
        "PETRIFY",
        "TRAP",
        "CONVERT",
        "FEAR",
        "CONFUSION",
        "DISRUPTION",
    ];
    let upper = effect.to_uppercase();
    SUFFIXES.iter().any(|suffix| upper.ends_with(suffix))
}

/// The base-record stats the game shows white instead of blue.
fn mundane_base_stat(record: &DbRecord, effect: &str) -> bool {
    if is_weapon_class(&record.record_type) {
        [
            "offensivePierceRatio",
            "offensivePhysical",
            "offensiveBaseFire",
            "offensiveBaseCold",
            "offensiveBaseLightning",
            "offensiveBaseLife",
        ]
        .iter()
        .any(|base| base.eq_ignore_ascii_case(effect))
    } else if is_shield_class(&record.record_type) {
        ["defensiveBlock", "blockRecoveryTime", "offensivePhysical"]
            .iter()
            .any(|base| base.eq_ignore_ascii_case(effect))
    } else {
        false
    }
}

fn is_weapon_class(class: &str) -> bool {
    class.to_uppercase().starts_with("WEAPON") && !is_shield_class(class)
}

fn is_shield_class(class: &str) -> bool {
    class.eq_ignore_ascii_case("WeaponArmor_Shield")
}

fn is_armor_class(class: &str) -> bool {
    class.to_uppercase().starts_with("ARMORPROTECTIVE")
}

fn is_relic_class(class: &str) -> bool {
    class.eq_ignore_ascii_case("ItemRelic") || class.eq_ignore_ascii_case("ItemCharm")
}

fn is_potion_class(class: &str) -> bool {
    class.eq_ignore_ascii_case("OneShot_PotionHealth")
        || class.eq_ignore_ascii_case("OneShot_PotionMana")
        || class.eq_ignore_ascii_case("OneShot_Scroll_Eternal")
}

/// The gear-class → cost-equation prefix map (`GearType`).
fn requirement_equation_prefix(class: &str) -> Option<&'static str> {
    let map = [
        ("ArmorProtective_Head", "head"),
        ("ArmorProtective_UpperBody", "upperBody"),
        ("ArmorProtective_Forearm", "forearm"),
        ("ArmorProtective_LowerBody", "lowerBody"),
        ("ArmorJewelry_Ring", "ring"),
        ("ArmorJewelry_Amulet", "amulet"),
        ("ItemArtifact", ""),
        ("WeaponHunting_Spear", "spear"),
        ("WeaponMagical_Staff", "staff"),
        ("WeaponHunting_RangedOneHand", "bow"),
        ("WeaponHunting_Bow", "bow"),
        ("WeaponMelee_Sword", "sword"),
        ("WeaponMelee_Mace", "mace"),
        ("WeaponMelee_Axe", "axe"),
        ("WeaponArmor_Shield", "shield"),
    ];
    map.iter()
        .find(|(gear_class, _)| gear_class.eq_ignore_ascii_case(class))
        .map(|(_, prefix)| *prefix)
}

fn float_at(variable: &DbVariable, index: usize) -> f32 {
    match &variable.values {
        DbValues::Integers(values) => values.get(index).copied().unwrap_or(0) as f32,
        DbValues::Floats(values) => values.get(index).copied().unwrap_or(0.0),
        DbValues::Booleans(values) => f32::from(values.get(index).copied().unwrap_or(false)),
        DbValues::Strings(_) => 0.0,
    }
}

/// Value at `index`, capped to the last value — the reference's
/// `Math.Min(NumberOfValues - 1, varNum)` idiom.
fn float_at_capped(variable: &DbVariable, index: usize) -> f32 {
    float_at(variable, index.min(variable.len().saturating_sub(1)))
}

fn integer_at(variable: &DbVariable, index: usize) -> i32 {
    match &variable.values {
        DbValues::Integers(values) => values.get(index).copied().unwrap_or(0),
        DbValues::Floats(values) => values.get(index).copied().unwrap_or(0.0) as i32,
        DbValues::Booleans(values) => i32::from(values.get(index).copied().unwrap_or(false)),
        DbValues::Strings(_) => 0,
    }
}

fn first_string(variable: &DbVariable) -> Option<&str> {
    match &variable.values {
        DbValues::Strings(values) => values.first().map(String::as_str).filter(|s| !s.is_empty()),
        DbValues::Integers(_) | DbValues::Floats(_) | DbValues::Booleans(_) => None,
    }
}

fn values_display(variable: &DbVariable) -> String {
    match &variable.values {
        DbValues::Integers(values) => join_values(values),
        DbValues::Floats(values) => join_values(values),
        DbValues::Strings(values) => values.join(", "),
        DbValues::Booleans(values) => join_values(values),
    }
}

fn join_values<T: std::fmt::Display>(values: &[T]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn first_upper(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn file_stem(path: &str) -> String {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    name.rsplit_once('.')
        .map_or_else(|| name.to_string(), |(stem, _)| stem.to_string())
}

fn thousands(value: i32) -> String {
    let raw = value.abs().to_string();
    let mut grouped = String::new();
    for (position, digit) in raw.chars().rev().enumerate() {
        if position > 0 && position % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let grouped: String = grouped.chars().rev().collect();
    if value < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

/// Skips consecutive duplicate blank lines so nested renders don't
/// pile up spacers.
fn push_line(results: &mut Vec<StatLine>, line: StatLine) {
    if line.text.trim().is_empty()
        && results
            .last()
            .is_none_or(|last| last.text.trim().is_empty())
    {
        return;
    }
    results.push(line);
}
