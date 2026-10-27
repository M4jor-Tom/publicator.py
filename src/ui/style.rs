//! The retired web UI's look: dark page, #111 header, #2a2a2a cards in a fluid
//! grid, browser-grey full-width buttons and white fields.
use super::Message;
use iced::{
    Background, Border, Color, ContentFit, Element, Font, Length, Theme, color, font,
    theme::Palette,
    widget::{
        Button, Container, button, container, grid, image, pick_list, text, text_editor, text_input,
    },
};
use std::{ops::Range, path::Path};

pub const ACCENT: Color = color!(0x88aacc);
const WARNING: Color = color!(0xe0a800);
pub const ERROR: Color = color!(0xff9090);
pub const MUTED: Color = color!(0x888888);
pub const TODAY: Color = color!(0x4caf50);
const UPCOMING: Color = color!(0x2196f3);
const CONTROL: Color = color!(0xe9e9ed);
const CONTROL_PRESSED: Color = color!(0xd0d0d7);
const CONTROL_BORDER: Color = color!(0x8f8f9d);
pub const BOLD: Font = Font {
    weight: font::Weight::Bold,
    ..Font::DEFAULT
};

/// Fluid grid cell bound; cells shrink to ~3/4 of it, like CSS `minmax(280px, 1fr)`.
const CARD_WIDTH: f32 = 360.0;
const GAP: f32 = 16.0;

pub fn theme() -> Theme {
    // ponytail: primary is the browser's button grey, so every default button matches the web UI.
    Theme::custom(
        "Publicator",
        Palette {
            background: color!(0x1a1a1a),
            text: color!(0xeeeeee),
            primary: CONTROL,
            success: TODAY,
            warning: WARNING,
            danger: ERROR,
        },
    )
}

/// The last scrolled viewport of a tab's list.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub offset: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for Viewport {
    /// The initial window's list area, until the list reports its own viewport.
    fn default() -> Self {
        Self {
            offset: 0.0,
            width: 1060.0,
            height: 600.0,
        }
    }
}

impl From<iced::widget::scrollable::Viewport> for Viewport {
    fn from(viewport: iced::widget::scrollable::Viewport) -> Self {
        Self {
            offset: viewport.absolute_offset().y,
            width: viewport.bounds().width,
            height: viewport.bounds().height,
        }
    }
}

/// Columns `cards` lays out at `width` (same formula as `grid::fluid`).
pub fn columns(width: f32) -> usize {
    (((width + GAP) / (CARD_WIDTH + GAP)).ceil() as usize).max(1)
}

/// Item indices visible in a `columns`-wide list of rows about `row_height` tall, plus one row.
// ponytail: estimated row height; measure real rows if thumbnails visibly stall.
pub fn visible_range(
    viewport: Viewport,
    columns: usize,
    row_height: f32,
    len: usize,
) -> Range<usize> {
    let first = (viewport.offset.max(0.0) / row_height) as usize;
    let last =
        ((viewport.offset.max(0.0) + viewport.height.max(0.0)) / row_height).ceil() as usize + 1;
    (first * columns).min(len)..(last * columns).min(len)
}

/// Cards in a responsive grid, as many columns as fit.
pub fn cards<'a>(children: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    grid(children)
        .fluid(CARD_WIDTH)
        .spacing(GAP)
        .height(grid::Sizing::EvenlyDistribute(Length::Shrink))
        .into()
}

pub fn card<'a>(content: impl Into<Element<'a, Message>>) -> Container<'a, Message> {
    container(content)
        .padding(12)
        .width(Length::Fill)
        .style(|_| boxed(color!(0x2a2a2a), Border::default().rounded(8)))
}

/// A thumbnail at most `height` tall, or the reason it is missing.
pub fn art<'a>(
    preview: Option<&Option<Result<std::path::PathBuf, String>>>,
    missing: bool,
    height: f32,
) -> Element<'a, Message> {
    match preview {
        Some(Some(Ok(path))) => thumbnail(path, height),
        Some(Some(Err(_))) if missing => text("Artwork missing").color(MUTED).into(),
        Some(Some(Err(error))) => text(format!("Preview unavailable: {error}"))
            .color(MUTED)
            .into(),
        _ => text("Loading preview…").color(MUTED).into(),
    }
}

/// Like the web's `max-width: 100%; max-height`: scaled down to fit, never up.
fn thumbnail<'a>(path: &Path, height: f32) -> Element<'a, Message> {
    container(
        image(image::Handle::from_path(path))
            .expand(true)
            .content_fit(ContentFit::ScaleDown),
    )
    .max_height(height)
    .into()
}

/// A full-width button, as the web cards stretched theirs.
pub fn wide<'a>(label: impl text::IntoFragment<'a>) -> Button<'a, Message> {
    button(text(label).center().width(Length::Fill)).width(Length::Fill)
}

