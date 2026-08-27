//! Component preview harness: shows one component from
//! [`univault_gui::components`] in isolation, over a checkerboard
//! backdrop that proves transparency.
//!
//! ```sh
//! cargo run -p univault-gui --bin preview -- gilded-border
//! ```

use eframe::egui::{self, Color32, Rect, pos2, vec2};
use univault_gui::components::gilded_border::GildedBorder;

#[derive(Clone, Copy)]
enum Subject {
    GildedBorder,
}

impl Subject {
    const ALL: [(&'static str, Self); 1] = [("gilded-border", Self::GildedBorder)];

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(known, _)| *known == name)
            .map(|&(_, subject)| subject)
    }

    fn name(self) -> &'static str {
        match self {
            Self::GildedBorder => "gilded-border",
        }
    }
}

fn main() -> eframe::Result {
    let subject = std::env::args().nth(1).and_then(|name| {
        let found = Subject::from_name(&name);
        if found.is_none() {
            eprintln!("unknown component: {name}");
        }
        found
    });
    let Some(subject) = subject else {
        eprintln!("usage: cargo run -p univault-gui --bin preview -- <component>");
        eprintln!("components:");
        for (name, _) in Subject::ALL {
            eprintln!("  {name}");
        }
        std::process::exit(2);
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        &format!("univault preview — {}", subject.name()),
        options,
        Box::new(move |cc| Ok(Box::new(PreviewApp::new(cc, subject)))),
    )
}

struct PreviewApp {
    subject: Subject,
    gilded_border: GildedBorder,
    checkerboard: bool,
}

impl PreviewApp {
    fn new(cc: &eframe::CreationContext<'_>, subject: Subject) -> Self {
        Self {
            subject,
            gilded_border: GildedBorder::load(&cc.egui_ctx),
            checkerboard: true,
        }
    }
}

impl eframe::App for PreviewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::bottom("preview-controls").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(self.subject.name());
                ui.separator();
                ui.checkbox(&mut self.checkerboard, "checkerboard backdrop");
                ui.separator();
                ui.label("resize the window to test edge stretch");
            });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let canvas = ui.max_rect();
                if self.checkerboard {
                    checkerboard(ui.painter(), canvas);
                } else {
                    ui.painter()
                        .rect_filled(canvas, 0.0, Color32::from_gray(28));
                }
                match self.subject {
                    Subject::GildedBorder => {
                        let outer = canvas.shrink(24.0);
                        self.gilded_border.paint(ui.painter(), outer);
                        ui.painter().text(
                            outer.center(),
                            egui::Align2::CENTER_CENTER,
                            "content area",
                            egui::FontId::proportional(16.0),
                            Color32::from_gray(140),
                        );
                    }
                }
            });
    }
}

fn checkerboard(painter: &egui::Painter, rect: Rect) {
    const SQUARE: f32 = 16.0;
    painter.rect_filled(rect, 0.0, Color32::from_gray(52));
    let light = Color32::from_gray(72);
    let mut row = 0_u32;
    let mut y = rect.min.y;
    while y < rect.max.y {
        let mut col = row % 2;
        let mut x = rect.min.x;
        while x < rect.max.x {
            if col.is_multiple_of(2) {
                let square = Rect::from_min_size(pos2(x, y), vec2(SQUARE, SQUARE)).intersect(rect);
                painter.rect_filled(square, 0.0, light);
            }
            col += 1;
            x += SQUARE;
        }
        row += 1;
        y += SQUARE;
    }
}
