#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod db;
mod laya;
mod llm;
mod model;
mod pst;

fn main() -> iced::Result {
    app::run()
}
