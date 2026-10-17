use iced::{
    Border, Color, Element, Fill, Shadow, Theme, Vector,
    widget::{button, container, row, text},
};

use super::Message;

/// The seven source colors used by the interface. Keeping them as hex strings
/// makes the palette easy to copy from design tools and update without math.
#[derive(Clone, Copy)]
struct Palette {
    primary: &'static str,
    secondary: &'static str,
    tertiary: &'static str,
    error: &'static str,
    neutral: &'static str,
    neutral_variant: &'static str,
    custom1: &'static str,
}

const PALETTE: Palette = Palette {
    primary: "#00309A",
    secondary: "#8A8FAB",
    tertiary: "#B083A7",
    error: "#FF5449",
    neutral: "#919094",
    neutral_variant: "#90909A",
    custom1: "#166EA6",
};

pub(super) fn primary() -> Color {
    color(PALETTE.primary)
}
pub(super) fn accent_text() -> Color {
    Color::from_rgb(0.48, 0.72, 0.98)
}
pub(super) fn bright_text() -> Color {
    Color::from_rgb(0.90, 0.91, 0.94)
}
pub(super) fn muted() -> Color {
    shade(color(PALETTE.neutral_variant), 0.78)
}
pub(super) fn background() -> Color {
    shade(color(PALETTE.neutral), 0.15)
}
pub(super) fn on_primary() -> Color {
    Color::from_rgb(0.92, 0.95, 1.0)
}
pub(super) fn positive() -> Color {
    color(PALETTE.custom1)
}
pub(super) fn parse_hex_color(hex: &str) -> Color {
    Color::parse(hex).unwrap_or_else(positive)
}

fn surface() -> Color {
    shade(color(PALETTE.neutral), 0.09)
}
fn surface_elevated() -> Color {
    shade(color(PALETTE.neutral), 0.12)
}
fn outline() -> Color {
    shade(color(PALETTE.neutral_variant), 0.28)
}
fn user_bubble() -> Color {
    shade(color(PALETTE.primary), 0.48)
}

pub(super) fn app_theme() -> Theme {
    Theme::custom(
        "Dungbeetle Material 3".into(),
        iced::theme::Palette {
            background: background(),
            text: color(PALETTE.neutral),
            primary: primary(),
            success: positive(),
            danger: color(PALETTE.error),
        },
    )
}

pub(super) fn enabled_button<'a>(
    label: impl text::IntoFragment<'a>,
    message: Option<Message>,
) -> iced::widget::Button<'a, Message> {
    let button = button(text(label).size(12)).padding([6, 11]);
    if let Some(message) = message {
        button.on_press(message)
    } else {
        button
    }
}

pub(super) fn small_button<'a>(
    label: impl text::IntoFragment<'a>,
    message: Option<Message>,
) -> iced::widget::Button<'a, Message> {
    let button = button(text(label).size(11))
        .padding([4, 9])
        .style(button::secondary);
    if let Some(message) = message {
        button.on_press(message)
    } else {
        button
    }
}

pub(super) fn primary_button<'a>(
    label: impl text::IntoFragment<'a>,
    message: Option<Message>,
) -> iced::widget::Button<'a, Message> {
    let button = button(text(label).size(12))
        .padding([7, 14])
        .style(button::primary);
    if let Some(message) = message {
        button.on_press(message)
    } else {
        button
    }
}

pub(super) fn prompt_button<'a>(
    label: &'a str,
    prompt: &'static str,
) -> iced::widget::Button<'a, Message> {
    button(row![
        text(label).size(12).width(Fill),
        text("->").size(12).color(accent_text())
    ])
    .width(Fill)
    .padding([8, 12])
    .style(button::secondary)
    .on_press(Message::UsePrompt(prompt))
}

pub(super) fn collapsible_section_label(
    label: &str,
    collapsed: bool,
    section: super::SidebarSection,
) -> Element<'_, Message> {
    button(row![
        text(label).size(10).color(muted()).width(Fill),
        text(if collapsed { "+" } else { "-" })
            .size(11)
            .color(muted())
    ])
    .width(Fill)
    .padding([4, 2])
    .style(button::text)
    .on_press(Message::ToggleSidebarSection(section))
    .into()
}

pub(super) fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .width(Fill)
        .padding(12)
        .style(card_style)
        .into()
}

pub(super) fn label_badge<'a>(
    name: &str,
    hex_color: &str,
    detail: Option<String>,
) -> Element<'a, Message> {
    let base = parse_hex_color(hex_color);
    let label_text = match detail {
        Some(extra) => format!("{name} \u{00b7} {extra}"),
        None => name.to_owned(),
    };
    container(text(label_text).size(10).color(bright_text()))
        .padding([2, 7])
        .style(move |_| container::Style {
            background: Some(
                Color {
                    r: base.r * 0.35,
                    g: base.g * 0.35,
                    b: base.b * 0.35,
                    a: 0.95,
                }
                .into(),
            ),
            border: Border::default().rounded(8.0).color(base).width(1),
            ..Default::default()
        })
        .into()
}

