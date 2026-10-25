use std::fmt;

use dioxus::prelude::*;

use super::DispatchMessage;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn from_rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub const fn from_hex(hex: u32) -> Self {
        let r = ((hex >> 16) & 0xFF) as f32 / 255.0;
        let g = ((hex >> 8) & 0xFF) as f32 / 255.0;
        let b = (hex & 0xFF) as f32 / 255.0;
        Self { r, g, b, a: 1.0 }
    }

    pub fn parse(hex: &str) -> Option<Self> {
        let trimmed = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
        if trimmed.len() == 6 {
            let val = u32::from_str_radix(trimmed, 16).ok()?;
            Some(Self::from_hex(val))
        } else {
            None
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = (self.r.clamp(0.0, 1.0) * 255.0).round() as u8;
        let g = (self.g.clamp(0.0, 1.0) * 255.0).round() as u8;
        let b = (self.b.clamp(0.0, 1.0) * 255.0).round() as u8;
        if (self.a - 1.0).abs() < f32::EPSILON {
            write!(f, "rgb({r}, {g}, {b})")
        } else {
            write!(f, "rgba({r}, {g}, {b}, {:.3})", self.a.clamp(0.0, 1.0))
        }
    }
}

const fn shade(color: Color, amount: f32) -> Color {
    Color {
        r: color.r * amount,
        g: color.g * amount,
        b: color.b * amount,
        a: color.a,
    }
}

/// Compile-time application palette constants (Material 3 dark surfaces).
const PRIMARY: Color = Color::from_hex(0x00309A);
const SECONDARY: Color = Color::from_hex(0x8A8FAB);
const TERTIARY: Color = Color::from_hex(0xB083A7);
#[allow(dead_code)]
const ERROR: Color = Color::from_hex(0xFF5449);
const NEUTRAL: Color = Color::from_hex(0x919094);
const NEUTRAL_VARIANT: Color = Color::from_hex(0x90909A);
const CUSTOM1: Color = Color::from_hex(0x166EA6);

const ACCENT_TEXT: Color = Color::from_rgb(0.48, 0.72, 0.98);
const BRIGHT_TEXT: Color = Color::from_rgb(0.90, 0.91, 0.94);
const MUTED: Color = shade(NEUTRAL_VARIANT, 0.78);
const BACKGROUND: Color = shade(NEUTRAL, 0.15);
const ON_PRIMARY: Color = Color::from_rgb(0.92, 0.95, 1.0);
const POSITIVE: Color = CUSTOM1;
const SURFACE: Color = shade(NEUTRAL, 0.09);
const SURFACE_ELEVATED: Color = shade(NEUTRAL, 0.12);
const OUTLINE: Color = shade(NEUTRAL_VARIANT, 0.28);
const USER_BUBBLE: Color = shade(PRIMARY, 0.48);

#[allow(dead_code)]
pub(super) const fn primary() -> Color {
    PRIMARY
}
pub(super) const fn accent_text() -> Color {
    ACCENT_TEXT
}
pub(super) const fn bright_text() -> Color {
    BRIGHT_TEXT
}
pub(super) const fn muted() -> Color {
    MUTED
}
#[allow(dead_code)]
pub(super) const fn background() -> Color {
    BACKGROUND
}
pub(super) const fn on_primary() -> Color {
    ON_PRIMARY
}
#[allow(dead_code)]
pub(super) const fn positive() -> Color {
    POSITIVE
}
pub(super) fn parse_hex_color(hex: &str) -> Color {
    Color::parse(hex).unwrap_or(POSITIVE)
}

