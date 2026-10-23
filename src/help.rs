use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Padding, Paragraph};

use crate::theme::{self, ACCENT, DIM};
use crate::ui;

/// The columns between the two halves when the keys need two columns.
const GAP: u16 = 4;

/// A group of keys that do related things, with what each one does.
pub(crate) struct Section {
    pub(crate) title: &'static str,
    pub(crate) keys: &'static [(&'static [&'static str], &'static str)],
}

const MOVE: Section = Section {
    title: "Move",
    keys: &[
        (&["↑", "↓", "j", "k"], "one row"),
        (&["PgUp", "PgDn", "space"], "one page"),
        (&["g", "G", "Home", "End"], "first or last row"),
        (&["click"], "select, and open with a second click"),
    ],
};

const ACT: Section = Section {
    title: "On the selected row",
    keys: &[
        (&["o"], "show it in the Finder"),
        (&["y"], "copy its path"),
        (&["d"], "move it to the Trash, after asking"),
    ],
};

const GENERAL: Section = Section {
    title: "",
    keys: &[(&["?"], "these keys"), (&["q"], "quit")],
};

pub(crate) const BROWSER: &[Section] = &[
    MOVE,
    Section {
        title: "Folders",
        keys: &[
            (&["→", "enter", "l"], "open"),
            (&["←", "esc", "h"], "back"),
            (&["s"], "sort by size, name or items"),
        ],
    },
    ACT,
    GENERAL,
];

pub(crate) const OVERVIEW: &[Section] = &[
    MOVE,
    Section {
        title: "The disk",
        keys: &[
            (&["→", "enter", "l"], "open a group"),
            (&["b"], "browse the whole disk"),
        ],
    },
    GENERAL,
];

pub(crate) const PLACES: &[Section] = &[
    MOVE,
    Section {
        title: "Places",
        keys: &[
            (&["→", "enter", "l"], "open a place"),
            (&["←", "esc", "h"], "back to the overview"),
            (&["b"], "browse the whole disk"),
        ],
    },
    ACT,
    GENERAL,
];

/// Shows every key of a screen in a panel in its middle, in two columns when one is too
/// tall for the screen; any key closes it.
pub(crate) fn render(frame: &mut Frame, sections: &[Section]) {
    let keys_width = sections
        .iter()
        .flat_map(|section| section.keys)
        .map(|(keys, _)| keycaps_width(keys))
        .max()
        .unwrap_or(0);
    let area = frame.area();
    let one = lines(sections, keys_width);
    let columns = if one.len() as u16 + 2 > area.height {
        let (left, right) = sections.split_at(sections.len().div_ceil(2));
        vec![lines(left, keys_width), lines(right, keys_width)]
    } else {
        vec![one]
    };
    let widest = columns.iter().flatten().map(Line::width).max().unwrap_or(0) as u16;
    let count = columns.len() as u16;
    let tallest = columns.iter().map(Vec::len).max().unwrap_or(0) as u16;
    let width = (widest * count + GAP * (count - 1) + 6).min(area.width.saturating_sub(2));
    let height = (tallest + 2).min(area.height);
    let [row] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [popup] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(row);
    let block = theme::panel("All keys")
        .title_bottom(theme::aside("any key closes"))
        .padding(Padding::horizontal(2));
    frame.render_widget(Clear, popup);
    let inner = ui::framed(frame, popup, block);
    let slots = Layout::horizontal(vec![Constraint::Length(widest); columns.len()])
        .spacing(GAP)
        .split(inner);
    for (column, slot) in columns.into_iter().zip(slots.iter()) {
        frame.render_widget(Paragraph::new(column), *slot);
    }
}

fn lines(sections: &[Section], keys_width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for section in sections {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        if !section.title.is_empty() {
            let title = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
            lines.push(Line::styled(section.title, title));
        }
        for (keys, what) in section.keys {
            let mut spans = keycaps(keys);
            let gap = keys_width - keycaps_width(keys) + 2;
            spans.push(Span::raw(" ".repeat(gap)));
            spans.push(Span::styled(*what, Style::new().fg(DIM)));
            lines.push(Line::from(spans));
        }
    }
    lines
}

fn keycaps(keys: &[&str]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, key) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(theme::keycap(key));
    }
    spans
}

fn keycaps_width(keys: &[&str]) -> usize {
    let caps: usize = keys.iter().map(|key| key.chars().count() + 2).sum();
    caps + keys.len().saturating_sub(1)
}
