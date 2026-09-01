//! Component preview harness: shows one component from
//! [`univault_gui::components`] in isolation, over a checkerboard
//! backdrop that proves transparency.
//!
//! ```sh
//! cargo run -p univault-gui --bin preview -- gilded-border
//! ```

use eframe::egui::{self, Color32, Rect, pos2, vec2};
use univault_gui::components::gilded_border::GildedBorder;
use univault_gui::components::tabbed_panel::{self, TabbedPanel};
use univault_gui::review::ReviewOverlay;

#[derive(Clone, Copy)]
enum Subject {
    GildedBorder,
    TabbedPanel,
}

impl Subject {
    const ALL: [(&'static str, Self); 2] = [
        ("gilded-border", Self::GildedBorder),
        ("tabbed-panel", Self::TabbedPanel),
    ];

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(known, _)| *known == name)
            .map(|&(_, subject)| subject)
    }

    fn name(self) -> &'static str {
        match self {
            Self::GildedBorder => "gilded-border",
            Self::TabbedPanel => "tabbed-panel",
        }
    }
}

fn main() -> eframe::Result {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let review_mode = args.iter().any(|arg| arg == "--review");
    args.retain(|arg| arg != "--review");
    let size = args
        .iter()
        .position(|arg| arg == "--size")
        .map(|at| args.drain(at..(at + 2).min(args.len())).nth(1));
    let size = match size {
        None => [900.0, 700.0],
        Some(spec) => spec.as_deref().and_then(parse_size).unwrap_or_else(|| {
            eprintln!("--size expects WxH, e.g. --size 520x700");
            std::process::exit(2);
        }),
    };
    let subject = args.first().and_then(|name| {
        let found = Subject::from_name(name);
        if found.is_none() {
            eprintln!("unknown component: {name}");
        }
        found
    });
    let Some(subject) = subject else {
        eprintln!(
            "usage: cargo run -p univault-gui --bin preview -- <component> [--review] [--size WxH]"
        );
        eprintln!("components:");
        for (name, _) in Subject::ALL {
            eprintln!("  {name}");
        }
        std::process::exit(2);
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(size),
        ..Default::default()
    };
    eframe::run_native(
        &format!("univault preview — {}", subject.name()),
        options,
        Box::new(move |cc| {
            let mut app = PreviewApp::new(cc, subject);
            app.review_mode = review_mode;
            Ok(Box::new(app))
        }),
    )
}

struct PreviewApp {
    subject: Subject,
    gilded_border: GildedBorder,
    tabbed_panel: TabbedPanel,
    selected_tab: usize,
    checkerboard: bool,
    review_mode: bool,
    review: ReviewOverlay,
}

impl PreviewApp {
    fn new(cc: &eframe::CreationContext<'_>, subject: Subject) -> Self {
        Self {
            subject,
            gilded_border: GildedBorder::load(&cc.egui_ctx),
            tabbed_panel: TabbedPanel::load(&cc.egui_ctx),
            selected_tab: 0,
            checkerboard: true,
            review_mode: false,
            review: ReviewOverlay::default(),
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
                ui.checkbox(&mut self.review_mode, "review");
                if self.review_mode {
                    self.review.toolbar(ui);
                } else {
                    ui.separator();
                    ui.label("resize the window to test edge stretch");
                }
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
                    Subject::TabbedPanel => {
                        let region = canvas.shrink(24.0);
                        let titles = [
                            "Sword (152)",
                            "Axe (98)",
                            "Mace (74)",
                            "Spear (61)",
                            "Bow (88)",
                            "Thrown (37)",
                            "Staff (120)",
                            "Shield (143)",
                        ];
                        let tabs: Vec<tabbed_panel::Tab> = titles
                            .iter()
                            .map(|title| tabbed_panel::Tab::new(*title))
                            .collect();
                        let selected = self.selected_tab;
                        let response =
                            ui.scope_builder(egui::UiBuilder::new().max_rect(region), |ui| {
                                self.tabbed_panel.show(ui, &tabs, selected, |ui| {
                                    ui.label(format!("content of \"{}\"", titles[selected]));
                                    ui.set_min_size(ui.available_size());
                                })
                            });
                        if let Some(index) = response.inner.clicked {
                            self.selected_tab = index;
                        }
                    }
                }
                if self.review_mode {
                    self.review
                        .overlay(ui, canvas, canvas.shrink(24.0), self.subject.name());
                }
            });
    }
}

fn parse_size(spec: &str) -> Option<[f32; 2]> {
    let (w, h) = spec.split_once('x')?;
    Some([w.parse().ok()?, h.parse().ok()?])
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
