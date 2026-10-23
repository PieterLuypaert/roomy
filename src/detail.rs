use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Padding, Paragraph};

use crate::format;
use crate::theme::{self, ACCENT, DIM, WARN};
use crate::tree::{Kind, Node};
use crate::ui;

/// Below this many columns the list keeps the whole width.
const MIN_WIDTH: u16 = 100;
/// The most entries a pane lists, more than any screen shows.
const MOST: usize = 64;

/// What the pane beside a list says about the selected entry.
pub(crate) struct Detail {
    pub(crate) title: String,
    pub(crate) size: u64,
    pub(crate) share: f64,
    /// What the entry is part of, as in "96% of Panenco".
    pub(crate) of: String,
    pub(crate) facts: Vec<Line<'static>>,
    pub(crate) about: String,
    pub(crate) heading: &'static str,
    pub(crate) inside: Vec<Entry>,
    pub(crate) whole: u64,
}

/// One of the biggest things inside the selected entry.
pub(crate) struct Entry {
    pub(crate) label: String,
    pub(crate) size: u64,
    pub(crate) style: Style,
}

/// Splits off the pane's side of `area`, unless the screen is too narrow for both.
pub(crate) fn split(area: Rect) -> (Rect, Option<Rect>) {
    if area.width < MIN_WIDTH {
        return (area, None);
    }
    let width = (area.width * 7 / 20).clamp(34, 56);
    let [list, pane] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]).areas(area);
    (list, Some(pane))
}

/// The pane for `node`, which takes `size` of `whole`, the folder or group called `of`.
pub(crate) fn of_node(node: &Node, size: u64, of: &str, whole: u64) -> Detail {
    let mut facts = vec![Line::styled(kind(node), Style::new().fg(DIM))];
    let mut about = String::new();
    if node.is_denied {
        facts.push(Line::styled("no access", Style::new().fg(WARN)));
        about = format::ACCESS_HINT.to_string();
    } else if node.is_mount {
        facts.push(Line::styled("another disk", Style::new().fg(DIM)));
    }
    let slash = if node.is_dir() { "/" } else { "" };
    Detail {
        title: format!("{}{slash}", node.label()),
        size,
        share: format::share(size, whole),
        of: of.to_string(),
        facts,
        about,
        heading: "biggest inside",
        inside: biggest(node),
        whole: node.size,
    }
}

/// "43.1 GB · 96% of Panenco", with the name cut to what fits in `width`.
fn amount(detail: &Detail, width: u16) -> Line<'static> {
    let size = format::size(detail.size);
    let percent = format::percent(detail.share);
    let mut spans = vec![Span::styled(size.clone(), theme::size(detail.size))];
    if !percent.is_empty() {
        let text = format!(" · {percent} of ");
        let used = size.chars().count() + text.chars().count();
        let of = ui::fit_end(&detail.of, usize::from(width).saturating_sub(used));
        spans.push(Span::styled(text + &of, Style::new().fg(DIM)));
    }
    Line::from(spans)
}

fn kind(node: &Node) -> String {
    match node.kind {
        Kind::Dir => format::items(node.items),
        Kind::File => "file".to_string(),
        Kind::Link => "symbolic link".to_string(),
        Kind::Other => "special file".to_string(),
    }
}

fn biggest(node: &Node) -> Vec<Entry> {
    let mut children: Vec<&Node> = node
        .children
        .iter()
        .filter(|child| !child.is_trashed)
        .collect();
    children.sort_by_key(|child| std::cmp::Reverse(child.size));
    children
        .into_iter()
        .take(MOST)
        .map(|child| Entry {
            label: entry_label(child),
            size: child.size,
            style: entry_style(child),
        })
        .collect()
}

fn entry_label(node: &Node) -> String {
    let slash = if node.is_dir() { "/" } else { "" };
    format!("{}{slash}", node.label())
}

fn entry_style(node: &Node) -> Style {
    if node.is_dir() {
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        theme::file(&node.name)
    }
}

pub(crate) fn render(frame: &mut Frame, area: Rect, detail: &Detail) {
    let title = ui::fit_end(&detail.title, usize::from(area.width.saturating_sub(6)));
    let block = theme::panel(&title).padding(Padding::horizontal(1));
    let inner = ui::framed(frame, area, block);
    let mut lines = vec![amount(detail, inner.width)];
    lines.extend(detail.facts.iter().cloned());
    if !detail.about.is_empty() {
        lines.push(Line::default());
        let about = ui::wrap(&detail.about, usize::from(inner.width));
        lines.extend(
            about
                .into_iter()
                .map(|line| Line::styled(line, Style::new().fg(DIM))),
        );
    }
    if !detail.inside.is_empty() {
        lines.push(Line::default());
        lines.push(Line::styled(detail.heading, Style::new().fg(DIM)));
        let room = usize::from(inner.height).saturating_sub(lines.len());
        let entries = detail.inside.iter().take(room);
        lines.extend(entries.map(|entry| entry_line(entry, detail.whole, inner.width)));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// "████████▌ node_modules/   31.0 GB"
fn entry_line(entry: &Entry, whole: u64, width: u16) -> Line<'static> {
    let share = format::share(entry.size, whole);
    let bar_width = usize::from((width / 4).clamp(4, 10));
    let size = format::size(entry.size);
    let room = usize::from(width).saturating_sub(bar_width + 2 + size.chars().count());
    let label = ui::fit_start(&entry.label, room);
    let gap = room.saturating_sub(label.chars().count()) + 1;
    Line::from(vec![
        Span::styled(
            format!("{:<bar_width$} ", ui::bar(share, bar_width as u16)),
            Style::new().fg(theme::heat(share)),
        ),
        Span::styled(label, entry.style),
        Span::raw(" ".repeat(gap)),
        Span::styled(size, theme::size(entry.size)),
    ])
}