pub(super) const APP_CSS: &str = r#"
* {
    box-sizing: border-box;
    user-select: none;
    -webkit-user-select: none;
}
html, body, #main {
    margin: 0;
    padding: 0;
    width: 100%;
    height: 100%;
    overflow: hidden;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Ubuntu, Cantarell, "Noto Sans", sans-serif;
    background-color: rgb(22, 22, 22);
    color: rgb(145, 144, 148);
}
.selectable-text, .selectable-text * {
    user-select: text;
    -webkit-user-select: text;
}
button {
    font-family: inherit;
    cursor: pointer;
    border-radius: 6px;
    border: 1px solid rgb(40, 40, 43);
    background-color: rgb(18, 18, 20);
    color: rgb(230, 232, 240);
    transition: filter 0.1s ease, background-color 0.1s ease;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    text-align: left;
}
button:hover:not(:disabled) {
    filter: brightness(1.18);
}
button:disabled {
    opacity: 0.45;
    cursor: default;
}
button.btn-primary {
    background-color: rgb(0, 48, 154);
    border-color: rgb(122, 184, 250);
    color: rgb(235, 242, 255);
}
button.btn-secondary {
    background-color: rgb(24, 25, 29);
    border-color: rgb(40, 40, 43);
    color: rgb(230, 232, 240);
}
button.btn-text {
    background-color: transparent;
    border-color: transparent;
    color: rgb(230, 232, 240);
}
button.btn-text:hover:not(:disabled) {
    background-color: rgba(255, 255, 255, 0.05);
}
input[type="text"], input[type="password"] {
    font-family: inherit;
    background-color: rgb(13, 13, 13);
    color: rgb(230, 232, 240);
    border: 1px solid rgb(40, 40, 43);
    border-radius: 6px;
    outline: none;
    user-select: text;
    -webkit-user-select: text;
}
input[type="text"]:focus, input[type="password"]:focus {
    border-color: rgb(122, 184, 250);
}
input[type="checkbox"] {
    accent-color: rgb(0, 48, 154);
    cursor: pointer;
    margin: 0;
}
hr {
    border: none;
    border-top: 1px solid rgb(40, 40, 43);
    margin: 0;
    width: 100%;
}
::-webkit-scrollbar {
    width: 6px;
    height: 6px;
}
::-webkit-scrollbar-track {
    background: transparent;
}
::-webkit-scrollbar-thumb {
    background: rgba(144, 144, 154, 0.28);
    border-radius: 3px;
}
::-webkit-scrollbar-thumb:hover {
    background: rgba(122, 184, 250, 0.55);
}
"#;

pub(super) const BUTTON_MAX_WIDTH: f32 = 240.0;
pub(super) const BUTTON_MAX_HEIGHT: f32 = 36.0;

pub(super) fn constrain_button(btn: Element) -> Element {
    rsx! {
        div {
            style: "max-width: {BUTTON_MAX_WIDTH}px; max-height: {BUTTON_MAX_HEIGHT}px; overflow: hidden; display: inline-flex; flex-shrink: 0;",
            {btn}
        }
    }
}

pub(super) fn enabled_button<M: DispatchMessage>(
    label: impl fmt::Display,
    message: Option<M>,
) -> Element {
    let text_str = label.to_string();
    let is_disabled = message.is_none();
    constrain_button(rsx! {
        button {
            class: "btn-primary",
            style: "font-size: 12px; padding: 6px 11px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;",
            disabled: is_disabled,
            onclick: move |_| {
                if let Some(msg) = message.clone() {
                    msg.dispatch();
                }
            },
            "{text_str}"
        }
    })
}

pub(super) fn small_button<M: DispatchMessage>(
    label: impl fmt::Display,
    message: Option<M>,
) -> Element {
    let text_str = label.to_string();
    let is_disabled = message.is_none();
    constrain_button(rsx! {
        button {
            class: "btn-secondary",
            style: "font-size: 11px; padding: 4px 9px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;",
            disabled: is_disabled,
            onclick: move |_| {
                if let Some(msg) = message.clone() {
                    msg.dispatch();
                }
            },
            "{text_str}"
        }
    })
}

pub(super) fn primary_button<M: DispatchMessage>(
    label: impl fmt::Display,
    message: Option<M>,
) -> Element {
    let text_str = label.to_string();
    let is_disabled = message.is_none();
    constrain_button(rsx! {
        button {
            class: "btn-primary",
            style: "font-size: 12px; padding: 7px 14px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;",
            disabled: is_disabled,
            onclick: move |_| {
                if let Some(msg) = message.clone() {
                    msg.dispatch();
                }
            },
            "{text_str}"
        }
    })
}

pub(super) fn prompt_button<M: DispatchMessage>(label: &str, on_press: M) -> Element {
    let label_text = label.to_owned();
    let accent = accent_text();
    rsx! {
        button {
            class: "btn-secondary",
            style: "width: 100%; padding: 8px 12px; display: flex; flex-direction: row; align-items: center; justify-content: space-between; gap: 8px;",
            onclick: move |_| on_press.clone().dispatch(),
            span { style: "font-size: 12px; flex: 1; text-align: left;", "{label_text}" }
            span { style: "font-size: 11px; color: {accent};", "Use" }
        }
    }
}

