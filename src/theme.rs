use ratatui::style::{Color, Modifier, Style};

pub(crate) const ACCENT: Color = Color::Indexed(75);
pub(crate) const DIM: Color = Color::Indexed(244);
pub(crate) const WARN: Color = Color::Indexed(203);
pub(crate) const TRACK: Color = Color::Indexed(240);
pub(crate) const SELECTION: Style = Style::new()
    .fg(Color::Indexed(255))
    .bg(Color::Indexed(237))
    .add_modifier(Modifier::BOLD);
pub(crate) const BADGE: Style = Style::new()
    .fg(Color::Black)
    .bg(ACCENT)
    .add_modifier(Modifier::BOLD);

/// Brighter the more of its folder an entry takes.
pub(crate) fn heat(share: f64) -> Color {
    match share {
        s if s >= 0.5 => Color::Indexed(203),
        s if s >= 0.2 => Color::Indexed(209),
        s if s >= 0.05 => Color::Indexed(221),
        _ => Color::Indexed(110),
    }
}
