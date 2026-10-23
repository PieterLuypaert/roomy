use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::{DefaultTerminal, Frame};

use crate::format;
use crate::scan::{self, Progress};
use crate::tree::{Kind, Node};

const TICK: Duration = Duration::from_millis(100);
// Clicks, wheel and drags in SGR form, without the any-motion tracking that floods redraws.
const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const PARTIAL: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
const ACCENT: Color = Color::Indexed(75);
const DIM: Color = Color::Indexed(244);
const WARN: Color = Color::Indexed(203);
const TRACK: Color = Color::Indexed(240);
const SELECTION: Style = Style::new()
    .fg(Color::Indexed(255))
    .bg(Color::Indexed(237))
    .add_modifier(Modifier::BOLD);
const BADGE: Style = Style::new()
    .fg(Color::Black)
    .bg(ACCENT)
    .add_modifier(Modifier::BOLD);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sort {
    Size,
    Name,
    Items,
}

impl Sort {
    fn next(self) -> Self {
        match self {
            Sort::Size => Sort::Name,
            Sort::Name => Sort::Items,
            Sort::Items => Sort::Size,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Sort::Size => "size",
            Sort::Name => "name",
            Sort::Items => "items",
        }
    }

    fn order(self, node: &Node) -> Vec<usize> {
        let mut rows: Vec<usize> = (0..node.children.len()).collect();
        let children = &node.children;
        match self {
            Sort::Size => {}
            Sort::Name => rows.sort_by_cached_key(|&i| children[i].label().to_lowercase()),
            Sort::Items => rows.sort_by_key(|&i| std::cmp::Reverse(children[i].items)),
        }
        rows
    }
}

pub struct Browser {
    root: Node,
    root_path: PathBuf,
    trail: Vec<usize>,
    rows: Vec<usize>,
    cursor: usize,
    offset: usize,
    sort: Sort,
    list: Rect,
    denied: u64,
    unreadable: u64,
    status: Option<String>,
}

impl Browser {
    pub fn new(root: Node, root_path: PathBuf) -> Self {
        let denied = root.count_where(|node| node.is_denied);
        let unreadable = root.count_where(|node| node.is_unreadable);
        let mut browser = Browser {
            root,
            root_path,
            trail: Vec::new(),
            rows: Vec::new(),
            cursor: 0,
            offset: 0,
            sort: Sort::Size,
            list: Rect::default(),
            denied,
            unreadable,
            status: None,
        };
        browser.show(None);
        browser
    }

    pub fn current(&self) -> &Node {
        self.root.descend(&self.trail)
    }

    pub fn selected(&self) -> Option<&Node> {
        let child = *self.rows.get(self.cursor)?;
        self.current().children.get(child)
    }

