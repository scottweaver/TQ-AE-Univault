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

/// Rail band thicknesses, in source pixels. The art is asymmetric —
/// its left rail is thicker than its right.
const RAIL_TOP: f32 = 13.0;
const RAIL_LEFT: f32 = 10.0;
const RAIL_RIGHT: f32 = 6.0;
const RAIL_BOTTOM: f32 = 8.0;

/// Mitered rail corner slice, in source pixels.
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
/// clear of any plate merge) and the corner origins.
const TOP_STRIP: Src = Src::new(680.0, 35.0, 60.0, 13.0);
const LEFT_STRIP: Src = Src::new(0.0, 100.0, 10.0, 500.0);
const RIGHT_STRIP: Src = Src::new(743.0, 100.0, 6.0, 500.0);
const BOTTOM_STRIP: Src = Src::new(100.0, 737.0, 500.0, 8.0);
const CORNER_TL: Src = Src::new(0.0, 35.0, 24.0, 24.0);
const CORNER_TR: Src = Src::new(725.0, 35.0, 24.0, 24.0);
const CORNER_BL: Src = Src::new(0.0, 721.0, 24.0, 24.0);
const CORNER_BR: Src = Src::new(725.0, 721.0, 24.0, 24.0);

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
    right: 14,
    top: 56,
    bottom: 16,
};

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

    /// The mitered rail frame around `rect`: corners native, strips
    /// stretched between them.
    fn rail_frame(&self, painter: &egui::Painter, rect: Rect) {
        for (src, corner) in [
            (CORNER_TL, rect.min),
            (CORNER_TR, pos2(rect.max.x - CORNER, rect.min.y)),
            (CORNER_BL, pos2(rect.min.x, rect.max.y - CORNER)),
            (CORNER_BR, pos2(rect.max.x - CORNER, rect.max.y - CORNER)),
        ] {
            self.blit(
                painter,
                src,
                Rect::from_min_size(corner, vec2(CORNER, CORNER)),
            );
        }
        let h_span = rect.width() - 2.0 * CORNER;
        if h_span > 0.0 {
            self.blit(
                painter,
                TOP_STRIP,
                Rect::from_min_size(
                    pos2(rect.min.x + CORNER, rect.min.y),
                    vec2(h_span, RAIL_TOP),
                ),
            );
            self.blit(
                painter,
                BOTTOM_STRIP,
                Rect::from_min_size(
                    pos2(rect.min.x + CORNER, rect.max.y - RAIL_BOTTOM),
                    vec2(h_span, RAIL_BOTTOM),
                ),
            );
        }
        let v_span = rect.height() - 2.0 * CORNER;
        if v_span > 0.0 {
            self.blit(
                painter,
                LEFT_STRIP,
                Rect::from_min_size(
                    pos2(rect.min.x, rect.min.y + CORNER),
                    vec2(RAIL_LEFT, v_span),
                ),
            );
            self.blit(
                painter,
                RIGHT_STRIP,
                Rect::from_min_size(
                    pos2(rect.max.x - RAIL_RIGHT, rect.min.y + CORNER),
                    vec2(RAIL_RIGHT, v_span),
                ),
            );
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
        let size = self.texture.size_vec2();
        let uv = Rect::from_min_max(
            pos2(src.x / size.x, src.y / size.y),
            pos2((src.x + src.w) / size.x, (src.y + src.h) / size.y),
        );
        painter.image(self.texture.id(), dest, uv, Color32::WHITE);
    }
}

/// One plate rect per title, laid left-to-right along the strip
/// above the rail, each sized to its label. The strip starts one
/// wing in from the frame edge so an open plate's curls always land
/// on the rail, never past it.
fn tab_rects<'t>(ui: &egui::Ui, titles: &[&'t str], outer: Rect) -> Vec<(Rect, &'t str)> {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let mut x = outer.min.x + ACTIVE_WING;
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
