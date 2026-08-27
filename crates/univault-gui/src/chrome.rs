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

/// The under-tabs leather strip of the caravan window (leather
/// body, dark line, gold trim, black gap, gold rim — the reference
/// "textured tab border"), tiled as the border of a tab's panel.
const LEATHER_STRIP: Src = Src::new(150.0, 108.0, 240.0, 15.0);

/// On-screen thickness of the leather panel border — the strip's
/// native rows.
pub const LEATHER_H: f32 = 15.0;

/// A clean strip of the character screen's light flagstone — the
/// reference's exact surface; a mirrored collage of it becomes the
/// stretched backdrop.
const LIGHT_BLOCK: Src = Src::new(417.0, 424.0, 39.0, 196.0);

/// Mirror repetitions assembling the collage from [`LIGHT_BLOCK`].
const LIGHT_COLS: usize = 8;
const LIGHT_ROWS: usize = 2;

/// The engraved slot plates of the character screen, one per worn
/// piece.
const PLATE_HELM: Src = Src::new(161.0, 16.0, 74.0, 76.0);
const PLATE_TORSO: Src = Src::new(13.0, 272.0, 74.0, 107.0);
const PLATE_LEGS: Src = Src::new(310.0, 308.0, 73.0, 71.0);
const PLATE_ARMS: Src = Src::new(310.0, 225.0, 73.0, 74.0);
const PLATE_AMULET: Src = Src::new(327.0, 394.0, 39.0, 40.0);
const PLATE_RING: Src = Src::new(29.0, 394.0, 39.0, 40.0);
const PLATE_ARTIFACT: Src = Src::new(30.0, 205.0, 39.0, 50.0);
const PLATE_WEAPON_LEFT: Src = Src::new(13.0, 65.0, 73.0, 128.0);
const PLATE_WEAPON_RIGHT: Src = Src::new(310.0, 65.0, 73.0, 127.0);

/// Which engraved plate backs a doll slot.
#[derive(Clone, Copy)]
pub enum SlotPlate {
    Helm,
    Torso,
    Legs,
    Arms,
    Amulet,
    Ring,
    Artifact,
    WeaponLeft,
    WeaponRight,
}

