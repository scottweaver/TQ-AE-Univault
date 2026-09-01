//! The bronze tabbed panel: leather tab plates over a mitered rail
//! frame with a black interior, from its own bundled art. The rail
//! is a nine-patch (corners native, strips stretched); tab plates
//! are 3-sliced so any label width fits. The active plate's slice
//! carries wings into the rail band, reproducing the art's gold
//! curl where the open tab merges through the rail.
//!
//! When the strip outgrows the pane it scrolls instead of clipping
//! tabs away: triangular chevrons appear at either end and slide
//! the plates while the pointer rests on them — position-based, so
//! an item dragged onto a chevron scrolls too and can reach an
//! off-screen tab. The scroll offset lives in egui temp memory
//! under the panel's id; selecting a tab scrolls it into view.

use eframe::egui::{self, Color32, CursorIcon, Rect, Sense, TextureHandle, pos2, vec2};

const ART: &[u8] = include_bytes!("../../assets/components/tabs.png");

/// Tab plate height above the rail, in source pixels.
const TAB_H: f32 = 35.0;

/// Rail band thicknesses, in source pixels. The art's own right and
/// bottom rails are thinner afterthoughts (user-confirmed), so the
/// frame is symmetric by construction: the right rail is the left
/// mirrored, the bottom rail is the top flipped.
const RAIL_TOP: f32 = 13.0;
const RAIL_LEFT: f32 = 10.0;
const RAIL_RIGHT: f32 = RAIL_LEFT;
const RAIL_BOTTOM: f32 = RAIL_TOP;

/// Mitered rail corner slice, in source pixels. All four corners
/// derive from the art's top-left corner — the only one whose two
/// rails both carry the full outer treatment — mirrored/flipped
/// into place.
const CORNER: f32 = 24.0;

/// The inactive plate (the art's right tab — the only one with both
/// caps clean) and the active plate (the art's middle tab, sliced
/// through the rail with wings so the merge curls come along; the
/// wings stop 2 px short of the neighboring plates' antialiased
/// edges, which LINEAR sampling would otherwise bleed in).
const INACTIVE: Src = Src::new(458.0, 0.0, 218.0, 35.0);
const INACTIVE_CAPS: f32 = 14.0;
const ACTIVE: Src = Src::new(218.0, 0.0, 238.0, 48.0);
const ACTIVE_CAPS: f32 = 26.0;
const ACTIVE_WING: f32 = 10.0;

/// Hover zone reserved at each end of a scrolling strip, the
/// triangle drawn inside it, and how fast a hovered chevron slides
/// the plates.
const CHEVRON_ZONE: f32 = 22.0;
const CHEVRON_W: f32 = 9.0;
const CHEVRON_H: f32 = 14.0;
const SCROLL_SPEED: f32 = 280.0;

/// Clean rail strips (the top one sampled right of the last tab,
/// clear of any plate merge) and the master corner.
const TOP_STRIP: Src = Src::new(680.0, 35.0, 60.0, 13.0);
const LEFT_STRIP: Src = Src::new(0.0, 100.0, 10.0, 500.0);
const CORNER_TL: Src = Src::new(0.0, 35.0, 24.0, 24.0);

/// The art's top-left corner carries a bright nub of the flush
/// first tab's rim (cols 0–2, rows 35–36) that reads as a stray
/// vertical line once our tabs are inset; the top-right corner's
/// clean outer-edge turn, oriented per corner, papers over it.
const CORNER_PATCH: Src = Src::new(744.0, 35.0, 5.0, 5.0);

/// The panel interior — the art's flat fill.
const INTERIOR: Color32 = Color32::BLACK;

const LABEL_INACTIVE: Color32 = Color32::from_rgb(186, 166, 118);
const LABEL_ACTIVE: Color32 = Color32::from_rgb(236, 218, 164);
const LABEL_DISABLED: Color32 = Color32::from_gray(115);

const TAB_GAP: f32 = 12.0;
const TAB_PAD: f32 = 18.0;
const TAB_MIN_W: f32 = 44.0;

/// Content inset: past the tab strip and the rails, with breathing
/// room inside the interior.
pub const MARGIN: egui::Margin = egui::Margin {
    left: 18,
    right: 18,
    top: 56,
    bottom: 21,
};

