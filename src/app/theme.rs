use std::fmt;

use dioxus::prelude::*;

use super::DispatchMessage;
use crate::components::{
    badge::{Badge, BadgeVariant},
    button::{Button, ButtonSize, ButtonVariant},
    collapsible::CollapsibleTrigger,
};

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

/// Application palette constants (modern dark web UI surfaces).
const PRIMARY: Color = Color::from_hex(0x2563EB);
const SECONDARY: Color = Color::from_hex(0x8A8FAB);
#[allow(dead_code)]
const TERTIARY: Color = Color::from_hex(0xB083A7);
#[allow(dead_code)]
const ERROR: Color = Color::from_hex(0xEF4444);
const CUSTOM1: Color = Color::from_hex(0x0284C7);

const ACCENT_TEXT: Color = Color::from_hex(0x60A5FA);
const BRIGHT_TEXT: Color = Color::from_hex(0xF1F5F9);
const MUTED: Color = Color::from_hex(0x94A3B8);
const BACKGROUND: Color = Color::from_hex(0x0B0F17);
const ON_PRIMARY: Color = Color::from_hex(0xF8FAFC);
const POSITIVE: Color = CUSTOM1;
const SURFACE: Color = Color::from_hex(0x111827);
const SURFACE_ELEVATED: Color = Color::from_hex(0x1E293B);
const OUTLINE: Color = Color::from_hex(0x1E293B);
const USER_BUBBLE: Color = Color::from_hex(0x1D4ED8);

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

pub(super) const DX_COMPONENTS_THEME_CSS: &str =
    include_str!("../../assets/dx-components-theme.css");

pub(super) const APP_CSS: &str = r#"
:root {
    --dark: initial;
    --light: ;

    /* Override dx-components-theme variables to match our slate dark palette */
    --primary-color: #0b0f17;
    --primary-color-1: #0b0f17;
    --primary-color-2: #0f1420;
    --primary-color-3: #131b2e;
    --primary-color-4: #172136;
    --primary-color-5: #1a243b;
    --primary-color-6: rgba(148, 163, 184, 0.16);
    --primary-color-7: rgba(148, 163, 184, 0.28);

    --secondary-color: #ffffff;
    --secondary-color-1: #f8fafc;
    --secondary-color-2: #3b82f6;
    --secondary-color-3: #e2e8f0;
    --secondary-color-4: #cbd5e1;
    --secondary-color-5: #94a3b8;
    --secondary-color-6: #64748b;

    --focused-border-color: #3b82f6;
    --primary-error-color: #dc2626;
    --secondary-error-color: #ef4444;
    --contrast-error-color: #fee2e2;

    --bg-app: #0b0f17;
    --bg-sidebar: #0f1420;
    --bg-surface: #131b2e;
    --bg-elevated: #1a243b;
    --bg-hover: rgba(255, 255, 255, 0.045);
    --bg-active: rgba(59, 130, 246, 0.14);
    --bg-input: #0b101d;

    --border-subtle: rgba(148, 163, 184, 0.12);
    --border-default: rgba(148, 163, 184, 0.18);
    --border-strong: rgba(148, 163, 184, 0.28);
    --border-focus: #3b82f6;

    --text-primary: #f1f5f9;
    --text-secondary: #cbd5e1;
    --text-muted: #94a3b8;
    --text-accent: #60a5fa;
    --text-on-primary: #ffffff;

    --primary: #2563eb;
    --primary-hover: #3b82f6;
    --primary-subtle: rgba(37, 99, 235, 0.18);
    --danger: #ef4444;
    --danger-subtle: rgba(239, 68, 68, 0.14);
    --success: #10b981;

    --radius-xs: 4px;
    --radius-sm: 6px;
    --radius-md: 8px;
    --radius-lg: 12px;
    --radius-full: 9999px;

    --shadow-sm: 0 1px 2px rgba(0, 0, 0, 0.35);
    --shadow-md: 0 4px 12px rgba(0, 0, 0, 0.4);
    --shadow-lg: 0 8px 24px rgba(0, 0, 0, 0.5);

    --font-sans: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Ubuntu, Cantarell, "Noto Sans", sans-serif;
    --font-mono: "JetBrains Mono", "Fira Code", ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
}

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
    font-family: var(--font-sans);
    font-size: 13px;
    line-height: 1.45;
    background-color: var(--bg-app);
    color: var(--text-secondary);
    -webkit-font-smoothing: antialiased;
}

