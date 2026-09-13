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
pub(super) fn muted() -> Color {
    shade(color(PALETTE.neutral_variant), 0.78)
}
pub(super) fn background() -> Color {
    shade(color(PALETTE.neutral), 0.15)
}
pub(super) fn on_primary() -> Color {
    shade(color(PALETTE.primary), 0.10)
}
pub(super) fn positive() -> Color {
    color(PALETTE.custom1)
}

fn surface() -> Color {
    shade(color(PALETTE.neutral), 0.09)
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
    label: &'a str,
    message: Option<Message>,
) -> iced::widget::Button<'a, Message> {
    let button = button(label);
    if let Some(message) = message {
        button.on_press(message)
    } else {
        button
    }
}

pub(super) fn primary_button<'a>(
    label: &'a str,
    message: Option<Message>,
) -> iced::widget::Button<'a, Message> {
    let button = button(label).padding([9, 16]).style(button::primary);
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
        text(label).size(13).width(Fill),
        text("->").size(13).color(primary())
    ])
    .width(Fill)
    .padding([10, 14])
    .style(button::secondary)
    .on_press(Message::UsePrompt(prompt))
}

pub(super) fn section_label(label: &str) -> Element<'_, Message> {
    text(label).size(10).color(muted()).into()
}
pub(super) fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .width(Fill)
        .padding(16)
        .style(card_style)
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
        background: Some(shade(color(PALETTE.neutral), 0.15).into()),
        border: Border {
            color: outline(),
            width: 1.0,
            ..Default::default()
        },
        ..Default::default()
    }
}
fn card_style(_: &Theme) -> container::Style {
    panel(surface(), outline(), 12.0)
}
pub(super) fn status_style(_: &Theme) -> container::Style {
    panel(surface(), outline(), 20.0)
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
    panel(primary(), primary(), 11.0)
}
pub(super) fn user_avatar_style(_: &Theme) -> container::Style {
    panel(user_bubble(), user_bubble(), 15.0)
}
pub(super) fn assistant_avatar_style(_: &Theme) -> container::Style {
    panel(shade(color(PALETTE.secondary), 0.22), outline(), 15.0).color(primary())
}
pub(super) fn user_bubble_style(_: &Theme) -> container::Style {
    panel(user_bubble(), user_bubble(), 14.0)
}
pub(super) fn assistant_bubble_style(_: &Theme) -> container::Style {
    panel(surface(), outline(), 14.0)
}
pub(super) fn composer_style(_: &Theme) -> container::Style {
    let mut style = panel(surface(), shade(color(PALETTE.neutral_variant), 0.38), 14.0);
    style.shadow = Shadow {
        color: shade(color(PALETTE.neutral), 0.015),
        offset: Vector::new(0.0, 5.0),
        blur_radius: 18.0,
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
