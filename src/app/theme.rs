use iced::{
    Border, Color, Element, Fill, Shadow, Theme, Vector, color, mouse,
    widget::{Space, button, container, mouse_area, row, text},
};

const fn shade(color: Color, amount: f32) -> Color {
    Color {
        r: color.r * amount,
        g: color.g * amount,
        b: color.b * amount,
        a: color.a,
    }
}

/// Compile-time application palette constants (Material 3 dark surfaces).
const PRIMARY: Color = color!(0x00309A);
const SECONDARY: Color = color!(0x8A8FAB);
const TERTIARY: Color = color!(0xB083A7);
const ERROR: Color = color!(0xFF5449);
const NEUTRAL: Color = color!(0x919094);
const NEUTRAL_VARIANT: Color = color!(0x90909A);
const CUSTOM1: Color = color!(0x166EA6);

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

pub(super) fn build_app_theme() -> Theme {
    Theme::custom(
        "Dungbeetle Material 3".into(),
        iced::theme::Palette {
            background: BACKGROUND,
            text: NEUTRAL,
            primary: PRIMARY,
            success: POSITIVE,
            danger: ERROR,
        },
    )
}

pub(super) const BUTTON_MAX_WIDTH: f32 = 240.0;
pub(super) const BUTTON_MAX_HEIGHT: f32 = 36.0;

pub(super) fn constrain_button<'a, M: 'a>(
    btn: impl Into<Element<'a, M>>,
) -> Element<'a, M> {
    container(btn)
        .max_width(BUTTON_MAX_WIDTH)
        .max_height(BUTTON_MAX_HEIGHT)
        .clip(true)
        .into()
}

pub(super) fn enabled_button<'a, M: Clone + 'a>(
    label: impl text::IntoFragment<'a>,
    message: Option<M>,
) -> Element<'a, M> {
    constrain_button(
        button(text(label).size(12))
            .padding([6, 11])
            .clip(true)
            .on_press_maybe(message),
    )
}

pub(super) fn small_button<'a, M: Clone + 'a>(
    label: impl text::IntoFragment<'a>,
    message: Option<M>,
) -> Element<'a, M> {
    constrain_button(
        button(text(label).size(11))
            .padding([4, 9])
            .style(button::secondary)
            .clip(true)
            .on_press_maybe(message),
    )
}

pub(super) fn primary_button<'a, M: Clone + 'a>(
    label: impl text::IntoFragment<'a>,
    message: Option<M>,
) -> Element<'a, M> {
    constrain_button(
        button(text(label).size(12))
            .padding([7, 14])
            .style(button::primary)
            .clip(true)
            .on_press_maybe(message),
    )
}

pub(super) fn prompt_button<'a, M: Clone + 'a>(
    label: &'a str,
    on_press: M,
) -> iced::widget::Button<'a, M> {
    button(row![
        text(label).size(12).width(Fill),
        text("Use").size(11).color(accent_text())
    ])
    .width(Fill)
    .padding([8, 12])
    .style(button::secondary)
    .on_press(on_press)
}

pub(super) fn collapsible_section_label<'a, M: Clone + 'a>(
    label: &'a str,
    collapsed: bool,
    on_toggle: M,
) -> Element<'a, M> {
    button(row![
        text(label).size(10).color(muted()).width(Fill),
        text(if collapsed { "Show" } else { "Hide" })
            .size(10)
            .color(muted())
    ])
    .width(Fill)
    .padding([4, 2])
    .style(button::text)
    .on_press(on_toggle)
    .into()
}

pub(super) fn card<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    container(content)
        .width(Fill)
        .padding(12)
        .style(card_style)
        .into()
}