/// How a slice is oriented when blitted.
#[derive(Clone, Copy)]
enum Flip {
    None,
    H,
    V,
    Hv,
}

/// Which end of the strip a chevron scrolls toward.
#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

/// A pixel rectangle inside the source art.
#[derive(Clone, Copy)]
struct Src {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Src {
    const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
}

/// One plate on the strip. A disabled plate renders dim, reports
/// neither clicks nor hover, and offers `disabled_hint` as its
/// tooltip.
pub struct Tab {
    title: String,
    enabled: bool,
    disabled_hint: Option<String>,
}

impl Tab {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            enabled: true,
            disabled_hint: None,
        }
    }

    pub fn disabled(title: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            enabled: false,
            disabled_hint: Some(hint.into()),
        }
    }
}

/// What [`TabbedPanel::show`] reports back: the clicked tab, the
/// enabled tab under the pointer (for callers that switch tabs
/// mid-drag) — the caller owns the selection — and the content's
/// result.
pub struct TabbedPanelResponse<R> {
    pub clicked: Option<usize>,
    pub hovered: Option<usize>,
    pub inner: R,
}

/// The uploaded panel art. The handle is an `Arc`, so cloning is
/// cheap.
#[derive(Clone)]
pub struct TabbedPanel {
    texture: TextureHandle,
}

