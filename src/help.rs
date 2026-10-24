use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Padding, Paragraph};

use crate::theme::{self, ACCENT, DIM};
use crate::ui;

/// The columns between the two halves when the keys need two columns.
const GAP: u16 = 3;

/// A group of keys that do related things, with what each one does.
pub(crate) struct Section {
    pub(crate) title: &'static str,
    pub(crate) keys: &'static [(&'static [&'static str], &'static str)],
}

const MOVE: Section = Section {
    title: "Move",
    keys: &[
        (&["↑", "↓", "j", "k"], "up and down"),
        (&["PgDn", "space"], "next page"),
        (&["PgUp"], "previous page"),
        (&["g", "Home"], "first row"),
        (&["G", "End"], "last row"),
        (&["click"], "select; again opens"),
        (&["wheel"], "scroll"),
    ],
};

const ACT: Section = Section {
    title: "Selected row",
    keys: &[
        (&["o"], "show in the Finder"),
        (&["y"], "copy the path"),
        (&["d", "⌦"], "trash, after asking"),
    ],
};

const GENERAL: Section = Section {
    title: "",
    keys: &[(&["?"], "these keys"), (&["q", "^C"], "quit")],
};

pub(crate) const BROWSER: &[Section] = &[
    MOVE,
    Section {
        title: "Folders",
        keys: &[
            (&["→", "enter", "l"], "open"),
            (&["←", "esc", "h", "⌫"], "back; or right-click"),
            (&["s"], "change the order"),
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
            (&["←", "esc", "h", "⌫"], "back; or right-click"),
            (&["b"], "browse the whole disk"),
        ],
    },
    ACT,
    GENERAL,
];

/// Shows every key of a screen in a panel in its middle, in two columns when one is too
/// tall for the screen; any key closes it.
pub(crate) fn render(frame: &mut Frame, sections: &[Section]) {
    let area = frame.area();
    let columns = if height(sections) + 2 > area.height {
        let (left, right) = sections.split_at(balance(sections));
        vec![left, right]
    } else {
        vec![sections]
    };
    let keys = keys_width(sections);
    let columns: Vec<Vec<Line>> = columns
        .into_iter()
        .map(|column| lines(column, keys))
        .collect();
    let widest = columns.iter().flatten().map(Line::width).max().unwrap_or(0) as u16;
    let tallest = columns.iter().map(Vec::len).max().unwrap_or(0) as u16;
    let count = columns.len() as u16;
    let width = widest * count + GAP * (count - 1) + 6;
    let popup = ui::centered(area, width, tallest + 2);
    let note = if tallest + 2 > popup.height {
        "a taller window shows them all"
    } else {
        "any key closes"
    };
    let block = theme::panel("All keys")
        .title_bottom(theme::aside(note))
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

/// Where to split the sections so that the taller of the two columns is as short as it gets.
fn balance(sections: &[Section]) -> usize {
    (1..sections.len())
        .min_by_key(|&at| height(&sections[..at]).max(height(&sections[at..])))
        .unwrap_or(sections.len())
}

fn height(sections: &[Section]) -> u16 {
    let rows: usize = sections
        .iter()
        .map(|section| section.keys.len() + usize::from(!section.title.is_empty()))
        .sum();
    (rows + sections.len().saturating_sub(1)) as u16
}

fn keys_width(sections: &[Section]) -> usize {
    sections
        .iter()
        .flat_map(|section| section.keys)
        .map(|(keys, _)| keycaps_width(keys))
        .max()
        .unwrap_or(0)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_columns_split_where_the_taller_one_is_shortest() {
        let at = balance(BROWSER);

        let (left, right) = BROWSER.split_at(at);
        assert_eq!(at, 1);
        assert_eq!((height(left), height(right)), (8, 12));
    }

    #[test]
    fn every_screen_fits_two_columns_in_ninety_columns() {
        for sections in [BROWSER, OVERVIEW, PLACES] {
            let keys = keys_width(sections);
            let widest = lines(sections, keys).iter().map(Line::width).max().unwrap();

            assert!(widest * 2 + usize::from(GAP) + 6 <= 90, "{widest}");
        }
    }
}
