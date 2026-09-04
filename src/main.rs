#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod db;
mod llm;
mod model;
mod pst;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PST RAG")
            .with_inner_size([980.0, 720.0])
            .with_min_inner_size([720.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "PST RAG",
        options,
        Box::new(|cc| Ok(Box::new(app::PstRagApp::new(cc)))),
    )
}