impl SlotPlate {
    const fn src(self) -> Src {
        match self {
            Self::Helm => PLATE_HELM,
            Self::Torso => PLATE_TORSO,
            Self::Legs => PLATE_LEGS,
            Self::Arms => PLATE_ARMS,
            Self::Amulet => PLATE_AMULET,
            Self::Ring => PLATE_RING,
            Self::Artifact => PLATE_ARTIFACT,
            Self::WeaponLeft => PLATE_WEAPON_LEFT,
            Self::WeaponRight => PLATE_WEAPON_RIGHT,
        }
    }
}

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

    /// Texture coordinates with optional mirroring, for reusing a
    /// piece on the opposite edge of the frame.
    fn uv_flipped(self, texture: &TextureHandle, flip_h: bool, flip_v: bool) -> Rect {
        let plain = self.uv(texture);
        let (u0, u1) = if flip_h {
            (plain.max.x, plain.min.x)
        } else {
            (plain.min.x, plain.max.x)
        };
        let (v0, v1) = if flip_v {
            (plain.max.y, plain.min.y)
        } else {
            (plain.min.y, plain.max.y)
        };
        Rect {
            min: pos2(u0, v0),
            max: pos2(u1, v1),
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
    /// The 2×2 mirrored light-flagstone collage — the stretched
    /// backdrop.
    light_stone: TextureHandle,
    /// The under-tabs leather strip, and its transpose for the
    /// vertical edges (rim facing the content).
    leather_h: TextureHandle,
    leather_v: TextureHandle,
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
        let light = light_stone(cache)?;
        let (strip, strip_t) = leather_strips(&cache.chrome("caravan/caravanwindow01.tex")?)?;
        Some(Self {
            caravan: load("caravan/caravanwindow01.tex")?,
            title: load("caravan/caravantitle01.tex")?,
            tab: load("caravan/storageareatab01.tex")?,
            character: load("characterscreen/characterwindow01.tex")?,
            light_stone: ctx.load_texture(
                "chrome:light-stone",
                light,
                egui::TextureOptions::LINEAR,
            ),
            leather_h: ctx.load_texture("chrome:leather-h", strip, egui::TextureOptions::LINEAR),
            leather_v: ctx.load_texture("chrome:leather-v", strip_t, egui::TextureOptions::LINEAR),
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

    /// The leather panel border — the reference's textured tab
    /// border on all four sides, gold rim facing the content. Sides
    /// first, so the horizontal strips own the corners.
    pub fn leather_frame(&self, painter: &egui::Painter, rect: Rect) {
        let side = Src::full(&self.leather_v);
        tile_v(
            painter,
            &self.leather_v,
            side,
            Rect::from_min_max(rect.min, pos2(rect.min.x + LEATHER_H, rect.max.y)),
        );
        let span = Rect::from_min_max(pos2(rect.max.x - LEATHER_H, rect.min.y), rect.max);
        let clipped = painter.with_clip_rect(span);
        let mut y = span.min.y;
        while y < span.max.y {
            clipped.image(
                self.leather_v.id(),
                Rect::from_min_size(pos2(span.min.x, y), vec2(LEATHER_H, side.h)),
                side.uv_flipped(&self.leather_v, true, false),
                Color32::WHITE,
            );
            y += side.h;
        }
        let strip = Src::full(&self.leather_h);
        tile_h_flipped(
            painter,
            &self.leather_h,
            strip,
            Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + LEATHER_H)),
            false,
        );
        tile_h_flipped(
            painter,
            &self.leather_h,
            strip,
            Rect::from_min_max(pos2(rect.min.x, rect.max.y - LEATHER_H), rect.max),
            true,
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
        // The active tab rises above the row and descends to the
        // strip's baseline, anchoring onto the panel like the game's
        // caravan tabs.
        let plate = if selected {
            Rect::from_min_max(
                pos2(rect.min.x, rect.min.y - 2.0),
                pos2(rect.max.x, rect.max.y + 4.0),
            )
        } else {
            rect
        };
        three_slice(ui.painter(), &self.tab, TAB_CAPS, plate, tint);
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

    /// The window backdrop: light flagstone, stretched over the rect.
    pub fn backdrop(&self, painter: &egui::Painter, rect: Rect, tint: Color32) {
        let full = Src::full(&self.light_stone);
        blit(painter, &self.light_stone, full, rect, tint);
    }

    /// One engraved slot plate, stretched into a doll slot box.
    pub fn slot_plate(&self, painter: &egui::Painter, plate: SlotPlate, rect: Rect) {
        blit(painter, &self.character, plate.src(), rect, Color32::WHITE);
    }
}

/// Repaints the active tab joined onto the border below it: the
/// plate is the tab art minus its bottom border, descending to
/// `bottom` so its side lines flow into the pane's border line —
/// the caravan window's anchored-tab look.
pub fn tab_anchor(chrome: &Chrome, painter: &egui::Painter, rect: Rect, text: &str, bottom: f32) {
    let plate = Rect::from_min_max(pos2(rect.min.x, rect.min.y - 2.0), pos2(rect.max.x, bottom));
    let size = chrome.tab.size_vec2();
    let body = Src::new(0.0, 0.0, size.x, size.y - 4.0);
    three_slice_src(painter, &chrome.tab, body, TAB_CAPS, plate, Color32::WHITE);
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
        Color32::from_rgb(250, 247, 238),
    );
    let pos = pos2(
        rect.center().x - galley.size().x / 2.0,
        rect.center().y - galley.size().y / 2.0,
    );
    painter.galley(pos, galley, Color32::from_rgb(250, 247, 238));
}

/// The under-tabs leather strip and its transpose (for vertical
/// edges), cropped from the caravan window. `None` only if the art
/// is smaller than the strip.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // slice coordinates are small positive integers
fn leather_strips(
    image: &univault_core::tex::RgbaImage,
) -> Option<(egui::ColorImage, egui::ColorImage)> {
    let (x0, y0, w, h) = (
        LEATHER_STRIP.x as usize,
        LEATHER_STRIP.y as usize,
        LEATHER_STRIP.w as usize,
        LEATHER_STRIP.h as usize,
    );
    if image.width < x0 + w || image.height < y0 + h {
        return None;
    }
    let mut strip = vec![0_u8; w * h * 4];
    let mut transposed = vec![0_u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let from = ((y0 + y) * image.width + x0 + x) * 4;
            let to = (y * w + x) * 4;
            strip[to..to + 4].copy_from_slice(&image.pixels[from..from + 4]);
            let to_t = (x * h + y) * 4;
            transposed[to_t..to_t + 4].copy_from_slice(&image.pixels[from..from + 4]);
        }
    }
    Some((
        egui::ColorImage::from_rgba_unmultiplied([w, h], &strip),
        egui::ColorImage::from_rgba_unmultiplied([h, w], &transposed),
    ))
}

/// A soft inner shadow the frame casts onto the content — the
/// reference screens shade their borders for depth.
pub fn inner_shadow(painter: &egui::Painter, rect: Rect, depth: f32) {
    let dark = Color32::from_black_alpha(90);
    let clear = Color32::TRANSPARENT;
    let mut mesh = egui::Mesh::default();
    let mut quad = |corners: [(egui::Pos2, Color32); 4]| {
        let base = u32::try_from(mesh.vertices.len()).unwrap_or(0);
        for (pos, color) in corners {
            mesh.vertices.push(egui::epaint::Vertex {
                pos,
                uv: egui::epaint::WHITE_UV,
                color,
            });
        }
        mesh.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    };
    quad([
        (rect.left_top(), dark),
        (rect.right_top(), dark),
        (pos2(rect.max.x, rect.min.y + depth), clear),
        (pos2(rect.min.x, rect.min.y + depth), clear),
    ]);
    quad([
        (pos2(rect.min.x, rect.max.y - depth), clear),
        (pos2(rect.max.x, rect.max.y - depth), clear),
        (rect.right_bottom(), dark),
        (rect.left_bottom(), dark),
    ]);
    quad([
        (rect.left_top(), dark),
        (pos2(rect.min.x + depth, rect.min.y), clear),
        (pos2(rect.min.x + depth, rect.max.y), clear),
        (rect.left_bottom(), dark),
    ]);
    quad([
        (pos2(rect.max.x - depth, rect.min.y), clear),
        (rect.right_top(), dark),
        (rect.right_bottom(), dark),
        (pos2(rect.max.x - depth, rect.max.y), clear),
    ]);
    painter.add(egui::Shape::mesh(mesh));
}

fn blit(painter: &egui::Painter, texture: &TextureHandle, src: Src, dest: Rect, tint: Color32) {
    painter.image(texture.id(), dest, src.uv(texture), tint);
}

/// A 2×2 mirrored collage of [`LIGHT_BLOCK`], so the stretched
/// backdrop has no hard collage seams. `None` only if the art is
/// smaller than the block.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // slice coordinates are small positive integers
fn light_stone(cache: &GameCache) -> Option<egui::ColorImage> {
    let image = &cache.chrome("characterscreen/characterwindow01.tex")?;
    let (x0, y0, w, h) = (
        LIGHT_BLOCK.x as usize,
        LIGHT_BLOCK.y as usize,
        LIGHT_BLOCK.w as usize,
        LIGHT_BLOCK.h as usize,
    );
    if image.width < x0 + w || image.height < y0 + h {
        return None;
    }
    let (out_w, out_h) = (LIGHT_COLS * w, LIGHT_ROWS * h);
    let mut pixels = vec![0_u8; out_w * out_h * 4];
    for y in 0..out_h {
        for x in 0..out_w {
            let (tile_x, offset_x) = (x / w, x % w);
            let (tile_y, offset_y) = (y / h, y % h);
            let sx = if tile_x % 2 == 0 {
                offset_x
            } else {
                w - 1 - offset_x
            };
            let sy = if tile_y % 2 == 0 {
                offset_y
            } else {
                h - 1 - offset_y
            };
            let from = ((y0 + sy) * image.width + x0 + sx) * 4;
            let to = (y * out_w + x) * 4;
            pixels[to..to + 4].copy_from_slice(&image.pixels[from..from + 4]);
        }
    }
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [out_w, out_h],
        &pixels,
    ))
}

