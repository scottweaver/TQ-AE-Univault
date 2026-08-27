//! In-preview review overlay: drag rectangles, ellipses, and arrows
//! over a component, type a note per shape, and export the
//! annotated frame as PNG + JSON into `review/` at the repo root —
//! the PNG for human eyes, the JSON (window- and component-space
//! coordinates per shape) for an agent to act on. Preview-harness
//! only; the app proper doesn't wire it in.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2,
};

const EXPORT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../review");

const INK: Color32 = Color32::from_rgb(255, 64, 48);
const NOTE_INK: Color32 = Color32::from_rgb(255, 235, 230);
const NOTE_BG: Color32 = Color32::from_rgba_premultiplied(40, 8, 4, 230);
const STROKE_W: f32 = 2.0;
const BADGE_R: f32 = 9.0;

/// Drags smaller than this are discarded as slips.
const MIN_DRAG: f32 = 6.0;

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Rect,
    Ellipse,
    Arrow,
}

/// A committed shape, in window points.
#[derive(Clone, Copy)]
enum Geom {
    Rect(Rect),
    Ellipse(Rect),
    Arrow { from: Pos2, to: Pos2 },
}

impl Geom {
    fn from_drag(tool: Tool, from: Pos2, to: Pos2) -> Self {
        match tool {
            Tool::Rect => Self::Rect(Rect::from_two_pos(from, to)),
            Tool::Ellipse => Self::Ellipse(Rect::from_two_pos(from, to)),
            Tool::Arrow => Self::Arrow { from, to },
        }
    }

    /// Where the badge and note hang off the shape.
    fn anchor(self) -> Pos2 {
        match self {
            Self::Rect(rect) | Self::Ellipse(rect) => rect.left_top(),
            Self::Arrow { from, .. } => from,
        }
    }
}

enum Draft {
    Dragging {
        from: Pos2,
        to: Pos2,
    },
    Noting {
        geom: Geom,
        note: String,
        fresh: bool,
    },
}

struct Annotation {
    geom: Geom,
    note: String,
}

/// The overlay's whole state; the harness owns one and calls
/// [`Self::toolbar`] in its control row and [`Self::overlay`] over
/// the component canvas while review mode is on.
#[derive(Default)]
pub struct ReviewOverlay {
    tool: Option<Tool>,
    annotations: Vec<Annotation>,
    draft: Option<Draft>,
    awaiting_screenshot: bool,
    status: Option<String>,
}

impl ReviewOverlay {
    /// Tool selector, undo/clear, and the export button.
    pub fn toolbar(&mut self, ui: &mut egui::Ui) {
        let tool = self.tool.get_or_insert(Tool::Rect);
        ui.selectable_value(tool, Tool::Rect, "rect");
        ui.selectable_value(tool, Tool::Ellipse, "ellipse");
        ui.selectable_value(tool, Tool::Arrow, "arrow");
        ui.separator();
        if ui.button("undo").clicked() {
            self.annotations.pop();
        }
        if ui.button("clear").clicked() {
            self.annotations.clear();
            self.draft = None;
        }
        if ui.button("export").clicked() && !self.annotations.is_empty() {
            self.awaiting_screenshot = true;
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if let Some(status) = &self.status {
            ui.separator();
            ui.label(status.clone());
        }
    }

    /// Input capture, shape painting, the note editor, and export
    /// completion. Call after the component is drawn; `component` is
    /// the rect the component occupies, the JSON's second coordinate
    /// space.
    pub fn overlay(
        &mut self,
        ui: &mut egui::Ui,
        canvas: Rect,
        component: Rect,
        component_name: &str,
    ) {
        let response = ui.interact(
            canvas,
            ui.id().with("review-overlay"),
            Sense::click_and_drag(),
        );
        let drag_pos = response.interact_pointer_pos();
        let tool = *self.tool.get_or_insert(Tool::Rect);
        if response.drag_started()
            && let Some(pos) = drag_pos
            && !matches!(self.draft, Some(Draft::Noting { .. }))
        {
            self.draft = Some(Draft::Dragging { from: pos, to: pos });
        }
        if let Some(Draft::Dragging { to, .. }) = &mut self.draft
            && let Some(pos) = drag_pos
        {
            *to = pos;
        }
        if response.drag_stopped()
            && let Some(Draft::Dragging { from, to }) = self.draft
        {
            self.draft = ((to - from).length() >= MIN_DRAG).then(|| Draft::Noting {
                geom: Geom::from_drag(tool, from, to),
                note: String::new(),
                fresh: true,
            });
        }

        let painter = ui.painter();
        for (index, annotation) in self.annotations.iter().enumerate() {
            paint_geom(painter, annotation.geom);
            paint_badge(painter, annotation.geom.anchor(), index + 1);
            paint_note(painter, annotation.geom.anchor(), &annotation.note);
        }
        match &self.draft {
            Some(Draft::Dragging { from, to }) => {
                paint_geom(painter, Geom::from_drag(tool, *from, *to));
            }
            Some(Draft::Noting { geom, .. }) => {
                paint_geom(painter, *geom);
                paint_badge(painter, geom.anchor(), self.annotations.len() + 1);
            }
            None => {}
        }

        self.note_editor(ui);
        self.finish_export(ui, component, component_name);
    }

    /// The floating text field of a just-drawn shape. Enter commits
    /// the annotation, Escape discards it.
    fn note_editor(&mut self, ui: &mut egui::Ui) {
        let Some(Draft::Noting { geom, note, fresh }) = &mut self.draft else {
            return;
        };
        let at = geom.anchor() + vec2(BADGE_R + 4.0, BADGE_R + 4.0);
        let mut committed = false;
        let mut cancelled = false;
        egui::Area::new(ui.id().with("review-note"))
            .fixed_pos(at)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(note)
                            .hint_text("what's wrong here? (Enter saves, Esc discards)")
                            .desired_width(300.0),
                    );
                    if *fresh {
                        edit.request_focus();
                        *fresh = false;
                    }
                    cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    committed = !cancelled
                        && edit.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                });
            });
        if committed && let Some(Draft::Noting { geom, note, .. }) = self.draft.take() {
            self.annotations.push(Annotation { geom, note });
        } else if cancelled {
            self.draft = None;
        }
    }

    /// Writes the pending export once the harness delivers the
    /// screenshot of the annotated frame.
    fn finish_export(&mut self, ui: &egui::Ui, component: Rect, component_name: &str) {
        if !self.awaiting_screenshot {
            return;
        }
        let shot = ui.ctx().input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = shot else {
            return;
        };
        self.awaiting_screenshot = false;
        let pixels_per_point = ui.ctx().pixels_per_point();
        self.status = Some(
            match write_export(
                &image,
                &self.annotations,
                component,
                component_name,
                pixels_per_point,
            ) {
                Ok(stem) => format!("saved review/{stem}.png+.json"),
                Err(error) => format!("export failed: {error}"),
            },
        );
    }
}