pub(super) fn collapsible_section_label<M: DispatchMessage>(
    label: &str,
    collapsed: bool,
    on_toggle: M,
) -> Element {
    let label_text = label.to_owned();
    let toggle_word = if collapsed { "Show" } else { "Hide" };
    let muted_color = muted();
    rsx! {
        button {
            class: "btn-text",
            style: "width: 100%; padding: 4px 2px; display: flex; flex-direction: row; align-items: center; justify-content: space-between;",
            onclick: move |_| on_toggle.clone().dispatch(),
            span { style: "font-size: 10px; color: {muted_color}; flex: 1; text-align: left; font-weight: 600; letter-spacing: 0.04em;", "{label_text}" }
            span { style: "font-size: 10px; color: {muted_color};", "{toggle_word}" }
        }
    }
}

pub(super) fn card(content: Element) -> Element {
    let style = card_style();
    rsx! {
        div {
            style: "width: 100%; padding: 12px; {style}",
            {content}
        }
    }
}

pub(super) fn label_badge(name: &str, hex_color: &str, detail: Option<String>) -> Element {
    let base = parse_hex_color(hex_color);
    let bg = Color {
        r: base.r * 0.35,
        g: base.g * 0.35,
        b: base.b * 0.35,
        a: 0.95,
    };
    let label_text = match detail {
        Some(extra) => format!("{name} ({extra})"),
        None => name.to_owned(),
    };
    let bright = bright_text();
    rsx! {
        span {
            style: "display: inline-flex; align-items: center; padding: 2px 7px; font-size: 10px; color: {bright}; background-color: {bg}; border: 1px solid {base}; border-radius: 8px; white-space: nowrap;",
            "{label_text}"
        }
    }
}

/// Orientation of the divider line drawn by [`splitter`].
#[derive(Debug, Clone, Copy)]
pub(super) enum SplitAxis {
    /// A vertical line between two side-by-side panes (drag left/right).
    Vertical,
    /// A horizontal line between two stacked panes (drag up/down).
    Horizontal,
}

/// Thickness of the grabbable area around a pane boundary.
const SPLITTER_HIT_SIZE: f32 = 7.0;

/// A draggable pane boundary. Emits `on_press` when the user grabs it; the
/// drag itself is tracked by the application-level mouse listener.
pub(super) fn splitter<M: DispatchMessage>(axis: SplitAxis, active: bool, on_press: M) -> Element {
    let line_color = if active { ACCENT_TEXT } else { OUTLINE };
    let thickness = if active { 2.0 } else { 1.0 };
    match axis {
        SplitAxis::Vertical => rsx! {
            div {
                style: "width: {SPLITTER_HIT_SIZE}px; height: 100%; display: flex; align-items: stretch; justify-content: center; cursor: col-resize; flex-shrink: 0;",
                onmousedown: move |evt| {
                    evt.prevent_default();
                    on_press.clone().dispatch();
                },
                div {
                    style: "width: {thickness}px; height: 100%; background-color: {line_color};",
                }
            }
        },
        SplitAxis::Horizontal => rsx! {
            div {
                style: "width: 100%; height: {SPLITTER_HIT_SIZE}px; display: flex; flex-direction: column; align-items: stretch; justify-content: center; cursor: row-resize; flex-shrink: 0;",
                onmousedown: move |evt| {
                    evt.prevent_default();
                    on_press.clone().dispatch();
                },
                div {
                    style: "width: 100%; height: {thickness}px; background-color: {line_color};",
                }
            }
        },
    }
}

fn panel(background: Color, border: Color, radius: f32) -> String {
    format!(
        "background-color: {background}; border: 1px solid {border}; border-radius: {radius}px;"
    )
}