    pub fn path(&self) -> PathBuf {
        let mut path = self.root_path.clone();
        let mut node = &self.root;
        for &child in &self.trail {
            node = &node.children[child];
            path.push(&*node.name);
        }
        path
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub fn handle(&mut self, event: &Event) -> Outcome {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                self.status = None;
                self.on_key(*key)
            }
            Event::Mouse(mouse) => {
                self.on_mouse(*mouse);
                Outcome::Continue
            }
            _ => Outcome::Continue,
        }
    }

    pub fn open(&mut self) {
        let Some(child) = self.selected_child() else {
            return;
        };
        let node = &self.current().children[child];
        if let Some(reason) = refusal(node) {
            self.status = Some(reason);
        } else if node.is_dir() {
            self.trail.push(child);
            self.show(None);
        }
    }

    pub fn back(&mut self) {
        if let Some(child) = self.trail.pop() {
            self.show(Some(child));
        }
    }

    fn selected_child(&self) -> Option<usize> {
        self.rows.get(self.cursor).copied()
    }

    fn show(&mut self, focus: Option<usize>) {
        self.rows = self.sort.order(self.current());
        self.cursor = focus
            .and_then(|child| self.rows.iter().position(|&row| row == child))
            .unwrap_or(0);
        self.offset = 0;
    }

    fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        let focus = self.selected_child();
        self.show(focus);
    }

    fn move_by(&mut self, delta: isize) {
        let Some(last) = self.rows.len().checked_sub(1) else {
            return;
        };
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
    }

    fn on_key(&mut self, key: KeyEvent) -> Outcome {
        let page = self.list.height.max(2) as isize - 1;
        match key.code {
            KeyCode::Char('q') => return Outcome::Quit,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Outcome::Quit;
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::PageUp => self.move_by(-page),
            KeyCode::PageDown | KeyCode::Char(' ') => self.move_by(page),
            KeyCode::Home | KeyCode::Char('g') => self.move_by(isize::MIN),
            KeyCode::End | KeyCode::Char('G') => self.move_by(isize::MAX),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => self.open(),
            KeyCode::Left | KeyCode::Backspace | KeyCode::Esc | KeyCode::Char('h') => self.back(),
            KeyCode::Char('s') => self.cycle_sort(),
            _ => {}
        }
        Outcome::Continue
    }

    fn on_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollUp => self.move_by(-3),
            MouseEventKind::ScrollDown => self.move_by(3),
            MouseEventKind::Down(MouseButton::Left) => self.click(mouse.column, mouse.row),
            MouseEventKind::Down(MouseButton::Right) => self.back(),
            _ => {}
        }
    }

    fn click(&mut self, column: u16, row: u16) {
        if !self.list.contains(Position::new(column, row)) {
            return;
        }
        let index = self.offset + usize::from(row - self.list.y);
        if index >= self.rows.len() {
            return;
        }
        if self.cursor == index {
            self.open();
        } else {
            self.cursor = index;
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let [header, _, list, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        self.list = list;
        self.render_header(frame, header);
        self.render_rows(frame, list);
        self.render_footer(frame, footer);
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let current = self.current();
        let total = format!(
            "{} · {} ",
            format::size(current.size),
            format::items(current.items)
        );
        let [left, right] =
            Layout::horizontal([Constraint::Fill(1), Constraint::Length(width(&total))])
                .areas(area);
        let room = usize::from(left.width).saturating_sub(9);
        let path = fit_start(&format::path(&self.path()), room);
        let title = Line::from(vec![
            Span::styled(" ruim ", BADGE),
            Span::raw("  "),
            Span::styled(path, Style::new().add_modifier(Modifier::BOLD)),
        ]);
        frame.render_widget(Paragraph::new(title), left);
        frame.render_widget(Paragraph::new(total).right_aligned(), right);
    }

    fn render_rows(&mut self, frame: &mut Frame, area: Rect) {
        let current = self.root.descend(&self.trail);
        if current.children.is_empty() {
            let note = match current {
                n if n.is_denied => "No access",
                n if n.is_unreadable => "Could not be read",
                _ => "Empty folder",
            };
            frame.render_widget(Paragraph::new(note).centered().style(DIM), area);
            return;
        }
        let height = usize::from(area.height);
        if height == 0 {
            return;
        }
        self.offset = scrolled(self.offset, self.cursor, height);
        let end = (self.offset + height).min(self.rows.len());
        let columns = Columns::fit(area.width, current);
        let rows = self.rows[self.offset..end]
            .iter()
            .map(|&child| row(&current.children[child], current.size, &columns));
        let table = Table::new(rows, columns.widths())
            .column_spacing(1)
            .row_highlight_style(SELECTION);
        let mut state = TableState::default().with_selected(self.cursor - self.offset);
        frame.render_stateful_widget(table, area, &mut state);
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        if let Some(status) = &self.status {
            let line = Span::styled(format!(" {status}"), Style::new().fg(WARN));
            frame.render_widget(Paragraph::new(line), area);
            return;
        }
        let sort = format!("sort: {}", self.sort.label());
        let keys = [
            ("↑↓", "move"),
            ("→", "open"),
            ("←", "back"),
            ("s", sort.as_str()),
            ("q", "quit"),
        ];
        let mut spans = Vec::new();
        for (key, action) in keys {
            spans.push(Span::styled(
                format!(" {key} "),
                Style::new().add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(format!("{action} "), Style::new().fg(DIM)));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        let note = Span::styled(self.trouble(), Style::new().fg(DIM));
        frame.render_widget(Paragraph::new(note).right_aligned(), area);
    }

    fn trouble(&self) -> String {
        let mut parts = Vec::new();
        if self.denied > 0 {
            parts.push(format!("{} no access", format::count(self.denied)));
        }
        if self.unreadable > 0 {
            parts.push(format!("{} unreadable", format::count(self.unreadable)));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("{} ", parts.join(" · "))
        }
    }
}

fn refusal(node: &Node) -> Option<String> {
    let name = node.label();
    if node.is_denied {
        Some(format!("No access to {name}. {}", format::ACCESS_HINT))
    } else if node.is_unreadable && node.children.is_empty() {
        Some(format!("{name} could not be read."))
    } else if node.is_mount {
        Some(format!(
            "{name} is another disk. Run ruim on it to look inside."
        ))
    } else {
        None
    }
}

struct Columns {
    bar: u16,
    items: u16,
}

impl Columns {
    fn fit(width: u16, node: &Node) -> Self {
        let bar = match width {
            w if w >= 110 => 30,
            w if w >= 80 => 20,
            w if w >= 60 => 10,
            _ => 0,
        };
        let most = node
            .children
            .iter()
            .map(|child| child.items)
            .max()
            .unwrap_or(0);
        let items = if bar == 0 || most == 0 {
            0
        } else {
            self::width(&format::items(most))
        };
        Columns { bar, items }
    }

    fn widths(&self) -> [Constraint; 5] {
        [
            Constraint::Length(9),
            Constraint::Length(4),
            Constraint::Length(self.bar),
            Constraint::Fill(1),
            Constraint::Length(self.items),
        ]
    }
}

fn row<'a>(node: &'a Node, parent: u64, columns: &Columns) -> Row<'a> {
    let share = format::share(node.size, parent);
    let items = if columns.items > 0 && node.items > 0 {
        format::items(node.items)
    } else {
        String::new()
    };
    let dim = Style::new().fg(DIM);
    Row::new([
        Cell::from(Line::from(format::size(node.size)).right_aligned()),
        Cell::from(Line::from(format::percent(share)).right_aligned()).style(dim),
        Cell::from(bar_line(share, columns.bar)),
        Cell::from(name(node)),
        Cell::from(Line::from(items).right_aligned()).style(dim),
    ])
}

fn bar_line(share: f64, width: u16) -> Line<'static> {
    let filled = bar(share, width);
    let rest = usize::from(width).saturating_sub(filled.chars().count());
    Line::from(vec![
        Span::styled(filled, Style::new().fg(heat(share))),
        Span::styled("·".repeat(rest), Style::new().fg(TRACK)),
    ])
}

