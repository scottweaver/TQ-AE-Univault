//! The Titan Quest AE look: a bronze-and-gold palette over dark
//! leather-and-olive surfaces, classical serif faces (Cinzel for
//! headings, Alegreya for body — both OFL, bundled under
//! `assets/fonts/`), and squared-off chrome. [`apply`] installs all
//! of it once at startup; the constants are the palette for the
//! custom-painted surfaces (sack grids, item tiles, tooltips).

use std::collections::BTreeMap;
use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

pub const GOLD: Color32 = Color32::from_rgb(208, 172, 92);
pub const GOLD_DIM: Color32 = Color32::from_rgb(128, 103, 54);
pub const BRONZE_FAINT: Color32 = Color32::from_rgb(94, 77, 44);
pub const HEADING_GOLD: Color32 = Color32::from_rgb(219, 185, 110);
pub const TEXT: Color32 = Color32::from_rgb(225, 210, 177);
pub const TEXT_STRONG: Color32 = Color32::from_rgb(243, 233, 210);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(160, 145, 115);
pub const SURFACE: Color32 = Color32::from_rgb(31, 27, 18);
pub const SURFACE_RAISED: Color32 = Color32::from_rgb(48, 40, 25);
pub const SURFACE_DEEP: Color32 = Color32::from_rgb(18, 15, 10);
pub const POPUP: Color32 = Color32::from_rgb(13, 11, 8);
pub const GRID_BG: Color32 = Color32::from_rgb(33, 36, 24);
pub const GRID_LINE: Color32 = Color32::from_rgb(47, 51, 34);
pub const TILE_BG: Color32 = Color32::from_rgb(43, 47, 31);
pub const TILE_EDGE: Color32 = Color32::from_rgb(66, 70, 48);

const CINZEL: &[u8] = include_bytes!("../assets/fonts/Cinzel.ttf");
const ALEGREYA: &[u8] = include_bytes!("../assets/fonts/Alegreya.ttf");

/// Installs the whole look on the context — fonts, text styles, and
/// widget visuals — pinned to the dark theme regardless of the OS
/// preference.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_fonts(font_definitions());
    ctx.all_styles_mut(|style| {
        style.text_styles = text_styles();
        style.visuals = visuals();
    });
}

/// Heading text in the classical-caps face and gold — pane and
/// dialog titles.
pub fn heading(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into())
        .text_style(TextStyle::Heading)
        .color(HEADING_GOLD)
}

/// A file path under a pane heading: small, dim monospace, so it
/// informs without competing with the panes.
pub fn path_text(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into())
        .monospace()
        .size(10.5)
        .color(TEXT_WEAK)
}

/// A section title inside a pane (Equipment, Sack N): the classical
/// face at body scale, plain gold — a rank below [`heading`].
pub fn section(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into())
        .font(FontId::new(15.0, heading_family()))
        .color(GOLD)
}

fn heading_family() -> FontFamily {
    FontFamily::Name("tq-heading".into())
}

fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "alegreya".to_owned(),
        Arc::new(egui::FontData::from_static(ALEGREYA)),
    );
    fonts.font_data.insert(
        "cinzel".to_owned(),
        Arc::new(egui::FontData::from_static(CINZEL)),
    );
    let proportional = fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .expect("egui's defaults always define the Proportional family");
    proportional.insert(0, "alegreya".to_owned());
    // Cinzel first, then the body stack so symbol glyphs Cinzel
    // lacks (arrows, ⌘) still resolve.
    let headings = std::iter::once("cinzel".to_owned())
        .chain(proportional.iter().cloned())
        .collect();
    fonts.families.insert(heading_family(), headings);
    fonts
}

fn text_styles() -> BTreeMap<TextStyle, FontId> {
    [
        (TextStyle::Heading, FontId::new(19.0, heading_family())),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.0, FontFamily::Monospace),
        ),
    ]
    .into()
}

fn visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = SURFACE;
    visuals.window_fill = SURFACE;
    visuals.window_stroke = Stroke::new(1.5, GOLD_DIM);
    visuals.window_corner_radius = CornerRadius::same(4);
    visuals.menu_corner_radius = CornerRadius::same(3);
    visuals.extreme_bg_color = SURFACE_DEEP;
    visuals.code_bg_color = SURFACE_RAISED;
    visuals.faint_bg_color = Color32::from_rgb(39, 34, 22);
    visuals.selection.bg_fill = Color32::from_rgb(110, 88, 42);
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(232, 200, 120));
    visuals.hyperlink_color = GOLD;
    visuals.warn_fg_color = Color32::from_rgb(235, 180, 76);
    visuals.error_fg_color = Color32::from_rgb(230, 105, 85);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = SURFACE;
    widgets.noninteractive.weak_bg_fill = SURFACE;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, BRONZE_FAINT);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.inactive.bg_fill = SURFACE_RAISED;
    widgets.inactive.weak_bg_fill = SURFACE_RAISED;
    widgets.inactive.bg_stroke = Stroke::new(1.0, GOLD_DIM);
    widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.hovered.bg_fill = Color32::from_rgb(60, 50, 31);
    widgets.hovered.weak_bg_fill = Color32::from_rgb(60, 50, 31);
    widgets.hovered.bg_stroke = Stroke::new(1.5, GOLD);
    widgets.hovered.fg_stroke = Stroke::new(1.5, TEXT_STRONG);
    widgets.active.bg_fill = Color32::from_rgb(72, 60, 37);
    widgets.active.weak_bg_fill = Color32::from_rgb(72, 60, 37);
    widgets.active.bg_stroke = Stroke::new(1.5, GOLD);
    widgets.active.fg_stroke = Stroke::new(2.0, TEXT_STRONG);
    widgets.open.bg_fill = SURFACE_RAISED;
    widgets.open.weak_bg_fill = SURFACE_RAISED;
    widgets.open.bg_stroke = Stroke::new(1.0, GOLD);
    widgets.open.fg_stroke = Stroke::new(1.0, TEXT_STRONG);
    for widget in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        widget.corner_radius = CornerRadius::same(2);
    }
    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_parse_and_the_dark_look_installs() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label(heading("Vault"));
            ui.label("body text");
        });
        output.textures_delta.clear();
        let style = ctx.style_of(egui::Theme::Dark);
        assert!(style.visuals.dark_mode);
        assert_eq!(style.visuals.panel_fill, SURFACE);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
    }

    #[test]
    fn heading_family_leads_with_cinzel_and_keeps_fallbacks() {
        let fonts = font_definitions();
        let stack = &fonts.families[&heading_family()];
        assert_eq!(stack[0], "cinzel");
        assert!(stack.len() > 1);
        assert_eq!(fonts.families[&FontFamily::Proportional][0], "alegreya");
    }
}
