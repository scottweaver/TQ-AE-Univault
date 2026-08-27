//! The game's own UI art, dressed over the panes: the caravan
//! window's gold frame and grid cells, its iron nameplate and
//! leather tabs, the options-screen button plates, the item-tooltip
//! border, and the character screen's parchment — all pulled from
//! the cache ([`univault_core::gamedata::CHROME_TEXTURES`]) and
//! sliced here. Every slice coordinate lives in this module, so
//! tuning the chrome never touches the cache format. Absent chrome
//! (an old cache, missing archives) falls back to the painted
//! [`crate::theme`] at every call site.

use eframe::egui::{self, Color32, Rect, Sense, TextureHandle, pos2, vec2};
use univault_core::cache::GameCache;

/// The caravan window's grid: one 32×32 cell with its groove on the
/// right and bottom edges, so tiles chain into the game's grid.
const CELL: Src = Src::new(27.0, 126.0, 32.0, 32.0);

/// The caravan window frame, as eight nine-patch pieces. The bottom
/// corners are wider: they carry the ornamental brackets.
// The top-right corner is the top-left mirrored: the art's own
// top-right carries the baked-in close button.
const FRAME_TL: Src = Src::new(0.0, 0.0, 14.0, 14.0);
// The tile span stops short of the art's title notch, whose bands
// brighten mid-strip.
const FRAME_TC: Src = Src::new(16.0, 0.0, 150.0, 14.0);
const FRAME_LC: Src = Src::new(0.0, 150.0, 14.0, 320.0);
const FRAME_RC: Src = Src::new(551.0, 150.0, 14.0, 320.0);
const FRAME_BL: Src = Src::new(0.0, 609.0, 29.0, 28.0);
const FRAME_BC: Src = Src::new(150.0, 609.0, 240.0, 28.0);
const FRAME_BR: Src = Src::new(536.0, 609.0, 29.0, 28.0);

/// Content inset that keeps a pane's widgets off the frame bands.
pub const FRAME_MARGIN: egui::Margin = egui::Margin {
    left: 16,
    right: 16,
    top: 16,
    bottom: 30,
};

/// The character screen's doll panel: dark-grey cracked stone — the
/// game's own backdrop for contrast areas, and the source the beige
/// window backdrop is derived from.
const DOLL_STONE: Src = Src::new(110.0, 104.0, 178.0, 322.0);

/// The inventory panel's plain olive margin — near-uniform, so it
/// tiles without a visible pattern; the framed panes' interior.
const INVENTORY_MARGIN: Src = Src::new(24.0, 494.0, 44.0, 40.0);

/// End-cap widths of the 3-sliced strips, in source pixels.
const TITLE_CAPS: f32 = 24.0;
const TAB_CAPS: f32 = 10.0;
const BUTTON_CAPS: f32 = 8.0;

const NAMEPLATE_HEIGHT: f32 = 30.0;
const TAB_HEIGHT: f32 = 26.0;
const BUTTON_HEIGHT: f32 = 26.0;

/// Ink used on the iron and gold plates — the game letters its
/// chrome dark, not cream.
const PLATE_INK: Color32 = Color32::from_rgb(24, 18, 8);

/// A pixel rectangle inside a source texture.
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

    fn uv(self, texture: &TextureHandle) -> Rect {
        let size = texture.size_vec2();
        Rect::from_min_max(
            pos2(self.x / size.x, self.y / size.y),
            pos2((self.x + self.w) / size.x, (self.y + self.h) / size.y),
        )
    }

    /// Horizontally mirrored texture coordinates — for reusing a
    /// left-side piece on the right (the art's own top-right corner
    /// carries the baked-in close button).
    fn uv_mirrored(self, texture: &TextureHandle) -> Rect {
        let size = texture.size_vec2();
        Rect {
            min: pos2((self.x + self.w) / size.x, self.y / size.y),
            max: pos2(self.x / size.x, (self.y + self.h) / size.y),
        }
    }

    fn full(texture: &TextureHandle) -> Self {
        let size = texture.size_vec2();
        Self::new(0.0, 0.0, size.x, size.y)
    }
}

