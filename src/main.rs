#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
#[allow(dead_code, unused_imports)]
mod components;
mod db;
mod i18n;
mod laya;
mod llm;
mod model;
mod pst;
#[cfg(test)]
mod tests;
mod text;

fn main() {
    app::run();
}