.selectable-text, .selectable-text * {
    user-select: text;
    -webkit-user-select: text;
}

/* Segmented Control & ToggleGroup refinements */
.segmented-control {
    display: inline-flex;
    align-items: center;
    background-color: var(--bg-input);
    padding: 2px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border-subtle);
    gap: 2px;
}

/* Navigation & Tree Items */
.nav-item {
    width: 100%;
    padding: 6px 10px;
    border-radius: var(--radius-sm);
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: 12px;
    font-weight: 500;
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
}

.nav-item:hover:not(:disabled) {
    background-color: var(--bg-hover);
    color: var(--text-primary);
    border-color: transparent;
}

.nav-item.active {
    background-color: var(--bg-active);
    border-color: rgba(59, 130, 246, 0.35);
    color: var(--text-primary);
}

/* Mail Table */
.mail-table-header {
    width: 100%;
    padding: 8px 14px;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 10px;
    background-color: var(--bg-elevated);
    border-bottom: 1px solid var(--border-default);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-muted);
    flex-shrink: 0;
}

.mail-row {
    width: 100%;
    padding: 8px 14px;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 10px;
    border-bottom: 1px solid var(--border-subtle);
    border-left: 3px solid transparent;
    background-color: var(--bg-surface);
    transition: background-color 0.1s ease, border-color 0.1s ease;
    flex-shrink: 0;
}

.mail-row:hover {
    background-color: #172136;
}

.mail-row.checked {
    background-color: rgba(37, 99, 235, 0.10);
    border-left-color: rgba(96, 165, 250, 0.55);
}

.mail-row.selected {
    background-color: rgba(37, 99, 235, 0.18);
    border-left-color: var(--primary-hover);
}

/* Splitters */
.splitter-v {
    width: 7px;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: col-resize;
    flex-shrink: 0;
    position: relative;
    z-index: 10;
}

.splitter-v .splitter-line {
    width: 1px;
    height: 100%;
    background-color: var(--border-default);
    transition: width 0.12s ease, background-color 0.12s ease;
}

.splitter-v:hover .splitter-line, .splitter-v.active .splitter-line {
    width: 3px;
    background-color: var(--border-focus);
}

.splitter-h {
    width: 100%;
    height: 7px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    cursor: row-resize;
    flex-shrink: 0;
    position: relative;
    z-index: 10;
}

.splitter-h .splitter-line {
    width: 100%;
    height: 1px;
    background-color: var(--border-default);
    transition: height 0.12s ease, background-color 0.12s ease;
}

.splitter-h:hover .splitter-line, .splitter-h.active .splitter-line {
    height: 3px;
    background-color: var(--border-focus);
}

/* Status indicator pulse */
@keyframes pulse-dot {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.45; transform: scale(0.88); }
}

.status-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background-color: var(--success);
    flex-shrink: 0;
}

.status-dot.busy {
    background-color: var(--text-accent);
    animation: pulse-dot 1.2s infinite ease-in-out;
}

/* Form group helpers */
.form-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-secondary);
    margin-bottom: 2px;
}

.form-hint {
    font-size: 11px;
    color: var(--text-muted);
    line-height: 1.4;
}

.pill-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 1px 7px;
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    border-radius: var(--radius-full);
    background-color: rgba(96, 165, 250, 0.14);
    color: var(--text-accent);
}

/* Email Reader Header & Envelope */
.email-header-envelope {
    display: flex;
    flex-direction: row;
    align-items: flex-start;
    justify-content: space-between;
    gap: 14px;
    padding: 9px 12px;
    background-color: rgba(15, 23, 42, 0.5);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
}

.email-sender-avatar {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: linear-gradient(135deg, rgba(59, 130, 246, 0.28) 0%, rgba(29, 78, 216, 0.44) 100%);
    border: 1px solid rgba(96, 165, 250, 0.42);
    color: #93c5fd;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.03em;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    box-shadow: var(--shadow-sm);
}

.email-envelope-fields {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
}

.email-field-row {
    display: flex;
    flex-direction: row;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
}