/// The uploaded chrome textures. Handles are `Arc`s, so cloning the
/// set is cheap — call sites take a clone to keep [`crate::Caches`]
/// borrowable.
#[derive(Clone)]
pub struct Chrome {
    caravan: TextureHandle,
    title: TextureHandle,
    tab: TextureHandle,
    character: TextureHandle,
    /// The inventory panel art; absent in caches imported before it
    /// joined the manifest.
    inventory: Option<TextureHandle>,
    /// The doll stone re-lit to yellow/beige at upload — the window
    /// backdrop.
    beige: TextureHandle,
    button_up: TextureHandle,
    button_over: TextureHandle,
    button_down: TextureHandle,
    tooltip: Option<[TextureHandle; 8]>,
}

impl Chrome {
    /// Uploads the chrome set from the cache; `None` when the cache
    /// predates chrome or lacks the caravan/character/button art.
    pub fn load(ctx: &egui::Context, cache: &GameCache) -> Option<Self> {
        let load = |key: &str| -> Option<TextureHandle> {
            let image = cache.chrome(key)?;
            let pixels = egui::ColorImage::from_rgba_unmultiplied(
                [image.width, image.height],
                &image.pixels,
            );
            Some(ctx.load_texture(
                format!("chrome:{key}"),
                pixels,
                egui::TextureOptions::LINEAR,
            ))
        };
        let tooltip_keys = [
            "borderitemtl01.tex",
            "borderitemtc01.tex",
            "borderitemtr01.tex",
            "borderitemcl01.tex",
            "borderitemcr01.tex",
            "borderitembl01.tex",
            "borderitembc01.tex",
            "borderitembr01.tex",
        ];
        let tooltip: Option<Vec<TextureHandle>> =
            tooltip_keys.iter().map(|key| load(key)).collect();
        let beige = beige_stone(&cache.chrome("characterscreen/characterwindow01.tex")?)?;
        Some(Self {
            caravan: load("caravan/caravanwindow01.tex")?,
            title: load("caravan/caravantitle01.tex")?,
            tab: load("caravan/storageareatab01.tex")?,
            character: load("characterscreen/characterwindow01.tex")?,
            inventory: load("inventorytab01.tex"),
            beige: ctx.load_texture("chrome:beige-stone", beige, egui::TextureOptions::LINEAR),
            button_up: load("optionswindow/buttonup01.tex")?,
            button_over: load("optionswindow/buttonover01.tex")?,
            button_down: load("optionswindow/buttondown01.tex")?,
            tooltip: tooltip.and_then(|handles| handles.try_into().ok()),
        })
    }

    /// One grid cell's art.
    pub fn grid_cell(&self, painter: &egui::Painter, rect: Rect) {
        blit(painter, &self.caravan, CELL, rect, Color32::WHITE);
    }