pub(super) fn page_style() -> String {
    format!("background-color: {BACKGROUND};")
}
pub(super) fn sidebar_style() -> String {
    let bg = shade(NEUTRAL, 0.13);
    format!("background-color: {bg}; border-right: 1px solid {OUTLINE};")
}
pub(super) fn chat_pane_style() -> String {
    format!("background-color: {SURFACE_ELEVATED}; border-left: 1px solid {OUTLINE};")
}
pub(super) fn toolbar_style() -> String {
    format!("background-color: {SURFACE}; border: 1px solid {OUTLINE}; border-radius: 6px;")
}
pub(super) fn selection_bar_style() -> String {
    panel(
        Color {
            r: 0.07,
            g: 0.15,
            b: 0.29,
            a: 0.95,
        },
        ACCENT_TEXT,
        6.0,
    )
}
pub(super) fn table_header_style() -> String {
    format!("background-color: {SURFACE_ELEVATED}; border-bottom: 1px solid {OUTLINE};")
}
pub(super) fn mail_row_style(selected: bool, checked: bool) -> String {
    if selected {
        panel(
            Color {
                r: 0.08,
                g: 0.20,
                b: 0.42,
                a: 1.0,
            },
            ACCENT_TEXT,
            4.0,
        )
    } else if checked {
        panel(
            Color {
                r: 0.09,
                g: 0.15,
                b: 0.26,
                a: 0.95,
            },
            shade(ACCENT_TEXT, 0.55),
            2.0,
        )
    } else {
        panel(SURFACE, OUTLINE, 0.0)
    }
}
pub(super) fn reader_header_style() -> String {
    panel(SURFACE_ELEVATED, OUTLINE, 8.0)
}
pub(super) fn reader_body_style() -> String {
    panel(SURFACE, OUTLINE, 8.0)
}
pub(super) fn code_block_style() -> String {
    panel(
        Color {
            r: 0.04,
            g: 0.05,
            b: 0.07,
            a: 0.98,
        },
        OUTLINE,
        6.0,
    )
}
pub(super) fn blockquote_style() -> String {
    panel(
        Color {
            r: 0.07,
            g: 0.09,
            b: 0.13,
            a: 0.92,
        },
        shade(ACCENT_TEXT, 0.55),
        6.0,
    )
}
pub(super) fn md_table_header_style() -> String {
    panel(SURFACE_ELEVATED, OUTLINE, 0.0)
}
pub(super) fn md_table_row_style() -> String {
    panel(SURFACE, OUTLINE, 0.0)
}
fn card_style() -> String {
    panel(SURFACE, OUTLINE, 10.0)
}
pub(super) fn status_style() -> String {
    panel(SURFACE, OUTLINE, 16.0)
}
pub(super) fn badge_style() -> String {
    let c = shade(TERTIARY, 0.22);
    panel(c, c, 10.0)
}
pub(super) fn source_style() -> String {
    panel(SURFACE, OUTLINE, 10.0)
}
pub(super) fn accent_tile_style() -> String {
    panel(PRIMARY, PRIMARY, 9.0)
}
pub(super) fn user_avatar_style() -> String {
    panel(USER_BUBBLE, USER_BUBBLE, 14.0)
}
pub(super) fn assistant_avatar_style() -> String {
    format!(
        "{} color: {PRIMARY};",
        panel(shade(SECONDARY, 0.22), OUTLINE, 14.0)
    )
}
pub(super) fn user_bubble_style() -> String {
    panel(USER_BUBBLE, USER_BUBBLE, 12.0)
}
pub(super) fn assistant_bubble_style() -> String {
    panel(SURFACE, OUTLINE, 12.0)
}
pub(super) fn composer_style() -> String {
    let shadow_color = shade(NEUTRAL, 0.015);
    format!(
        "{} box-shadow: 0px 4px 14px {shadow_color};",
        panel(SURFACE, shade(NEUTRAL_VARIANT, 0.38), 12.0)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_dimension_constraints_are_enforced() {
        assert_eq!(BUTTON_MAX_WIDTH, 240.0);
        assert_eq!(BUTTON_MAX_HEIGHT, 36.0);

        fn test_comp() -> Element {
            let small: Element = small_button("Test", Some(()));
            let primary: Element = primary_button("Test", Some(()));
            let enabled: Element = enabled_button("Test", Some(()));
            let custom: Element = constrain_button(rsx! { button { "Custom" } });
            rsx! {
                {small}
                {primary}
                {enabled}
                {custom}
            }
        }
        let mut vdom = VirtualDom::new(test_comp);
        vdom.rebuild_in_place();
    }

    #[test]
    fn full_app_view_renders_in_virtual_dom() {
        fn app_comp() -> Element {
            let app = crate::app::PstRagApp::new();
            app.view()
        }
        let mut vdom = VirtualDom::new(app_comp);
        vdom.rebuild_in_place();
    }
}