.email-field-label {
    min-width: 40px;
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
    flex-shrink: 0;
}

.email-field-values {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: row;
    align-items: center;
    flex-wrap: wrap;
    gap: 5px;
}

.email-contact-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 8px;
    border-radius: var(--radius-sm);
    background-color: var(--bg-elevated);
    border: 1px solid var(--border-default);
    font-size: 11.5px;
    line-height: 1.35;
    max-width: 100%;
}

.email-contact-pill.sender {
    background-color: rgba(37, 99, 235, 0.14);
    border-color: rgba(96, 165, 250, 0.35);
}

.email-contact-name {
    font-weight: 600;
    color: var(--text-primary);
}

.email-contact-addr {
    font-size: 11px;
    font-family: var(--font-mono);
    color: var(--text-accent);
    opacity: 0.92;
}

.email-datetime-card {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
    flex-shrink: 0;
    text-align: right;
}

.email-datetime-primary {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 9px;
    background-color: var(--bg-elevated);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-sm);
    font-size: 11.5px;
    font-weight: 500;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
}

.email-time-badge {
    padding: 1px 6px;
    background-color: rgba(59, 130, 246, 0.16);
    border-radius: var(--radius-xs);
    color: var(--text-accent);
    font-weight: 600;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
}

.email-folder-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-muted);
    white-space: nowrap;
}

/* Rich HTML Email Reader Formatting */
.email-html-body {
    font-family: var(--font-sans);
    font-size: 13.5px;
    line-height: 1.65;
    color: var(--text-primary);
    word-break: break-word;
    overflow-wrap: anywhere;
}

.email-html-body p,
.email-html-body .email-paragraph {
    margin: 0 0 12px 0;
}

.email-html-body p:last-child,
.email-html-body .email-paragraph:last-child {
    margin-bottom: 0;
}

.email-html-body h1,
.email-html-body h2,
.email-html-body h3,
.email-html-body h4,
.email-html-body h5,
.email-html-body h6 {
    color: var(--text-primary);
    font-weight: 600;
    line-height: 1.3;
    margin: 16px 0 8px 0;
}

.email-html-body h1:first-child,
.email-html-body h2:first-child,
.email-html-body h3:first-child {
    margin-top: 0;
}

.email-html-body h1 { font-size: 18px; }
.email-html-body h2 { font-size: 16px; }
.email-html-body h3 { font-size: 14.5px; }
.email-html-body h4,
.email-html-body h5,
.email-html-body h6 { font-size: 13.5px; }

.email-html-body a,
.email-html-body .email-link {
    color: var(--text-accent);
    text-decoration: underline;
    text-decoration-color: rgba(96, 165, 250, 0.45);
    text-underline-offset: 2px;
    transition: color 0.12s ease, text-decoration-color 0.12s ease;
}

.email-html-body a:hover,
.email-html-body .email-link:hover {
    color: #93c5fd;
    text-decoration-color: #93c5fd;
}

.email-html-body .email-address-link {
    display: inline-block;
    color: #93c5fd;
    background-color: rgba(59, 130, 246, 0.12);
    border: 1px solid rgba(96, 165, 250, 0.24);
    padding: 0px 6px;
    border-radius: var(--radius-xs);
    text-decoration: none;
    font-size: 12.5px;
    line-height: 1.45;
}

.email-html-body .email-address-link:hover {
    background-color: rgba(59, 130, 246, 0.22);
    border-color: rgba(96, 165, 250, 0.45);
    text-decoration: none;
}

.email-html-body ul,
.email-html-body ol,
.email-html-body .email-list {
    margin: 0 0 12px 0;
    padding-left: 22px;
}

.email-html-body li {
    margin-bottom: 4px;
}

.email-html-body li:last-child {
    margin-bottom: 0;
}

.email-html-body blockquote,
.email-html-body .email-quote {
    margin: 8px 0 12px 0;
    padding: 8px 14px;
    border-left: 3px solid rgba(96, 165, 250, 0.55);
    background-color: rgba(15, 23, 42, 0.55);
    border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
    color: var(--text-secondary);
}