impl TabbedPanel {
    /// Decodes and uploads the bundled art. Call once per app.
    ///
    /// # Panics
    ///
    /// If the compiled-in art fails to decode — a build defect.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // texture dims are small u32s
    pub fn load(ctx: &egui::Context) -> Self {
        let decoded = image::load_from_memory_with_format(ART, image::ImageFormat::Png)
            .expect("bundled tabs.png is a valid PNG")
            .into_rgba8();
        let (width, height) = decoded.dimensions();
        let pixels = egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            decoded.as_raw(),
        );
        Self {
            texture: ctx.load_texture(
                "component:tabbed-panel",
                pixels,
                egui::TextureOptions::LINEAR,
            ),
        }
    }

    /// Lays `content` out inside [`MARGIN`], then dresses the
    /// allocated rect: black interior, rail frame, one plate per
    /// tab with `selected` drawn open through the rail. A strip
    /// wider than the pane scrolls behind end chevrons rather than
    /// clipping tabs out of reach. Reports clicks and hover on
    /// enabled plates; the caller applies the selection change.
    pub fn show<R>(
        &self,
        ui: &mut egui::Ui,
        tabs: &[Tab],
        selected: usize,
        content: impl FnOnce(&mut egui::Ui) -> R,
    ) -> TabbedPanelResponse<R> {
        let fill_slot = ui.painter().add(egui::Shape::Noop);
        let inner = egui::Frame::new().inner_margin(MARGIN).show(ui, content);
        let outer = inner.response.rect;
        let frame = Rect::from_min_max(pos2(outer.min.x, outer.min.y + TAB_H), outer.max);
        ui.painter()
            .set(fill_slot, egui::Shape::rect_filled(frame, 0.0, INTERIOR));

        let geo = strip_geometry(ui, tabs, outer);
        let pointer = ui.ctx().pointer_latest_pos();
        let zones = geo.scrolling.then_some((geo.left_zone, geo.right_zone));
        let offset = strip_offset(
            ui,
            inner.response.id,
            selected,
            &geo.plates,
            zones,
            geo.span,
            geo.max_offset,
        );

        let visible = geo.visible;
        let clip = if geo.scrolling { visible } else { outer };
        let strip = ui.painter().with_clip_rect(clip.intersect(ui.clip_rect()));
        let tab_rects: Vec<Rect> = geo
            .plates
            .rects
            .iter()
            .map(|rect| rect.translate(vec2(geo.strip_left - offset, outer.min.y)))
            .collect();
        for (index, rect) in tab_rects.iter().enumerate() {
            if index != selected {
                self.three_slice(&strip, INACTIVE, INACTIVE_CAPS, *rect);
            }
        }
        self.rail_frame(ui.painter(), frame);
        let mut clicked = None;
        let mut hovered = None;
        for ((index, rect), tab) in tab_rects.iter().enumerate().zip(tabs) {
            if index == selected {
                let open = Rect::from_min_max(
                    pos2(rect.min.x - ACTIVE_WING, rect.min.y),
                    pos2(rect.max.x + ACTIVE_WING, rect.max.y + RAIL_TOP),
                );
                self.three_slice(&strip, ACTIVE, ACTIVE_CAPS, open);
            }
            let ink = if !tab.enabled {
                LABEL_DISABLED
            } else if index == selected {
                LABEL_ACTIVE
            } else {
                LABEL_INACTIVE
            };
            strip.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                &tab.title,
                egui::TextStyle::Button.resolve(ui.style()),
                ink,
            );
            let hit = rect.intersect(visible);
            if hit.width() <= 0.0 {
                continue;
            }
            let response = ui.interact(
                hit,
                inner.response.id.with(("tabbed-panel", index)),
                Sense::click(),
            );
            if tab.enabled {
                let response = response.on_hover_cursor(CursorIcon::PointingHand);
                if response.clicked() {
                    clicked = Some(index);
                }
                if pointer.is_some_and(|pos| hit.contains(pos)) {
                    hovered = Some(index);
                }
            } else if let Some(hint) = &tab.disabled_hint {
                response.on_hover_text(hint.clone());
            }
        }
        if geo.scrolling {
            let over = |zone: Rect| pointer.is_some_and(|pos| zone.contains(pos));
            if offset > 0.5 {
                chevron(ui, geo.left_zone, Side::Left, over(geo.left_zone));
            }
            if offset < geo.max_offset - 0.5 {
                chevron(ui, geo.right_zone, Side::Right, over(geo.right_zone));
            }
        }
        TabbedPanelResponse {
            clicked,
            hovered,
            inner: inner.inner,
        }
    }

    /// The mitered rail frame around `rect`: the master corner
    /// oriented into each corner (native scale), strips stretched
    /// between them.
    fn rail_frame(&self, painter: &egui::Painter, rect: Rect) {
        let corner = vec2(CORNER, CORNER);
        let patch = vec2(CORNER_PATCH.w, CORNER_PATCH.h);
        let far = rect.max - patch;
        for (flip, patch_flip, at, patch_at) in [
            (Flip::None, Flip::H, rect.min, rect.min),
            (
                Flip::H,
                Flip::None,
                pos2(rect.max.x - CORNER, rect.min.y),
                pos2(far.x, rect.min.y),
            ),
            (
                Flip::V,
                Flip::Hv,
                pos2(rect.min.x, rect.max.y - CORNER),
                pos2(rect.min.x, far.y),
            ),
            (Flip::Hv, Flip::V, rect.max - corner, far),
        ] {
            self.blit_flipped(painter, CORNER_TL, Rect::from_min_size(at, corner), flip);
            self.blit_flipped(
                painter,
                CORNER_PATCH,
                Rect::from_min_size(patch_at, patch),
                patch_flip,
            );
        }
        let h_span = rect.width() - 2.0 * CORNER;
        if h_span > 0.0 {
            for (flip, y) in [
                (Flip::None, rect.min.y),
                (Flip::V, rect.max.y - RAIL_BOTTOM),
            ] {
                self.blit_flipped(
                    painter,
                    TOP_STRIP,
                    Rect::from_min_size(pos2(rect.min.x + CORNER, y), vec2(h_span, RAIL_TOP)),
                    flip,
                );
            }
        }
        let v_span = rect.height() - 2.0 * CORNER;
        if v_span > 0.0 {
            for (flip, x) in [(Flip::None, rect.min.x), (Flip::H, rect.max.x - RAIL_RIGHT)] {
                self.blit_flipped(
                    painter,
                    LEFT_STRIP,
                    Rect::from_min_size(pos2(x, rect.min.y + CORNER), vec2(RAIL_LEFT, v_span)),
                    flip,
                );
            }
        }
    }

    /// Caps-preserving horizontal 3-slice of `src` onto `dest`.
    fn three_slice(&self, painter: &egui::Painter, src: Src, caps: f32, dest: Rect) {
        let scale = dest.height() / src.h;
        let cap_w = caps * scale;
        let pieces = [
            (
                Src::new(src.x, src.y, caps, src.h),
                Rect::from_min_size(dest.min, vec2(cap_w, dest.height())),
            ),
            (
                Src::new(src.x + caps, src.y, src.w - 2.0 * caps, src.h),
                Rect::from_min_max(
                    pos2(dest.min.x + cap_w, dest.min.y),
                    pos2(dest.max.x - cap_w, dest.max.y),
                ),
            ),
            (
                Src::new(src.x + src.w - caps, src.y, caps, src.h),
                Rect::from_min_size(
                    pos2(dest.max.x - cap_w, dest.min.y),
                    vec2(cap_w, dest.height()),
                ),
            ),
        ];
        for (piece, rect) in pieces {
            if rect.width() > 0.0 {
                self.blit(painter, piece, rect);
            }
        }
    }

    fn blit(&self, painter: &egui::Painter, src: Src, dest: Rect) {
        self.blit_flipped(painter, src, dest, Flip::None);
    }

    fn blit_flipped(&self, painter: &egui::Painter, src: Src, dest: Rect, flip: Flip) {
        let size = self.texture.size_vec2();
        let (u0, u1) = (src.x / size.x, (src.x + src.w) / size.x);
        let (v0, v1) = ((src.y / size.y), (src.y + src.h) / size.y);
        let (u0, u1) = match flip {
            Flip::None | Flip::V => (u0, u1),
            Flip::H | Flip::Hv => (u1, u0),
        };
        let (v0, v1) = match flip {
            Flip::None | Flip::H => (v0, v1),
            Flip::V | Flip::Hv => (v1, v0),
        };
        let uv = Rect {
            min: pos2(u0, v0),
            max: pos2(u1, v1),
        };
        painter.image(self.texture.id(), dest, uv, Color32::WHITE);
    }
}