fn tile_h_flipped(
    painter: &egui::Painter,
    texture: &TextureHandle,
    src: Src,
    dest: Rect,
    flip_v: bool,
) {
    let clipped = painter.with_clip_rect(dest);
    let uv = src.uv_flipped(texture, false, flip_v);
    let mut x = dest.min.x;
    while x < dest.max.x {
        clipped.image(
            texture.id(),
            Rect::from_min_size(pos2(x, dest.min.y), vec2(src.w, dest.height())),
            uv,
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
    three_slice_src(
        painter,
        texture,
        Src::new(0.0, 0.0, size.x, size.y),
        caps,
        dest,
        tint,
    );
}

/// [`three_slice`], over a source sub-rectangle.
fn three_slice_src(
    painter: &egui::Painter,
    texture: &TextureHandle,
    src: Src,
    caps: f32,
    dest: Rect,
    tint: Color32,
) {
    let scale = dest.height() / src.h;
    let cap = caps * scale;
    blit(
        painter,
        texture,
        Src::new(src.x, src.y, caps, src.h),
        Rect::from_min_size(dest.min, vec2(cap, dest.height())),
        tint,
    );
    blit(
        painter,
        texture,
        Src::new(src.x + src.w - caps, src.y, caps, src.h),
        Rect::from_min_size(pos2(dest.max.x - cap, dest.min.y), vec2(cap, dest.height())),
        tint,
    );
    blit(
        painter,
        texture,
        Src::new(src.x + caps, src.y, src.w - 2.0 * caps, src.h),
        Rect::from_min_max(
            pos2(dest.min.x + cap, dest.min.y),
            pos2(dest.max.x - cap, dest.max.y),
        ),
        tint,
    );
}