fn name(node: &Node) -> Line<'_> {
    let label = node.label();
    let slash = if node.is_dir() { "/" } else { "" };
    let dir = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
    let warn = Style::new().fg(WARN);
    let note = |text: &'static str| Span::styled(text, warn.add_modifier(Modifier::DIM));
    match node.kind {
        _ if node.is_denied => Line::from(vec![
            Span::styled(format!("{label}{slash}"), warn),
            note("  no access"),
        ]),
        _ if node.is_unreadable && node.children.is_empty() => Line::from(vec![
            Span::styled(format!("{label}{slash}"), warn),
            note("  unreadable"),
        ]),
        _ if node.is_unreadable => Line::from(vec![
            Span::styled(format!("{label}/"), dir),
            note("  partly unreadable"),
        ]),
        _ if node.is_mount => Line::from(vec![
            Span::styled(format!("{label}/"), dir),
            Span::styled("  other disk", Style::new().fg(DIM)),
        ]),
        Kind::Dir => Line::from(Span::styled(format!("{label}/"), dir)),
        Kind::Link => Line::from(vec![
            Span::raw(label),
            Span::styled(" →", Style::new().fg(DIM)),
        ]),
        Kind::File | Kind::Other => Line::from(label),
    }
}

fn scrolled(offset: usize, cursor: usize, height: usize) -> usize {
    if cursor < offset {
        cursor
    } else if cursor >= offset + height {
        cursor + 1 - height
    } else {
        offset
    }
}

fn bar(share: f64, width: u16) -> String {
    let eighths = (share.clamp(0.0, 1.0) * f64::from(width) * 8.0).round() as usize;
    let mut bar = "█".repeat(eighths / 8);
    if !eighths.is_multiple_of(8) {
        bar.push(PARTIAL[eighths % 8]);
    }
    bar
}

fn heat(share: f64) -> Color {
    match share {
        s if s >= 0.5 => Color::Indexed(203),
        s if s >= 0.2 => Color::Indexed(209),
        s if s >= 0.05 => Color::Indexed(221),
        _ => Color::Indexed(110),
    }
}

fn width(text: &str) -> u16 {
    u16::try_from(text.chars().count()).unwrap_or(u16::MAX)
}

fn fit_start(text: &str, room: usize) -> String {
    let length = text.chars().count();
    if length <= room {
        return text.to_string();
    }
    let tail: String = text.chars().skip(length + 1 - room.max(1)).collect();
    format!("…{tail}")
}

pub fn run(path: PathBuf) -> io::Result<()> {
    let progress = Arc::new(Progress::default());
    let scanner = spawn_scan(path.clone(), Arc::clone(&progress));
    let mut terminal = start()?;
    let result = session(&mut terminal, path, &progress, scanner);
    stop();
    result
}

