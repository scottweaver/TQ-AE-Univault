//! The bronze tabbed panel: leather tab plates over a mitered rail
//! frame with a black interior, from its own bundled art. The rail
//! is a nine-patch (corners native, strips stretched); tab plates
//! are 3-sliced so any label width fits. The active plate's slice
//! carries wings into the rail band, reproducing the art's gold
//! curl where the open tab merges through the rail.

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

const TAB_GAP: f32 = 12.0;
const TAB_PAD: f32 = 18.0;
const TAB_MIN_W: f32 = 80.0;

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

/// What [`TabbedPanel::show`] reports back: the clicked tab, if
/// any — the caller owns the selection — and the content's result.
pub struct TabbedPanelResponse<R> {
    pub clicked: Option<usize>,
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
    /// title with `selected` drawn open through the rail. Reports a
    /// click on any plate; the caller applies the selection change.
    pub fn show<R>(
        &self,
        ui: &mut egui::Ui,
        titles: &[&str],
        selected: usize,
        content: impl FnOnce(&mut egui::Ui) -> R,
    ) -> TabbedPanelResponse<R> {
        let fill_slot = ui.painter().add(egui::Shape::Noop);
        let inner = egui::Frame::new().inner_margin(MARGIN).show(ui, content);
        let outer = inner.response.rect;
        let frame = Rect::from_min_max(pos2(outer.min.x, outer.min.y + TAB_H), outer.max);
        ui.painter()
            .set(fill_slot, egui::Shape::rect_filled(frame, 0.0, INTERIOR));

        let strip = ui.painter().with_clip_rect(outer.intersect(ui.clip_rect()));
        let tab_rects = tab_rects(ui, titles, outer);
        for (index, (rect, _)) in tab_rects.iter().enumerate() {
            if index != selected {
                self.three_slice(&strip, INACTIVE, INACTIVE_CAPS, *rect);
            }
        }
        self.rail_frame(ui.painter(), frame);
        let mut clicked = None;
        for (index, (rect, title)) in tab_rects.iter().enumerate() {
            if index == selected {
                let open = Rect::from_min_max(
                    pos2(rect.min.x - ACTIVE_WING, rect.min.y),
                    pos2(rect.max.x + ACTIVE_WING, rect.max.y + RAIL_TOP),
                );
                self.three_slice(&strip, ACTIVE, ACTIVE_CAPS, open);
            }
            let ink = if index == selected {
                LABEL_ACTIVE
            } else {
                LABEL_INACTIVE
            };
            strip.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                *title,
                egui::TextStyle::Button.resolve(ui.style()),
                ink,
            );
            let response = ui
                .interact(*rect, ui.id().with(("tabbed-panel", index)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            if response.clicked() {
                clicked = Some(index);
            }
        }
        TabbedPanelResponse {
            clicked,
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

/// One plate rect per title, laid left-to-right along the strip
/// above the rail, each sized to its label. The strip starts past
/// the corner block so an open plate's wings only ever land on
/// rail-run — inside the corner, the rail's inner gold line stops
/// short of the edge, and a wing fragment crossing that quiet zone
/// reads as a stray horizontal line.
fn tab_rects<'t>(ui: &egui::Ui, titles: &[&'t str], outer: Rect) -> Vec<(Rect, &'t str)> {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let mut x = outer.min.x + CORNER;
    titles
        .iter()
        .map(|title| {
            let label =
                ui.painter()
                    .layout_no_wrap((*title).to_string(), font.clone(), Color32::WHITE);
            let width = (label.rect.width() + 2.0 * TAB_PAD).max(TAB_MIN_W);
            let rect = Rect::from_min_size(pos2(x, outer.min.y), vec2(width, TAB_H));
            x += width + TAB_GAP;
            (rect, *title)
        })
        .collect()
}