.email-html-body blockquote blockquote,
.email-html-body .email-quote .email-quote {
    margin: 6px 0;
    border-left-color: rgba(148, 163, 184, 0.42);
    background-color: rgba(255, 255, 255, 0.025);
}

.email-html-body .email-quote-attribution {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
    margin: 12px 0 4px 0;
}

.email-html-body .email-reply-header {
    margin: 14px 0 10px 0;
    padding: 10px 14px;
    background-color: var(--bg-elevated);
    border: 1px solid var(--border-default);
    border-left: 3px solid var(--primary-hover);
    border-radius: var(--radius-sm);
    font-size: 12px;
    line-height: 1.45;
}

.email-html-body .email-reply-header-title {
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
    margin-bottom: 6px;
    padding-bottom: 4px;
    border-bottom: 1px solid var(--border-subtle);
}

.email-html-body .email-reply-meta-grid {
    display: flex;
    flex-direction: column;
    gap: 3px;
}

.email-html-body .email-reply-meta-row {
    display: flex;
    flex-direction: row;
    align-items: baseline;
    gap: 10px;
}

.email-html-body .email-reply-key {
    min-width: 72px;
    font-weight: 600;
    color: var(--text-muted);
    flex-shrink: 0;
}

.email-html-body .email-reply-val {
    color: var(--text-primary);
    flex: 1;
    min-width: 0;
}

.email-html-body hr,
.email-html-body .email-divider {
    border: none;
    border-top: 1px solid var(--border-default);
    margin: 14px 0;
}

.email-html-body table {
    border-collapse: collapse;
    width: auto;
    max-width: 100%;
    margin: 10px 0 14px 0;
    font-size: 12.5px;
    background-color: rgba(15, 23, 42, 0.42);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-sm);
    overflow: hidden;
}

.email-html-body th,
.email-html-body td {
    padding: 7px 11px;
    border: 1px solid var(--border-subtle);
    text-align: left;
    vertical-align: top;
}

.email-html-body th {
    background-color: var(--bg-elevated);
    font-weight: 600;
    color: var(--text-primary);
}

.email-html-body pre {
    font-family: var(--font-mono);
    font-size: 12px;
    background-color: #090d16;
    border: 1px solid var(--border-default);
    border-radius: var(--radius-sm);
    padding: 10px 12px;
    overflow-x: auto;
    margin: 8px 0 12px 0;
}

.email-html-body code {
    font-family: var(--font-mono);
    font-size: 12px;
    background-color: rgba(15, 23, 42, 0.7);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-xs);
    padding: 1px 5px;
}

.email-html-body pre code {
    background: none;
    border: none;
    padding: 0;
}

.email-html-body .email-signature {
    margin-top: 16px;
    padding-top: 10px;
    border-top: 1px dashed var(--border-default);
    font-size: 12px;
    color: var(--text-muted);
    line-height: 1.5;
}

.email-html-body .email-disclaimer {
    margin-top: 14px;
    padding: 8px 12px;
    background-color: rgba(15, 23, 42, 0.48);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    font-size: 11px;
    color: var(--text-muted);
    line-height: 1.45;
}

.email-html-body .email-cid-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    font-size: 11.5px;
    background-color: var(--bg-elevated);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-xs);
    color: var(--text-secondary);
}

.email-html-body .email-inline-img {
    max-width: 100%;
    height: auto;
    border-radius: var(--radius-sm);
    margin: 6px 0;
}