/// The strip's plate rects, relative to its own left edge, and the
/// total width they occupy.
struct PlateLayout {
    rects: Vec<Rect>,
    total_width: f32,
}

/// One plate rect per tab, laid left-to-right in a single row, each
/// sized to its label. Positions are relative — the caller places
/// the strip past the corner block (inside a corner, the rail's
/// inner gold line stops short of the edge, and an open plate's
/// wing crossing that quiet zone reads as a stray horizontal line)
/// and applies the scroll offset.
fn plate_layout(ui: &egui::Ui, tabs: &[Tab]) -> PlateLayout {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let mut x = 0.0;
    let rects: Vec<Rect> = tabs
        .iter()
        .map(|tab| {
            let label =
                ui.painter()
                    .layout_no_wrap(tab.title.clone(), font.clone(), Color32::WHITE);
            let width = (label.rect.width() + 2.0 * TAB_PAD).max(TAB_MIN_W);
            let rect = Rect::from_min_size(pos2(x, 0.0), vec2(width, TAB_H));
            x += width + TAB_GAP;
            rect
        })
        .collect();
    PlateLayout {
        rects,
        total_width: (x - TAB_GAP).max(0.0),
    }
}

/// The dressed strip's working measurements: the plates, whether
/// they outgrow the pane, the visible span between the corner
/// blocks (shrunk behind chevron zones when they do), where that
/// span starts, and the chevron hover zones at either end.
struct StripGeometry {
    plates: PlateLayout,
    scrolling: bool,
    span: f32,
    max_offset: f32,
    strip_left: f32,
    visible: Rect,
    left_zone: Rect,
    right_zone: Rect,
}

fn strip_geometry(ui: &egui::Ui, tabs: &[Tab], outer: Rect) -> StripGeometry {
    let plates = plate_layout(ui, tabs);
    let full_span = outer.width() - 2.0 * CORNER;
    let scrolling = plates.total_width > full_span;
    let reserve = if scrolling { CHEVRON_ZONE } else { 0.0 };
    let span = (full_span - 2.0 * reserve).max(TAB_MIN_W);
    let max_offset = (plates.total_width - span).max(0.0);
    let band =
        |min_x: f32| Rect::from_min_size(pos2(min_x, outer.min.y), vec2(CHEVRON_ZONE, TAB_H));
    let strip_left = outer.min.x + CORNER + reserve;
    StripGeometry {
        scrolling,
        span,
        max_offset,
        strip_left,
        visible: Rect::from_min_max(
            pos2(strip_left, outer.min.y),
            pos2(strip_left + span, outer.max.y),
        ),
        left_zone: band(outer.min.x + CORNER),
        right_zone: band(outer.max.x - CORNER - CHEVRON_ZONE),
        plates,
    }
}

