use std::io::{self, Write};
use std::ops::Range;
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

pub(crate) const TICK: Duration = Duration::from_millis(100);
// Clicks, wheel and drags in SGR form, without the any-motion tracking that floods redraws.
const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const PARTIAL: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
pub(crate) const ACCENT: Color = Color::Indexed(75);
pub(crate) const DIM: Color = Color::Indexed(244);
pub(crate) const WARN: Color = Color::Indexed(203);
pub(crate) const TRACK: Color = Color::Indexed(240);
pub(crate) const SELECTION: Style = Style::new()
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
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pointer {
    Nothing,
    Activate,
    Back,
}

#[derive(Debug, Default)]
pub(crate) struct Cursor {
    pub(crate) index: usize,
    offset: usize,
    area: Rect,
}

impl Cursor {
    pub(crate) fn reset(&mut self, index: usize) {
        self.index = index;
        self.offset = 0;
    }

    pub(crate) fn move_by(&mut self, delta: isize, len: usize) {
        if let Some(last) = len.checked_sub(1) {
            self.index = self.index.saturating_add_signed(delta).min(last);
        }
    }

    pub(crate) fn key(&mut self, code: KeyCode, len: usize) -> bool {
        let page = self.area.height.max(2) as isize - 1;
        let delta = match code {
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::PageUp => -page,
            KeyCode::PageDown | KeyCode::Char(' ') => page,
            KeyCode::Home | KeyCode::Char('g') => isize::MIN,
            KeyCode::End | KeyCode::Char('G') => isize::MAX,
            _ => return false,
        };
        self.move_by(delta, len);
        true
    }