    /// The caravan window frame around a pane; draw after the
    /// content — the pieces only cover the [`FRAME_MARGIN`] bands.
    pub fn pane_frame(&self, painter: &egui::Painter, rect: Rect) {
        let texture = &self.caravan;
        let tl = vec2(FRAME_TL.w, FRAME_TL.h);
        let tr = tl;
        let bl = vec2(FRAME_BL.w, FRAME_BL.h);
        let br = vec2(FRAME_BR.w, FRAME_BR.h);
        blit(
            painter,
            texture,
            FRAME_TL,
            Rect::from_min_size(rect.min, tl),
            Color32::WHITE,
        );
        painter.image(
            texture.id(),
            Rect::from_min_size(pos2(rect.max.x - tr.x, rect.min.y), tr),
            FRAME_TL.uv_mirrored(texture),
            Color32::WHITE,
        );
        blit(
            painter,
            texture,
            FRAME_BL,
            Rect::from_min_size(pos2(rect.min.x, rect.max.y - bl.y), bl),
            Color32::WHITE,
        );
        blit(
            painter,
            texture,
            FRAME_BR,
            Rect::from_min_size(pos2(rect.max.x - br.x, rect.max.y - br.y), br),
            Color32::WHITE,
        );
        tile_h(
            painter,
            texture,
            FRAME_TC,
            Rect::from_min_max(
                pos2(rect.min.x + tl.x, rect.min.y),
                pos2(rect.max.x - tr.x, rect.min.y + FRAME_TC.h),
            ),
        );
        tile_h(
            painter,
            texture,
            FRAME_BC,
            Rect::from_min_max(
                pos2(rect.min.x + bl.x, rect.max.y - FRAME_BC.h),
                pos2(rect.max.x - br.x, rect.max.y),
            ),
        );
        tile_v(
            painter,
            texture,
            FRAME_LC,
            Rect::from_min_max(
                pos2(rect.min.x, rect.min.y + tl.y),
                pos2(rect.min.x + FRAME_LC.w, rect.max.y - bl.y),
            ),
        );
        tile_v(
            painter,
            texture,
            FRAME_RC,
            Rect::from_min_max(
                pos2(rect.max.x - FRAME_RC.w, rect.min.y + tr.y),
                pos2(rect.max.x, rect.max.y - br.y),
            ),
        );
    }

    /// The iron nameplate with a title on it.
    pub fn nameplate(&self, ui: &mut egui::Ui, text: &str) {
        let galley = ui.painter().layout_no_wrap(
            text.to_owned(),
            egui::FontId::new(15.0, egui::FontFamily::Name("tq-heading".into())),
            PLATE_INK,
        );
        let size = vec2(galley.size().x + 56.0, NAMEPLATE_HEIGHT);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        three_slice(ui.painter(), &self.title, TITLE_CAPS, rect, Color32::WHITE);
        let pos = pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(pos, galley, PLATE_INK);
    }