/// The strip's scroll offset for this frame: persisted in temp
/// memory under the panel's id, scrolled to reveal a newly selected
/// plate, nudged while the pointer rests on a chevron zone (passed
/// only when the strip overflows), and clamped to the scrollable
/// range.
fn strip_offset(
    ui: &egui::Ui,
    base: egui::Id,
    selected: usize,
    plates: &PlateLayout,
    zones: Option<(Rect, Rect)>,
    span: f32,
    max_offset: f32,
) -> f32 {
    let state_id = base.with("tab-strip-scroll");
    let (stored, last_selected) = ui
        .ctx()
        .data(|data| data.get_temp::<(f32, usize)>(state_id))
        .unwrap_or((0.0, selected));
    let mut offset = stored;
    if selected != last_selected
        && let Some(plate) = plates.rects.get(selected)
    {
        offset = reveal_offset(offset, plate.min.x, plate.max.x, span);
    }
    if let Some((left_zone, right_zone)) = zones {
        let pointer = ui.ctx().pointer_latest_pos();
        let step = SCROLL_SPEED * ui.input(|input| input.stable_dt).min(0.1);
        let over = |zone: Rect| pointer.is_some_and(|pos| zone.contains(pos));
        if over(left_zone) && offset > 0.0 {
            offset -= step;
            ui.ctx().request_repaint();
        }
        if over(right_zone) && offset < max_offset {
            offset += step;
            ui.ctx().request_repaint();
        }
    }
    let offset = offset.clamp(0.0, max_offset);
    ui.ctx()
        .data_mut(|data| data.insert_temp(state_id, (offset, selected)));
    offset
}

/// One scroll chevron: a triangle pointing off-strip, lit while the
/// pointer rests on its zone. Scrolling keys on pointer position
/// rather than widget hover so a drag in progress scrolls too.
fn chevron(ui: &egui::Ui, zone: Rect, side: Side, lit: bool) {
    let center = zone.center();
    let (near, far) = match side {
        Side::Left => (center.x + CHEVRON_W / 2.0, center.x - CHEVRON_W / 2.0),
        Side::Right => (center.x - CHEVRON_W / 2.0, center.x + CHEVRON_W / 2.0),
    };
    let ink = if lit { LABEL_ACTIVE } else { LABEL_INACTIVE };
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            pos2(near, center.y - CHEVRON_H / 2.0),
            pos2(far, center.y),
            pos2(near, center.y + CHEVRON_H / 2.0),
        ],
        ink,
        egui::Stroke::NONE,
    ));
}

/// The smallest scroll adjustment that brings a newly selected
/// plate — spanning `min..max` in strip coordinates — fully into a
/// window `span` wide starting at `offset`.
fn reveal_offset(offset: f32, min: f32, max: f32, span: f32) -> f32 {
    if min < offset {
        min
    } else if max > offset + span {
        max - span
    } else {
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::reveal_offset;

    #[test]
    #[allow(clippy::float_cmp)] // reveal_offset returns its inputs unchanged — no arithmetic drift
    fn reveal_scrolls_only_far_enough_to_show_the_plate() {
        // Already visible: untouched.
        assert_eq!(reveal_offset(100.0, 120.0, 200.0, 300.0), 100.0);
        // Off the left edge: scroll back to its leading edge.
        assert_eq!(reveal_offset(150.0, 120.0, 200.0, 300.0), 120.0);
        // Off the right edge: scroll just enough to fit it.
        assert_eq!(reveal_offset(0.0, 500.0, 620.0, 300.0), 320.0);
        // Exactly at the edges counts as visible.
        assert_eq!(reveal_offset(120.0, 120.0, 420.0, 300.0), 120.0);
    }
}