pub fn warning<'a>(content: impl text::IntoFragment<'a>) -> Element<'a, Message> {
    text(content).color(WARNING).into()
}

fn boxed(background: Color, border: Border) -> container::Style {
    container::Style {
        background: Some(background.into()),
        border,
        ..container::Style::default()
    }
}

pub fn header(_: &Theme) -> container::Style {
    boxed(color!(0x111111), Border::default())
}

/// A bordered table cell (Publish).
pub fn cell(_: &Theme) -> container::Style {
    container::Style {
        border: Border::default().width(1).color(color!(0x555555)),
        ..container::Style::default()
    }
}

/// The amber banner: lineage hints, nearest prompts, pending confirmations.
pub fn notice(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(color!(0x7a5b00)),
        ..boxed(
            color!(0xfff3cd),
            Border::default().rounded(3).width(1).color(WARNING),
        )
    }
}

pub fn prompt_exact(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(color!(0x2c4a2c)),
        ..boxed(color!(0xeef4ee), Border::default().rounded(3))
    }
}

pub fn prompt_text(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(color!(0x222222)),
        ..boxed(color!(0xf7f7f7), Border::default().rounded(3))
    }
}

pub fn day(today: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let border = if today {
            Border::default().width(2).color(TODAY)
        } else {
            Border::default().width(1).color(color!(0x333333))
        };
        boxed(color!(0x222222), border)
    }
}

pub fn other_day(_: &Theme) -> container::Style {
    boxed(
        color!(0x1a1a1a),
        Border::default().width(1).color(color!(0x262626)),
    )
}

/// Browser buttons; the active tab or choice looks pressed and outlined.
pub fn tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if !active {
            return button::primary(theme, status);
        }
        button::Style {
            background: Some(CONTROL_PRESSED.into()),
            border: Border::default().rounded(4).width(2).color(ACCENT),
            ..button::primary(theme, button::Status::Active)
        }
    }
}

/// A calendar event chip: upcoming outlined blue, past dimmed.
pub fn event(upcoming: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: Some(
            if status == button::Status::Hovered {
                color!(0x333333)
            } else {
                color!(0x2a2a2a)
            }
            .into(),
        ),
        text_color: if upcoming {
            color!(0xcccccc)
        } else {
            color!(0x777777)
        },
        border: Border::default()
            .rounded(3)
            .width(if upcoming { 2 } else { 0 })
            .color(UPCOMING),
        ..button::Style::default()
    }
}

fn field_border(focused: bool) -> Border {
    Border::default()
        .rounded(4)
        .width(1)
        .color(if focused { ACCENT } else { CONTROL_BORDER })
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(if status == text_input::Status::Disabled {
            CONTROL
        } else {
            Color::WHITE
        }),
        border: field_border(matches!(status, text_input::Status::Focused { .. })),
        icon: Color::BLACK,
        placeholder: color!(0x6d6d6d),
        value: Color::BLACK,
        selection: color!(0xb3d4fc),
    }
}

pub fn editor(_: &Theme, status: text_editor::Status) -> text_editor::Style {
    text_editor::Style {
        background: Background::Color(Color::WHITE),
        border: field_border(matches!(status, text_editor::Status::Focused { .. })),
        placeholder: color!(0x6d6d6d),
        value: Color::BLACK,
        selection: color!(0xb3d4fc),
    }
}

pub fn select(_: &Theme, status: pick_list::Status) -> pick_list::Style {
    pick_list::Style {
        text_color: Color::BLACK,
        placeholder_color: color!(0x6d6d6d),
        handle_color: Color::BLACK,
        background: Background::Color(if status == pick_list::Status::Hovered {
            CONTROL_PRESSED
        } else {
            CONTROL
        }),
        border: Border::default().rounded(4).width(1).color(CONTROL_BORDER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(offset: f32, width: f32, height: f32) -> Viewport {
        Viewport {
            offset,
            width,
            height,
        }
    }

    #[test]
    fn visible_range_follows_grid_columns_and_clamps_to_the_list() {
        assert_eq!(columns(300.0), 1);
        assert_eq!(columns(1060.0), 3);
        assert_eq!(columns(1900.0), 6);
        // One column: rows 0..=2 visible, plus one.
        assert_eq!(visible_range(at(0.0, 300.0, 600.0), 1, 300.0, 50), 0..3);
        // Three columns, scrolled two rows down: rows 2..=4 plus one.
        assert_eq!(visible_range(at(600.0, 1060.0, 600.0), 3, 300.0, 50), 6..15);
        // A wider window after resize covers more items for the same rows.
        assert_eq!(
            visible_range(at(600.0, 1900.0, 600.0), 6, 300.0, 50),
            12..30
        );
        // The last partial row clamps to the list length.
        assert_eq!(visible_range(at(900.0, 1060.0, 600.0), 3, 300.0, 10), 9..10);
    }
}