/* Scrollbars */
::-webkit-scrollbar {
    width: 7px;
    height: 7px;
}
::-webkit-scrollbar-track {
    background: transparent;
}
::-webkit-scrollbar-thumb {
    background: rgba(148, 163, 184, 0.22);
    border-radius: 4px;
}
::-webkit-scrollbar-thumb:hover {
    background: rgba(96, 165, 250, 0.48);
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
        Button {
            variant: ButtonVariant::Primary,
            size: ButtonSize::Sm,
            style: "font-size: 12px; padding: 6px 12px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;",
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
        Button {
            variant: ButtonVariant::Outline,
            size: ButtonSize::Xs,
            style: "font-size: 11px; padding: 5px 10px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;",
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
        Button {
            variant: ButtonVariant::Primary,
            size: ButtonSize::Sm,
            style: "font-size: 12px; padding: 6px 14px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;",
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
        Button {
            variant: ButtonVariant::Outline,
            style: "width: 100%; height: auto; padding: 10px 12px; display: flex; flex-direction: row; align-items: center; justify-content: space-between; gap: 10px; border-radius: var(--radius-md); white-space: normal;",
            onclick: move |_| on_press.clone().dispatch(),
            span { style: "font-size: 12px; flex: 1; text-align: left; line-height: 1.35;", "{label_text}" }
            span {
                style: "font-size: 11px; font-weight: 600; color: {accent}; display: inline-flex; align-items: center; gap: 4px; flex-shrink: 0;",
                "Use →"
            }
        }
    }
}

pub(super) fn collapsible_section_label<M: DispatchMessage>(
    label: &str,
    collapsed: bool,
    on_toggle: M,
) -> Element {
    let label_text = label.to_owned();
    let chevron_rotation = if collapsed { "-90deg" } else { "0deg" };
    let muted_color = muted();
    rsx! {
        CollapsibleTrigger {
            style: "width: 100%; padding: 6px 4px; display: flex; flex-direction: row; align-items: center; justify-content: space-between; border-radius: var(--radius-xs);",
            as: move |_attrs: Vec<Attribute>| {
                let on_toggle = on_toggle.clone();
                let label_text = label_text.clone();
                rsx! {
                    Button {
                        variant: ButtonVariant::Ghost,
                        size: ButtonSize::Xs,
                        style: "width: 100%; padding: 6px 4px; display: flex; flex-direction: row; align-items: center; justify-content: space-between; border-radius: var(--radius-xs);",
                        onclick: move |_| on_toggle.clone().dispatch(),
                        span {
                            style: "font-size: 10.5px; color: {muted_color}; flex: 1; text-align: left; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase;",
                            "{label_text}"
                        }
                        svg {
                            width: "14",
                            height: "14",
                            view_box: "0 0 24 24",
                            fill: "none",
                            stroke: "{muted_color}",
                            stroke_width: "2",
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            style: "transition: transform 0.16s ease; transform: rotate({chevron_rotation}); flex-shrink: 0;",
                            polyline { points: "6 9 12 15 18 9" }
                        }
                    }
                }
            },
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
        r: base.r * 0.24,
        g: base.g * 0.24,
        b: base.b * 0.24,
        a: 0.92,
    };
    let border_col = Color {
        r: base.r,
        g: base.g,
        b: base.b,
        a: 0.55,
    };
    let label_name = name.to_owned();
    let bright = bright_text();
    rsx! {
        Badge {
            variant: BadgeVariant::Outline,
            style: "display: inline-flex; align-items: center; gap: 5px; padding: 2px 8px; font-size: 11px; font-weight: 500; color: {bright}; background-color: {bg}; border: 1px solid {border_col}; border-radius: 9999px; white-space: nowrap;",
            span {
                style: "width: 6px; height: 6px; border-radius: 50%; background-color: {base}; flex-shrink: 0;"
            }
            span { "{label_name}" }
            if let Some(extra) = detail {
                span {
                    style: "font-size: 10px; opacity: 0.78; font-variant-numeric: tabular-nums;",
                    "{extra}"
                }
            }
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

/// A draggable pane boundary. Emits `on_press` when the user grabs it; the
/// drag itself is tracked by the application-level mouse listener.
pub(super) fn splitter<M: DispatchMessage>(axis: SplitAxis, active: bool, on_press: M) -> Element {
    let active_class = if active { "active" } else { "" };
    match axis {
        SplitAxis::Vertical => rsx! {
            div {
                class: "splitter-v {active_class}",
                onmousedown: move |evt| {
                    evt.prevent_default();
                    on_press.clone().dispatch();
                },
                div { class: "splitter-line" }
            }
        },
        SplitAxis::Horizontal => rsx! {
            div {
                class: "splitter-h {active_class}",
                onmousedown: move |evt| {
                    evt.prevent_default();
                    on_press.clone().dispatch();
                },
                div { class: "splitter-line" }
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
    format!("background-color: var(--bg-app); color: {BRIGHT_TEXT};")
}
pub(super) fn sidebar_style() -> String {
    "background-color: var(--bg-sidebar); border-right: 1px solid var(--border-default);".into()
}
pub(super) fn chat_pane_style() -> String {
    "background-color: var(--bg-sidebar); border-left: 1px solid var(--border-default);".into()
}
pub(super) fn toolbar_style() -> String {
    "background-color: var(--bg-surface); border: 1px solid var(--border-default); border-radius: var(--radius-md); box-shadow: var(--shadow-sm);".into()
}
pub(super) fn selection_bar_style() -> String {
    "background: linear-gradient(90deg, rgba(37, 99, 235, 0.22) 0%, rgba(30, 58, 138, 0.28) 100%); border: 1px solid rgba(96, 165, 250, 0.45); border-radius: var(--radius-md); box-shadow: var(--shadow-sm);".into()
}
#[allow(dead_code)]
pub(super) fn table_header_style() -> String {
    format!("background-color: {SURFACE_ELEVATED}; border-bottom: 1px solid {OUTLINE};")
}
#[allow(dead_code)]
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
    "background-color: var(--bg-surface); border: 1px solid var(--border-default); border-radius: var(--radius-md); box-shadow: var(--shadow-sm);".into()
}
pub(super) fn reader_body_style() -> String {
    "background-color: var(--bg-surface); border: 1px solid var(--border-default); border-radius: var(--radius-md); box-shadow: var(--shadow-sm);".into()
}
pub(super) fn code_block_style() -> String {
    "background-color: #090d16; border: 1px solid var(--border-default); border-radius: var(--radius-sm);".into()
}
pub(super) fn blockquote_style() -> String {
    "background-color: rgba(59, 130, 246, 0.07); border-left: 3px solid var(--border-focus); border-radius: 0 var(--radius-sm) var(--radius-sm) 0;".into()
}
pub(super) fn md_table_header_style() -> String {
    "background-color: var(--bg-elevated); border-bottom: 1px solid var(--border-default);".into()
}
pub(super) fn md_table_row_style() -> String {
    "background-color: var(--bg-surface); border-bottom: 1px solid var(--border-subtle);".into()
}
fn card_style() -> String {
    "background-color: var(--bg-surface); border: 1px solid var(--border-subtle); border-radius: var(--radius-md); box-shadow: var(--shadow-sm);".into()
}
pub(super) fn status_style() -> String {
    "background-color: var(--bg-surface); border: 1px solid var(--border-subtle); border-radius: var(--radius-full);".into()
}
#[allow(dead_code)]
pub(super) fn badge_style() -> String {
    let c = shade(TERTIARY, 0.22);
    panel(c, c, 999.0)
}
pub(super) fn source_style() -> String {
    "background-color: var(--bg-surface); border: 1px solid var(--border-default); border-radius: var(--radius-md); box-shadow: var(--shadow-sm);".into()
}
pub(super) fn accent_tile_style() -> String {
    "background: linear-gradient(135deg, #3b82f6 0%, #1d4ed8 100%); border: 1px solid rgba(147, 197, 253, 0.35); border-radius: 9px; box-shadow: 0 2px 8px rgba(37, 99, 235, 0.35);".into()
}
pub(super) fn user_avatar_style() -> String {
    panel(USER_BUBBLE, USER_BUBBLE, 999.0)
}
pub(super) fn assistant_avatar_style() -> String {
    format!(
        "{} color: {ACCENT_TEXT};",
        panel(shade(SECONDARY, 0.22), OUTLINE, 999.0)
    )
}
pub(super) fn user_bubble_style() -> String {
    "background: linear-gradient(180deg, #1d4ed8 0%, #1e40af 100%); border: 1px solid rgba(96, 165, 250, 0.35); border-radius: 12px 12px 2px 12px; box-shadow: var(--shadow-sm);".into()
}
pub(super) fn assistant_bubble_style() -> String {
    "background-color: var(--bg-surface); border: 1px solid var(--border-default); border-radius: var(--radius-md) var(--radius-md) var(--radius-md) 2px; box-shadow: var(--shadow-sm);".into()
}
pub(super) fn composer_style() -> String {
    "background-color: var(--bg-surface); border: 1px solid var(--border-default); border-radius: var(--radius-lg); box-shadow: var(--shadow-md);".into()
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
            let custom: Element = constrain_button(rsx! { Button { "Custom" } });
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