fn paint_geom(painter: &egui::Painter, geom: Geom) {
    let stroke = Stroke::new(STROKE_W, INK);
    match geom {
        Geom::Rect(rect) => {
            painter.rect_stroke(rect, 0.0, stroke, StrokeKind::Middle);
        }
        Geom::Ellipse(rect) => {
            painter.add(egui::Shape::from(egui::epaint::EllipseShape::stroke(
                rect.center(),
                rect.size() / 2.0,
                stroke,
            )));
        }
        Geom::Arrow { from, to } => painter.arrow(from, to - from, stroke),
    }
}

fn paint_badge(painter: &egui::Painter, anchor: Pos2, number: usize) {
    painter.circle_filled(anchor, BADGE_R, INK);
    painter.text(
        anchor,
        Align2::CENTER_CENTER,
        number.to_string(),
        FontId::proportional(12.0),
        Color32::WHITE,
    );
}

fn paint_note(painter: &egui::Painter, anchor: Pos2, note: &str) {
    if note.is_empty() {
        return;
    }
    let at = anchor + vec2(BADGE_R + 4.0, -BADGE_R);
    let galley = painter.layout_no_wrap(note.to_string(), FontId::proportional(13.0), NOTE_INK);
    let bg = Rect::from_min_size(at, galley.size() + vec2(8.0, 4.0));
    painter.rect_filled(bg, 3.0, NOTE_BG);
    painter.galley(at + vec2(4.0, 2.0), galley, NOTE_INK);
}

/// Writes `review-<unix-secs>.png` (the annotated frame) and its
/// `.json` twin; returns the file stem.
fn write_export(
    image: &ColorImage,
    annotations: &[Annotation],
    component: Rect,
    component_name: &str,
    pixels_per_point: f32,
) -> Result<String, String> {
    let dir = PathBuf::from(EXPORT_DIR);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let stem = format!("review-{secs}");

    let [width, height] = image.size;
    let mut bytes = Vec::with_capacity(width * height * 4);
    for pixel in &image.pixels {
        bytes.extend_from_slice(&pixel.to_array());
    }
    #[allow(clippy::cast_possible_truncation)] // screenshot dims are small
    image::RgbaImage::from_raw(width as u32, height as u32, bytes)
        .expect("pixel buffer length matches dimensions by construction")
        .save_with_format(dir.join(format!("{stem}.png")), image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;

    let annotations: Vec<serde_json::Value> = annotations
        .iter()
        .enumerate()
        .map(|(index, annotation)| annotation_json(index + 1, annotation, component))
        .collect();
    let json = serde_json::json!({
        "component": component_name,
        "exported_at_unix": secs,
        "pixels_per_point": pixels_per_point,
        "component_rect_window": rect_json(component),
        "annotations": annotations,
    });
    std::fs::write(
        dir.join(format!("{stem}.json")),
        serde_json::to_string_pretty(&json).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(stem)
}

fn rect_json(rect: Rect) -> serde_json::Value {
    serde_json::json!([rect.min.x, rect.min.y, rect.width(), rect.height()])
}

fn pos_json(pos: Pos2) -> serde_json::Value {
    serde_json::json!([pos.x, pos.y])
}

fn annotation_json(number: usize, annotation: &Annotation, component: Rect) -> serde_json::Value {
    let relative = |pos: Pos2| pos2(pos.x - component.min.x, pos.y - component.min.y);
    let mut value = match annotation.geom {
        Geom::Rect(rect) | Geom::Ellipse(rect) => serde_json::json!({
            "kind": if matches!(annotation.geom, Geom::Rect(_)) { "rect" } else { "ellipse" },
            "window": rect_json(rect),
            "component": rect_json(Rect::from_min_size(relative(rect.min), rect.size())),
        }),
        Geom::Arrow { from, to } => serde_json::json!({
            "kind": "arrow",
            "window_from": pos_json(from),
            "window_to": pos_json(to),
            "component_from": pos_json(relative(from)),
            "component_to": pos_json(relative(to)),
        }),
    };
    let object = value
        .as_object_mut()
        .expect("annotation_json builds an object");
    object.insert("n".into(), serde_json::json!(number));
    object.insert("note".into(), serde_json::json!(annotation.note));
    value
}