fn panel(background: Color, border: Color, radius: f32) -> container::Style {
    container::Style {
        background: Some(background.into()),
        border: Border::default().rounded(radius).color(border).width(1),
        ..Default::default()
    }
}

pub(super) fn page_style(_: &Theme) -> container::Style {
    background().into()
}
pub(super) fn sidebar_style(_: &Theme) -> container::Style {
    container::Style {
        background: Some(shade(color(PALETTE.neutral), 0.13).into()),
        border: Border {
            color: outline(),
            width: 1.0,
            ..Default::default()
        },
        ..Default::default()
    }
}
pub(super) fn chat_pane_style(_: &Theme) -> container::Style {
    container::Style {
        background: Some(shade(color(PALETTE.neutral), 0.12).into()),
        border: Border {
            color: outline(),
            width: 1.0,
            ..Default::default()
        },
        ..Default::default()
    }
}
pub(super) fn toolbar_style(_: &Theme) -> container::Style {
    container::Style {
        background: Some(surface().into()),
        border: Border {
            color: outline(),
            width: 1.0,
            ..Default::default()
        },
        ..Default::default()
    }
}
pub(super) fn selection_bar_style(_: &Theme) -> container::Style {
    panel(
        Color {
            r: 0.07,
            g: 0.15,
            b: 0.29,
            a: 0.95,
        },
        accent_text(),
        6.0,
    )
}
pub(super) fn table_header_style(_: &Theme) -> container::Style {
    container::Style {
        background: Some(surface_elevated().into()),
        border: Border {
            color: outline(),
            width: 1.0,
            ..Default::default()
        },
        ..Default::default()
    }
}
pub(super) fn mail_row_style(selected: bool, checked: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        if selected {
            panel(
                Color {
                    r: 0.08,
                    g: 0.20,
                    b: 0.42,
                    a: 1.0,
                },
                accent_text(),
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
                shade(accent_text(), 0.55),
                2.0,
            )
        } else {
            panel(surface(), outline(), 0.0)
        }
    }
}
pub(super) fn reader_header_style(_: &Theme) -> container::Style {
    panel(surface_elevated(), outline(), 8.0)
}
pub(super) fn reader_body_style(_: &Theme) -> container::Style {
    panel(surface(), outline(), 8.0)
}
pub(super) fn code_block_style(_: &Theme) -> container::Style {
    panel(
        Color {
            r: 0.04,
            g: 0.05,
            b: 0.07,
            a: 0.98,
        },
        outline(),
        6.0,
    )
}
pub(super) fn blockquote_style(_: &Theme) -> container::Style {
    panel(
        Color {
            r: 0.07,
            g: 0.09,
            b: 0.13,
            a: 0.92,
        },
        shade(accent_text(), 0.55),
        6.0,
    )
}
pub(super) fn md_table_header_style(_: &Theme) -> container::Style {
    panel(surface_elevated(), outline(), 0.0)
}
pub(super) fn md_table_row_style(_: &Theme) -> container::Style {
    panel(surface(), outline(), 0.0)
}
fn card_style(_: &Theme) -> container::Style {
    panel(surface(), outline(), 10.0)
}
pub(super) fn status_style(_: &Theme) -> container::Style {
    panel(surface(), outline(), 16.0)
}
pub(super) fn badge_style(_: &Theme) -> container::Style {
    panel(
        shade(color(PALETTE.tertiary), 0.22),
        shade(color(PALETTE.tertiary), 0.22),
        10.0,
    )
}
pub(super) fn source_style(_: &Theme) -> container::Style {
    panel(surface(), outline(), 10.0)
}
pub(super) fn accent_tile_style(_: &Theme) -> container::Style {
    panel(primary(), primary(), 9.0)
}
pub(super) fn user_avatar_style(_: &Theme) -> container::Style {
    panel(user_bubble(), user_bubble(), 14.0)
}
pub(super) fn assistant_avatar_style(_: &Theme) -> container::Style {
    panel(shade(color(PALETTE.secondary), 0.22), outline(), 14.0).color(primary())
}
pub(super) fn user_bubble_style(_: &Theme) -> container::Style {
    panel(user_bubble(), user_bubble(), 12.0)
}
pub(super) fn assistant_bubble_style(_: &Theme) -> container::Style {
    panel(surface(), outline(), 12.0)
}
pub(super) fn composer_style(_: &Theme) -> container::Style {
    let mut style = panel(surface(), shade(color(PALETTE.neutral_variant), 0.38), 12.0);
    style.shadow = Shadow {
        color: shade(color(PALETTE.neutral), 0.015),
        offset: Vector::new(0.0, 4.0),
        blur_radius: 14.0,
    };
    style
}

fn color(hex: &str) -> Color {
    Color::parse(hex).expect("application palette must contain valid #RRGGBB colors")
}

fn shade(color: Color, amount: f32) -> Color {
    Color {
        r: color.r * amount,
        g: color.g * amount,
        b: color.b * amount,
        a: color.a,
    }
}