    /// A leather tab plate; dim until selected, like the game's
    /// caravan tabs — white lettering when active, dull white-grey
    /// otherwise.
    pub fn tab(
        &self,
        ui: &mut egui::Ui,
        selected: bool,
        enabled: bool,
        text: &str,
    ) -> egui::Response {
        let ink = if !enabled {
            Color32::from_gray(115)
        } else if selected {
            Color32::from_rgb(250, 247, 238)
        } else {
            Color32::from_rgb(188, 182, 168)
        };
        let galley = ui.painter().layout_no_wrap(
            text.to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            ink,
        );
        let size = vec2(galley.size().x + 26.0, TAB_HEIGHT);
        let (rect, response) = ui.allocate_exact_size(
            size,
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let tint = if !enabled {
            Color32::from_gray(90)
        } else if selected {
            Color32::WHITE
        } else if response.hovered() {
            Color32::from_gray(210)
        } else {
            Color32::from_gray(160)
        };
        three_slice(ui.painter(), &self.tab, TAB_CAPS, rect, tint);
        let pos = pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(pos, galley, ink);
        response
    }

    /// A gold plate button in the game's three states.
    pub fn button(&self, ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
        let ink = if enabled {
            PLATE_INK
        } else {
            Color32::from_rgb(96, 84, 60)
        };
        let galley = ui.painter().layout_no_wrap(
            text.to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            ink,
        );
        let size = vec2(galley.size().x + 24.0, BUTTON_HEIGHT);
        let (rect, response) = ui.allocate_exact_size(
            size,
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let (texture, tint) = if !enabled {
            (&self.button_up, Color32::from_gray(135))
        } else if response.is_pointer_button_down_on() {
            (&self.button_down, Color32::WHITE)
        } else if response.hovered() {
            (&self.button_over, Color32::WHITE)
        } else {
            (&self.button_up, Color32::WHITE)
        };
        three_slice(ui.painter(), texture, BUTTON_CAPS, rect, tint);
        let pos = pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(pos, galley, ink);
        response
    }

    /// The game's thin gold item-tooltip border, when the art is
    /// present.
    pub fn tooltip_frame(&self, painter: &egui::Painter, rect: Rect) -> bool {
        let Some([tl, tc, tr, cl, cr, bl, bc, br]) = self.tooltip.as_ref() else {
            return false;
        };
        let band = 4.0;
        let corner = |texture: &TextureHandle, pos: egui::Pos2| {
            blit(
                painter,
                texture,
                Src::full(texture),
                Rect::from_min_size(pos, vec2(band, band)),
                Color32::WHITE,
            );
        };
        corner(tl, rect.min);
        corner(tr, pos2(rect.max.x - band, rect.min.y));
        corner(bl, pos2(rect.min.x, rect.max.y - band));
        corner(br, pos2(rect.max.x - band, rect.max.y - band));
        let edge = |texture: &TextureHandle, dest: Rect| {
            blit(painter, texture, Src::full(texture), dest, Color32::WHITE);
        };
        edge(
            tc,
            Rect::from_min_max(
                pos2(rect.min.x + band, rect.min.y),
                pos2(rect.max.x - band, rect.min.y + band),
            ),
        );
        edge(
            bc,
            Rect::from_min_max(
                pos2(rect.min.x + band, rect.max.y - band),
                pos2(rect.max.x - band, rect.max.y),
            ),
        );
        edge(
            cl,
            Rect::from_min_max(
                pos2(rect.min.x, rect.min.y + band),
                pos2(rect.min.x + band, rect.max.y - band),
            ),
        );
        edge(
            cr,
            Rect::from_min_max(
                pos2(rect.max.x - band, rect.min.y + band),
                pos2(rect.max.x, rect.max.y - band),
            ),
        );
        true
    }

    /// The window backdrop: the cracked stone in beige, tiled with
    /// alternating mirroring so the cracks don't read as a repeat.
    pub fn backdrop(&self, painter: &egui::Painter, rect: Rect, tint: Color32) {
        tile_mirrored(painter, &self.beige, rect, tint);
    }

    /// Dark-grey cracked stone stretched over a contrast area — the
    /// game's doll-panel backdrop.
    pub fn doll_stone(&self, painter: &egui::Painter, rect: Rect) {
        blit(painter, &self.character, DOLL_STONE, rect, Color32::WHITE);
    }

    /// A framed pane's interior: the inventory panel's plain olive,
    /// or the painted theme surface for caches predating that art.
    pub fn interior(&self, painter: &egui::Painter, rect: Rect) {
        match &self.inventory {
            Some(inventory) => tile(
                painter,
                inventory,
                INVENTORY_MARGIN,
                rect,
                Color32::from_gray(170),
            ),
            None => {
                painter.rect_filled(rect, 0.0, crate::theme::SURFACE);
            }
        }
    }
}

fn blit(painter: &egui::Painter, texture: &TextureHandle, src: Src, dest: Rect, tint: Color32) {
    painter.image(texture.id(), dest, src.uv(texture), tint);
}

/// The doll stone re-lit toward yellow/beige: a per-channel
/// multiply-and-lift chosen so the slab face lands near the game's
/// beige stone while the cracks stay dark. `None` only if the art
/// is smaller than the doll panel region.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // channel math clamps to 0..=255
fn beige_stone(image: &univault_core::tex::RgbaImage) -> Option<egui::ColorImage> {
    let (x0, y0, w, h) = (
        DOLL_STONE.x as usize,
        DOLL_STONE.y as usize,
        DOLL_STONE.w as usize,
        DOLL_STONE.h as usize,
    );
    if image.width < x0 + w || image.height < y0 + h {
        return None;
    }
    let relit = |value: u8, factor: f32, lift: f32| -> u8 {
        (f32::from(value).mul_add(factor, lift)).clamp(0.0, 255.0) as u8
    };
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let i = (y * image.width + x) * 4;
            pixels.push(relit(image.pixels[i], 2.0, 31.0));
            pixels.push(relit(image.pixels[i + 1], 2.0, 30.0));
            pixels.push(relit(image.pixels[i + 2], 1.9, 18.0));
            pixels.push(255);
        }
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels))
}