fn spawn_scan(path: PathBuf, progress: Arc<Progress>) -> JoinHandle<io::Result<Node>> {
    thread::spawn(move || scan::scan(&path, &progress))
}

fn session(
    terminal: &mut DefaultTerminal,
    path: PathBuf,
    progress: &Progress,
    scanner: JoinHandle<io::Result<Node>>,
) -> io::Result<()> {
    let label = format::path(&path);
    let Some(root) = wait(terminal, &label, progress, scanner)? else {
        return Ok(());
    };
    let mut browser = Browser::new(root, path);
    loop {
        terminal.draw(|frame| browser.render(frame))?;
        if browser.handle(&next_event()?) == Outcome::Quit {
            return Ok(());
        }
    }
}

fn next_event() -> io::Result<Event> {
    loop {
        let event = event::read()?;
        let is_motion = matches!(
            event,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved | MouseEventKind::Drag(_),
                ..
            })
        );
        if !is_motion {
            return Ok(event);
        }
    }
}

fn wait(
    terminal: &mut DefaultTerminal,
    label: &str,
    progress: &Progress,
    scanner: JoinHandle<io::Result<Node>>,
) -> io::Result<Option<Node>> {
    let started = Instant::now();
    while !scanner.is_finished() {
        terminal.draw(|frame| scanning(frame, label, progress, started.elapsed()))?;
        if event::poll(TICK)? && is_quit(&event::read()?) {
            progress.cancel();
            return Ok(None);
        }
    }
    let scanned = scanner
        .join()
        .map_err(|_| io::Error::other("the scan stopped unexpectedly"))?;
    scanned.map(Some)
}

fn is_quit(event: &Event) -> bool {
    let Event::Key(key) = event else {
        return false;
    };
    let is_ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
    key.kind == KeyEventKind::Press
        && (is_ctrl_c || matches!(key.code, KeyCode::Char('q') | KeyCode::Esc))
}

fn scanning(frame: &mut Frame, label: &str, progress: &Progress, elapsed: Duration) {
    let spinner = SPINNER[(elapsed.as_millis() / TICK.as_millis()) as usize % SPINNER.len()];
    let room = usize::from(frame.area().width).saturating_sub(4);
    let counts = format!(
        "{} · {} · {:.1} s",
        format::items(progress.items()),
        format::size(progress.bytes()),
        elapsed.as_secs_f64()
    );
    let lines = vec![
        Line::from(vec![
            Span::styled(" ruim ", BADGE),
            Span::raw(format!("  scanning {label} {spinner}")),
        ]),
        Line::raw(""),
        Line::raw(counts),
        Line::styled(
            fit_start(&format::path(&progress.current()), room),
            Style::new().fg(DIM),
        ),
        Line::raw(""),
        Line::styled("q to stop", Style::new().fg(DIM)),
    ];
    let [area] = Layout::vertical([Constraint::Length(lines.len() as u16)])
        .flex(Flex::Center)
        .areas(frame.area());
    frame.render_widget(Paragraph::new(lines).centered(), area);
}

fn start() -> io::Result<DefaultTerminal> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = mouse(MOUSE_OFF);
        previous(info);
    }));
    let terminal = ratatui::try_init()?;
    if let Err(err) = mouse(MOUSE_ON) {
        ratatui::restore();
        return Err(err);
    }
    Ok(terminal)
}

fn stop() {
    let released = mouse(MOUSE_OFF);
    ratatui::restore();
    if let Err(err) = released {
        eprintln!("ruim: could not release the mouse: {err}");
    }
}

fn mouse(sequence: &str) -> io::Result<()> {
    let mut out = io::stdout();
    out.write_all(sequence.as_bytes())?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_use_eighth_blocks() {
        assert_eq!(bar(0.0, 10), "");
        assert_eq!(bar(0.5, 10), "█████");
        assert_eq!(bar(0.55, 10), "█████▌");
        assert_eq!(bar(2.0, 4), "████");
    }

    #[test]
    fn fit_start_keeps_the_end_of_long_paths() {
        assert_eq!(fit_start("~/a/b", 10), "~/a/b");
        assert_eq!(fit_start("~/Documents/deep/folder", 10), "…ep/folder");
        assert_eq!(fit_start("~/Documents/deep/folder", 10).chars().count(), 10);
    }

    #[test]
    fn scrolling_keeps_the_cursor_in_view() {
        assert_eq!(scrolled(0, 3, 10), 0);
        assert_eq!(scrolled(0, 10, 10), 1);
        assert_eq!(scrolled(5, 2, 10), 2);
        assert_eq!(scrolled(5, 14, 10), 5);
    }
}
