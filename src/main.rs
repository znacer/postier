#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod db;
mod i18n;
mod laya;
mod llm;
mod model;
mod pst;
#[cfg(test)]
mod tests;

fn main() {
    app::run();
}