/// Tiles a whole texture with alternating horizontal/vertical
/// mirroring, so a strong feature (a crack) never lines up into a
/// visible repeat.
fn tile_mirrored(painter: &egui::Painter, texture: &TextureHandle, rect: Rect, tint: Color32) {
    let clipped = painter.with_clip_rect(rect);
    let size = texture.size_vec2();
    let src = Src::full(texture);
    let mut row = 0;
    let mut y = rect.min.y;
    while y < rect.max.y {
        let mut column = 0;
        let mut x = rect.min.x;
        while x < rect.max.x {
            let mut uv = if column % 2 == 0 {
                src.uv(texture)
            } else {
                src.uv_mirrored(texture)
            };
            if row % 2 == 1 {
                std::mem::swap(&mut uv.min.y, &mut uv.max.y);
            }
            clipped.image(
                texture.id(),
                Rect::from_min_size(pos2(x, y), size),
                uv,
                tint,
            );
            x += size.x;
            column += 1;
        }
        y += size.y;
        row += 1;
    }
}

fn tile(painter: &egui::Painter, texture: &TextureHandle, src: Src, rect: Rect, tint: Color32) {
    let clipped = painter.with_clip_rect(rect);
    let mut y = rect.min.y;
    while y < rect.max.y {
        let mut x = rect.min.x;
        while x < rect.max.x {
            blit(
                &clipped,
                texture,
                src,
                Rect::from_min_size(pos2(x, y), vec2(src.w, src.h)),
                tint,
            );
            x += src.w;
        }
        y += src.h;
    }
}

fn tile_h(painter: &egui::Painter, texture: &TextureHandle, src: Src, dest: Rect) {
    let clipped = painter.with_clip_rect(dest);
    let mut x = dest.min.x;
    while x < dest.max.x {
        blit(
            &clipped,
            texture,
            src,
            Rect::from_min_size(pos2(x, dest.min.y), vec2(src.w, dest.height())),
            Color32::WHITE,
        );
        x += src.w;
    }
}

fn tile_v(painter: &egui::Painter, texture: &TextureHandle, src: Src, dest: Rect) {
    let clipped = painter.with_clip_rect(dest);
    let mut y = dest.min.y;
    while y < dest.max.y {
        blit(
            &clipped,
            texture,
            src,
            Rect::from_min_size(pos2(dest.min.x, y), vec2(dest.width(), src.h)),
            Color32::WHITE,
        );
        y += src.h;
    }
}

/// Caps-preserving horizontal 3-slice of a whole texture strip onto
/// `dest`, scaled to `dest`'s height.
fn three_slice(
    painter: &egui::Painter,
    texture: &TextureHandle,
    caps: f32,
    dest: Rect,
    tint: Color32,
) {
    let size = texture.size_vec2();
    let scale = dest.height() / size.y;
    let cap = caps * scale;
    blit(
        painter,
        texture,
        Src::new(0.0, 0.0, caps, size.y),
        Rect::from_min_size(dest.min, vec2(cap, dest.height())),
        tint,
    );
    blit(
        painter,
        texture,
        Src::new(size.x - caps, 0.0, caps, size.y),
        Rect::from_min_size(pos2(dest.max.x - cap, dest.min.y), vec2(cap, dest.height())),
        tint,
    );
    blit(
        painter,
        texture,
        Src::new(caps, 0.0, size.x - 2.0 * caps, size.y),
        Rect::from_min_max(
            pos2(dest.min.x + cap, dest.min.y),
            pos2(dest.max.x - cap, dest.max.y),
        ),
        tint,
    );
}
