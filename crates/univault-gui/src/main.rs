//! egui/eframe front-end for tq-univault.

use eframe::egui;

fn main() -> eframe::Result {
    eframe::run_native(
        "TQ UniVault",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(App))),
    )
}

struct App;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading("TQ UniVault");
    }
}
