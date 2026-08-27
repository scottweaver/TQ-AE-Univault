//! The gilded frame: a thin hand-drawn double gold line with
//! chamfered corners and hatched corner brackets, from its own
//! bundled art. Painted nine-patch style — corners at native scale,
//! edges stretched between them — and nothing else: the interior is
//! left untouched, so whatever lies under the frame shows through.

use eframe::egui::{self, Color32, Rect, TextureHandle, pos2, vec2};

const ART: &[u8] = include_bytes!("../../assets/components/gilded-border.png");

/// Corner slice, in source pixels — covers the bracket ornament and
/// the chamfer transition back into the straight line (~30 px).
const CORNER: f32 = 40.0;

/// Edge band thickness, in source pixels; the hand-drawn line
/// wanders across roughly 2–6 px of it.
const BAND: f32 = 8.0;

/// Content inset that keeps widgets clear of the frame line.
pub const MARGIN: egui::Margin = egui::Margin::same(14);

/// The uploaded frame art. The handle is an `Arc`, so cloning is
/// cheap.
#[derive(Clone)]
pub struct GildedBorder {
    texture: TextureHandle,
}

impl GildedBorder {
    /// Decodes and uploads the bundled art. Call once per app.
    ///
    /// # Panics
    ///
    /// If the compiled-in art fails to decode — a build defect.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // texture dims are small u32s
    pub fn load(ctx: &egui::Context) -> Self {
        let decoded = image::load_from_memory_with_format(ART, image::ImageFormat::Png)
            .expect("bundled gilded-border.png is a valid PNG")
            .into_rgba8();
        let (width, height) = decoded.dimensions();
        let pixels = egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            decoded.as_raw(),
        );
        Self {
            texture: ctx.load_texture(
                "component:gilded-border",
                pixels,
                egui::TextureOptions::LINEAR,
            ),
        }
    }

    /// Lays `content` out inside [`MARGIN`], then paints the frame
    /// over the allocated rect. The background is whatever was
    /// already there.
    pub fn show<R>(
        &self,
        ui: &mut egui::Ui,
        content: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<R> {
        let inner = egui::Frame::new().inner_margin(MARGIN).show(ui, content);
        self.paint(ui.painter(), inner.response.rect);
        inner
    }

    /// Paints the frame along the edges of `rect`; corners keep the
    /// art's native size, edges stretch. Rects narrower than two
    /// corners drop the edge pieces rather than fold them over.
    pub fn paint(&self, painter: &egui::Painter, rect: Rect) {
        let art = self.texture.size_vec2();
        let corner = vec2(CORNER, CORNER);
        for (dest, src_x, src_y) in [
            (rect.min, 0.0, 0.0),
            (pos2(rect.max.x - CORNER, rect.min.y), art.x - CORNER, 0.0),
            (pos2(rect.min.x, rect.max.y - CORNER), 0.0, art.y - CORNER),
            (rect.max - corner, art.x - CORNER, art.y - CORNER),
        ] {
            painter.image(
                self.texture.id(),
                Rect::from_min_size(dest, corner),
                self.uv(src_x, src_y, CORNER, CORNER),
                Color32::WHITE,
            );
        }
        let h_span = rect.max.x - rect.min.x - 2.0 * CORNER;
        if h_span > 0.0 {
            for (dest_y, src_y) in [(rect.min.y, 0.0), (rect.max.y - BAND, art.y - BAND)] {
                painter.image(
                    self.texture.id(),
                    Rect::from_min_size(pos2(rect.min.x + CORNER, dest_y), vec2(h_span, BAND)),
                    self.uv(CORNER, src_y, art.x - 2.0 * CORNER, BAND),
                    Color32::WHITE,
                );
            }
        }
        let v_span = rect.max.y - rect.min.y - 2.0 * CORNER;
        if v_span > 0.0 {
            for (dest_x, src_x) in [(rect.min.x, 0.0), (rect.max.x - BAND, art.x - BAND)] {
                painter.image(
                    self.texture.id(),
                    Rect::from_min_size(pos2(dest_x, rect.min.y + CORNER), vec2(BAND, v_span)),
                    self.uv(src_x, CORNER, BAND, art.y - 2.0 * CORNER),
                    Color32::WHITE,
                );
            }
        }
    }

    fn uv(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        let size = self.texture.size_vec2();
        Rect::from_min_max(
            pos2(x / size.x, y / size.y),
            pos2((x + w) / size.x, (y + h) / size.y),
        )
    }
}