    pub(crate) fn mouse(&mut self, mouse: MouseEvent, len: usize) -> Pointer {
        match mouse.kind {
            MouseEventKind::ScrollUp => self.move_by(-3, len),
            MouseEventKind::ScrollDown => self.move_by(3, len),
            MouseEventKind::Down(MouseButton::Right) => return Pointer::Back,
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(index) = self.hit(mouse.column, mouse.row, len) else {
                    return Pointer::Nothing;
                };
                if index == self.index {
                    return Pointer::Activate;
                }
                self.index = index;
            }
            _ => {}
        }
        Pointer::Nothing
    }

    fn hit(&self, column: u16, row: u16, len: usize) -> Option<usize> {
        if !self.area.contains(Position::new(column, row)) {
            return None;
        }
        let index = self.offset + usize::from(row - self.area.y);
        (index < len).then_some(index)
    }

    pub(crate) fn window(&mut self, area: Rect, len: usize) -> Range<usize> {
        self.area = area;
        let height = usize::from(area.height);
        if height == 0 || len == 0 {
            return 0..0;
        }
        self.offset = scrolled(self.offset, self.index.min(len - 1), height);
        self.offset..(self.offset + height).min(len)
    }

    pub(crate) fn state(&self) -> TableState {
        TableState::default().with_selected(self.index.saturating_sub(self.offset))
    }
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
    floor: usize,
    rows: Vec<usize>,
    cursor: Cursor,
    sort: Sort,
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
            floor: 0,
            rows: Vec::new(),
            cursor: Cursor::default(),
            sort: Sort::Size,
            denied,
            unreadable,
            status: None,
        };
        browser.show(None);
        browser
    }

    pub fn root(&self) -> &Node {
        &self.root
    }

    pub fn current(&self) -> &Node {
        self.root.descend(&self.trail)
    }

    pub fn selected(&self) -> Option<&Node> {
        let child = self.selected_child()?;
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

    /// Shows the folder at `trail`, or the folder holding it when it is a file or does not
    /// open; back stops there.
    pub fn jump(&mut self, mut trail: Vec<usize>) {
        let target = self.root.descend(&trail);
        let reason = refusal(target);
        let opens = trail.is_empty() || (target.is_dir() && reason.is_none());
        let focus = if opens { None } else { trail.pop() };
        self.status = if opens { None } else { reason };
        self.floor = trail.len();
        self.trail = trail;
        self.show(focus);
    }

    pub fn handle(&mut self, event: &Event) -> Outcome {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                self.status = None;
                self.on_key(*key)
            }
            Event::Mouse(mouse) => {
                if is_click(mouse) {
                    self.status = None;
                }
                match self.cursor.mouse(*mouse, self.rows.len()) {
                    Pointer::Activate => {
                        self.open();
                        Outcome::Continue
                    }
                    Pointer::Back => self.back(),
                    Pointer::Nothing => Outcome::Continue,
                }
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

    pub fn back(&mut self) -> Outcome {
        if self.trail.len() <= self.floor {
            return Outcome::Leave;
        }
        let child = self.trail.pop();
        self.show(child);
        Outcome::Continue
    }

    fn selected_child(&self) -> Option<usize> {
        self.rows.get(self.cursor.index).copied()
    }

    fn show(&mut self, focus: Option<usize>) {
        self.rows = self.sort.order(self.current());
        let index = focus
            .and_then(|child| self.rows.iter().position(|&row| row == child))
            .unwrap_or(0);
        self.cursor.reset(index);
    }

    fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        let focus = self.selected_child();
        self.show(focus);
    }

    fn on_key(&mut self, key: KeyEvent) -> Outcome {
        if is_quit_key(key) {
            return Outcome::Quit;
        }
        if self.cursor.key(key.code, self.rows.len()) {
            return Outcome::Continue;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => self.open(),
            KeyCode::Left | KeyCode::Backspace | KeyCode::Esc | KeyCode::Char('h') => {
                return self.back();
            }
            KeyCode::Char('s') => self.cycle_sort(),
            _ => {}
        }
        Outcome::Continue
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let [top, list, bottom] = screen(frame.area());
        let current = self.current();
        let total = format!(
            "{} · {}",
            format::size(current.size),
            format::items(current.items)
        );
        header(frame, top, &format::path(&self.path()), &total);
        self.render_rows(frame, list);
        let sort = format!("sort: {}", self.sort.label());
        let keys = [
            ("↑↓", "move"),
            ("→", "open"),
            ("←", "back"),
            ("s", sort.as_str()),
            ("q", "quit"),
        ];
        footer(
            frame,
            bottom,
            &keys,
            self.status.as_deref(),
            &self.trouble(),
        );
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
        let visible = self.cursor.window(area, self.rows.len());
        let columns = Columns::fit(area.width, current);
        let rows = self.rows[visible]
            .iter()
            .map(|&child| row(&current.children[child], current.size, &columns));
        let table = Table::new(rows, columns.widths())
            .column_spacing(1)
            .row_highlight_style(SELECTION);
        frame.render_stateful_widget(table, area, &mut self.cursor.state());
    }

    fn trouble(&self) -> String {
        let mut parts = Vec::new();
        if self.denied > 0 {
            parts.push(format!("{} no access", format::count(self.denied)));
        }
        if self.unreadable > 0 {
            parts.push(format!("{} unreadable", format::count(self.unreadable)));
        }
        parts.join(" · ")
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
        let bar = bar_width(width);
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

pub(crate) fn bar_width(width: u16) -> u16 {
    match width {
        w if w >= 110 => 30,
        w if w >= 80 => 20,
        w if w >= 60 => 10,
        _ => 0,
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

pub(crate) fn bar_line(share: f64, width: u16) -> Line<'static> {
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

pub(crate) fn width(text: &str) -> u16 {
    u16::try_from(text.chars().count()).unwrap_or(u16::MAX)
}

pub(crate) fn fit_start(text: &str, room: usize) -> String {
    let length = text.chars().count();
    if length <= room {
        return text.to_string();
    }
    let tail: String = text.chars().skip(length + 1 - room.max(1)).collect();
    format!("…{tail}")
}

pub(crate) fn screen(area: Rect) -> [Rect; 3] {
    let [top, _, list, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    [top, list, bottom]
}

pub(crate) fn header(frame: &mut Frame, area: Rect, title: &str, summary: &str) {
    let summary = format!("{summary} ");
    let [left, right] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(width(&summary))]).areas(area);
    let room = usize::from(left.width).saturating_sub(9);
    let line = Line::from(vec![
        Span::styled(" ruim ", BADGE),
        Span::raw("  "),
        Span::styled(
            fit_start(title, room),
            Style::new().add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), left);
    frame.render_widget(Paragraph::new(summary).right_aligned(), right);
}

pub(crate) fn footer(
    frame: &mut Frame,
    area: Rect,
    keys: &[(&str, &str)],
    status: Option<&str>,
    note: &str,
) {
    if let Some(status) = status {
        let line = Span::styled(format!(" {status}"), Style::new().fg(WARN));
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let mut spans = Vec::new();
    for (key, action) in keys {
        spans.push(Span::styled(
            format!(" {key} "),
            Style::new().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!("{action} "), Style::new().fg(DIM)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    if !note.is_empty() {
        let right = Span::styled(format!("{note} "), Style::new().fg(DIM));
        frame.render_widget(Paragraph::new(right).right_aligned(), area);
    }
}

pub fn run(path: PathBuf) -> io::Result<()> {
    let progress = Arc::new(Progress::default());
    let scanner = {
        let (path, progress) = (path.clone(), Arc::clone(&progress));
        thread::spawn(move || scan::scan(&path, &progress))
    };
    let mut terminal = start()?;
    let result = session(&mut terminal, path, &progress, scanner);
    stop();
    result
}

fn session(
    terminal: &mut DefaultTerminal,
    path: PathBuf,
    progress: &Progress,
    scanner: JoinHandle<io::Result<Node>>,
) -> io::Result<()> {
    let label = format::path(&path);
    let scanned = wait(terminal, progress, scanner, |frame, elapsed| {
        scanning(frame, frame.area(), &label, progress, elapsed);
    })?;
    let Some(root) = scanned else {
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

pub(crate) fn is_click(mouse: &MouseEvent) -> bool {
    matches!(mouse.kind, MouseEventKind::Down(_))
}

pub(crate) fn next_event() -> io::Result<Event> {
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

pub(crate) fn wait<T>(
    terminal: &mut DefaultTerminal,
    progress: &Progress,
    worker: JoinHandle<io::Result<T>>,
    mut draw: impl FnMut(&mut Frame, Duration),
) -> io::Result<Option<T>> {
    let started = Instant::now();
    while !worker.is_finished() {
        terminal.draw(|frame| draw(frame, started.elapsed()))?;
        if event::poll(TICK)? && is_quit(&event::read()?) {
            progress.cancel();
            return Ok(None);
        }
    }
    let result = worker
        .join()
        .map_err(|_| io::Error::other("the scan stopped unexpectedly"))?;
    result.map(Some)
}

fn is_quit(event: &Event) -> bool {
    let Event::Key(key) = event else {
        return false;
    };
    key.kind == KeyEventKind::Press && (is_quit_key(*key) || key.code == KeyCode::Esc)
}

pub(crate) fn is_quit_key(key: KeyEvent) -> bool {
    let is_ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
    is_ctrl_c || key.code == KeyCode::Char('q')
}

pub(crate) fn scanning(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    progress: &Progress,
    elapsed: Duration,
) {
    let has_badge = area.y == frame.area().y;
    let lines = progress_lines(label, progress, elapsed, area.width, has_badge);
    let [middle] = Layout::vertical([Constraint::Length(lines.len() as u16)])
        .flex(Flex::Center)
        .areas(area);
    frame.render_widget(Paragraph::new(lines).centered(), middle);
}

fn progress_lines(
    label: &str,
    progress: &Progress,
    elapsed: Duration,
    width: u16,
    has_badge: bool,
) -> Vec<Line<'static>> {
    let spinner = SPINNER[(elapsed.as_millis() / TICK.as_millis()) as usize % SPINNER.len()];
    let counts = format!(
        "{} · {} · {:.1} s",
        format::items(progress.items()),
        format::size(progress.bytes()),
        elapsed.as_secs_f64()
    );
    let current = fit_start(
        &format::path(&progress.current()),
        usize::from(width).saturating_sub(4),
    );
    vec![
        Line::from(vec![
            Span::styled(if has_badge { " ruim " } else { "" }, BADGE),
            Span::raw(format!("  scanning {label} {spinner}")),
        ]),
        Line::raw(""),
        Line::raw(counts),
        Line::styled(current, Style::new().fg(DIM)),
        Line::raw(""),
        Line::styled("q to stop", Style::new().fg(DIM)),
    ]
}

pub(crate) fn start() -> io::Result<DefaultTerminal> {
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

pub(crate) fn stop() {
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