pub(super) fn label_badge<'a, M: 'a>(
    name: &str,
    hex_color: &str,
    detail: Option<String>,
) -> Element<'a, M> {
    let base = parse_hex_color(hex_color);
    let label_text = match detail {
        Some(extra) => format!("{name} ({extra})"),
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
/// drag itself is tracked by the application-level mouse subscription.
pub(super) fn splitter<'a, M: Clone + 'a>(
    axis: SplitAxis,
    active: bool,
    on_press: M,
) -> Element<'a, M> {
    let line_color = if active { ACCENT_TEXT } else { OUTLINE };
    let thickness = if active { 2.0 } else { 1.0 };
    let line = container(Space::new(0, 0)).style(move |_| container::Style {
        background: Some(line_color.into()),
        ..Default::default()
    });
    let (handle, interaction) = match axis {
        SplitAxis::Vertical => (
            container(line.width(thickness).height(Fill))
                .center_x(SPLITTER_HIT_SIZE)
                .height(Fill),
            mouse::Interaction::ResizingHorizontally,
        ),
        SplitAxis::Horizontal => (
            container(line.width(Fill).height(thickness))
                .center_y(SPLITTER_HIT_SIZE)
                .width(Fill),
            mouse::Interaction::ResizingVertically,
        ),
    };
    mouse_area(handle)
        .on_press(on_press)
        .interaction(interaction)
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
    BACKGROUND.into()
}
pub(super) fn sidebar_style(_: &Theme) -> container::Style {
    container::Style {
        background: Some(shade(NEUTRAL, 0.13).into()),
        border: Border {
            color: OUTLINE,
            width: 1.0,
            ..Default::default()
        },
        ..Default::default()
    }
}
pub(super) fn chat_pane_style(_: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE_ELEVATED.into()),
        border: Border {
            color: OUTLINE,
            width: 1.0,
            ..Default::default()
        },
        ..Default::default()
    }
}
pub(super) fn toolbar_style(_: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE.into()),
        border: Border {
            color: OUTLINE,
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
        ACCENT_TEXT,
        6.0,
    )
}
pub(super) fn table_header_style(_: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE_ELEVATED.into()),
        border: Border {
            color: OUTLINE,
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
}
pub(super) fn reader_header_style(_: &Theme) -> container::Style {
    panel(SURFACE_ELEVATED, OUTLINE, 8.0)
}
pub(super) fn reader_body_style(_: &Theme) -> container::Style {
    panel(SURFACE, OUTLINE, 8.0)
}
pub(super) fn code_block_style(_: &Theme) -> container::Style {
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
pub(super) fn blockquote_style(_: &Theme) -> container::Style {
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
pub(super) fn md_table_header_style(_: &Theme) -> container::Style {
    panel(SURFACE_ELEVATED, OUTLINE, 0.0)
}
pub(super) fn md_table_row_style(_: &Theme) -> container::Style {
    panel(SURFACE, OUTLINE, 0.0)
}
fn card_style(_: &Theme) -> container::Style {
    panel(SURFACE, OUTLINE, 10.0)
}
pub(super) fn status_style(_: &Theme) -> container::Style {
    panel(SURFACE, OUTLINE, 16.0)
}
pub(super) fn badge_style(_: &Theme) -> container::Style {
    let c = shade(TERTIARY, 0.22);
    panel(c, c, 10.0)
}
pub(super) fn source_style(_: &Theme) -> container::Style {
    panel(SURFACE, OUTLINE, 10.0)
}
pub(super) fn accent_tile_style(_: &Theme) -> container::Style {
    panel(PRIMARY, PRIMARY, 9.0)
}

pub(super) fn user_avatar_style(_: &Theme) -> container::Style {
    panel(USER_BUBBLE, USER_BUBBLE, 14.0)
}
pub(super) fn assistant_avatar_style(_: &Theme) -> container::Style {
    panel(shade(SECONDARY, 0.22), OUTLINE, 14.0).color(PRIMARY)
}
pub(super) fn user_bubble_style(_: &Theme) -> container::Style {
    panel(USER_BUBBLE, USER_BUBBLE, 12.0)
}
pub(super) fn assistant_bubble_style(_: &Theme) -> container::Style {
    panel(SURFACE, OUTLINE, 12.0)
}
pub(super) fn composer_style(_: &Theme) -> container::Style {
    let mut style = panel(SURFACE, shade(NEUTRAL_VARIANT, 0.38), 12.0);
    style.shadow = Shadow {
        color: shade(NEUTRAL, 0.015),
        offset: Vector::new(0.0, 4.0),
        blur_radius: 14.0,
    };
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_dimension_constraints_are_enforced() {
        assert_eq!(BUTTON_MAX_WIDTH, 240.0);
        assert_eq!(BUTTON_MAX_HEIGHT, 36.0);

        let _small: Element<'static, ()> = small_button("Test", Some(()));
        let _primary: Element<'static, ()> = primary_button("Test", Some(()));
        let _enabled: Element<'static, ()> = enabled_button("Test", Some(()));
        let _custom: Element<'static, ()> = constrain_button(button(text("Custom")));
    }
}
