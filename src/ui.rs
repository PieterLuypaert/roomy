use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Cell, Clear, HighlightSpacing, Padding, Paragraph, Row, Table, TableState,
};
use ratatui::{DefaultTerminal, Frame};

use crate::actions::{self, Trasher};
use crate::detail::{self, Detail};
use crate::format;
use crate::help;
use crate::scan::{self, Progress};
use crate::theme::{self, ACCENT, BADGE, DIM, DONE, SELECTION, TRACK, WARN};
use crate::tree::{Kind, Node};
use crate::volumes;

pub(crate) const TICK: Duration = Duration::from_millis(100);
// Clicks, wheel and drags in SGR form, without the any-motion tracking that floods redraws.
const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const PARTIAL: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
const SEPARATOR: &str = " › ";
pub(crate) const STOP_KEYS: [(&str, &str); 1] = [("q", "stop")];
/// The key that shows all the others, which the key row keeps when it runs out of room.
const HELP_KEY: &str = "?";
const KEYS: [(&str, &str); 7] = [
    ("↑↓", "move"),
    ("→", "open"),
    ("←", "back"),
    ("s", "sort"),
    ("d", "trash"),
    (HELP_KEY, "all keys"),
    ("q", "quit"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    Quit,
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Done,
    Info,
    Problem,
}

/// A message that floats over the bottom of the panel until the next key or click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Toast {
    text: String,
    tone: Tone,
}

impl Toast {
    fn new(text: impl Into<String>, tone: Tone) -> Self {
        Toast {
            text: text.into(),
            tone,
        }
    }

    pub(crate) fn done(text: impl Into<String>) -> Self {
        Toast::new(text, Tone::Done)
    }

    pub(crate) fn info(text: impl Into<String>) -> Self {
        Toast::new(text, Tone::Info)
    }

    pub(crate) fn problem(text: impl Into<String>) -> Self {
        Toast::new(text, Tone::Problem)
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }
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
        let children = &node.children;
        let mut rows: Vec<usize> = (0..children.len())
            .filter(|&i| !children[i].is_trashed)
            .collect();
        match self {
            Sort::Size => rows.sort_by_key(|&i| std::cmp::Reverse(children[i].size)),
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
    toast: Option<Toast>,
    pending: Option<usize>,
    trasher: Trasher,
    trashed: u64,
    /// Crumbs for what lies above the floor, when the overview opened the browser.
    origin: Option<Vec<String>>,
    is_helping: bool,
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
            toast: None,
            pending: None,
            trasher: actions::trash,
            trashed: 0,
            origin: None,
            is_helping: false,
        };
        browser.show(None);
        browser
    }

    pub fn with_trasher(mut self, trasher: Trasher) -> Self {
        self.trasher = trasher;
        self
    }

    pub fn take_trashed(&mut self) -> u64 {
        std::mem::take(&mut self.trashed)
    }

    pub fn trash(&mut self, trail: &[usize], path: &Path) -> io::Result<u64> {
        let size = match self.root.get(trail) {
            Some(node) if !trail.is_empty() && !node.is_trashed => node.size,
            _ => return Err(io::Error::other("that entry is no longer there")),
        };
        (self.trasher)(path)?;
        self.root.trash(trail);
        if self.trail.starts_with(trail) {
            self.trail.truncate(trail.len() - 1);
            self.floor = self.floor.min(self.trail.len());
        }
        self.denied = self.root.count_where(|node| node.is_denied);
        self.unreadable = self.root.count_where(|node| node.is_unreadable);
        let index = self.cursor.index;
        self.show(None);
        self.cursor
            .reset(index.min(self.rows.len().saturating_sub(1)));
        Ok(size)
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
        self.toast.as_ref().map(Toast::text)
    }

    fn crumbs(&self) -> Vec<String> {
        let Some(origin) = &self.origin else {
            return path_crumbs(&format::path(&self.path()));
        };
        let start = self.floor.saturating_sub(1);
        let mut node = self.root.descend(&self.trail[..start]);
        let mut crumbs = origin.clone();
        for &child in &self.trail[start..] {
            node = &node.children[child];
            crumbs.push(node.label().into_owned());
        }
        crumbs
    }

    /// Shows the folder at `trail`, or the folder holding it when it is a file or does not
    /// open; back stops there. An `origin` heads the breadcrumbs in place of the path above.
    pub fn jump(&mut self, mut trail: Vec<usize>, origin: Option<Vec<String>>) {
        self.origin = origin;
        let target = self.root.descend(&trail);
        let reason = refusal(target);
        let opens = trail.is_empty() || (target.is_dir() && reason.is_none());
        let focus = if opens { None } else { trail.pop() };
        self.toast = if opens {
            None
        } else {
            reason.map(Toast::problem)
        };
        self.floor = trail.len();
        self.trail = trail;
        self.show(focus);
    }

    pub fn handle(&mut self, event: &Event) -> Outcome {
        if self.is_helping {
            return close_overlay(&mut self.is_helping, event);
        }
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                self.toast = None;
                self.on_key(*key)
            }
            Event::Mouse(_) if self.pending.is_some() => Outcome::Continue,
            Event::Mouse(mouse) => {
                if is_click(mouse) {
                    self.toast = None;
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
            self.toast = Some(Toast::problem(reason));
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
        if let Some(child) = self.pending.take() {
            if is_yes(key.code) {
                self.confirm(child);
            }
            return Outcome::Continue;
        }
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
            KeyCode::Char('o') => self.act(actions::reveal, "Showed it in the Finder."),
            KeyCode::Char('y') => self.act(actions::copy, "Copied the path."),
            KeyCode::Char('d') | KeyCode::Delete => self.ask(),
            KeyCode::Char('?') => self.is_helping = true,
            _ => {}
        }
        Outcome::Continue
    }

    fn selected_path(&self) -> Option<PathBuf> {
        Some(self.path().join(&*self.selected()?.name))
    }

    fn act(&mut self, action: fn(&Path) -> io::Result<()>, done: &str) {
        let Some(path) = self.selected_path() else {
            return;
        };
        self.toast = Some(match action(&path) {
            Ok(()) => Toast::done(done),
            Err(err) => Toast::problem(format!("That did not work: {err}")),
        });
    }

    fn ask(&mut self) {
        let Some(child) = self.selected_child() else {
            return;
        };
        let path = self.path().join(&*self.current().children[child].name);
        match trash_refusal(&self.current().children[child], &path) {
            Some(reason) => self.toast = Some(Toast::problem(reason)),
            None => self.pending = Some(child),
        }
    }

    fn confirm(&mut self, child: usize) {
        let mut trail = self.trail.clone();
        trail.push(child);
        let node = self.root.descend(&trail);
        let (name, size) = (node.label().into_owned(), node.size);
        let path = self.path().join(&*node.name);
        self.toast = Some(match self.trash(&trail, &path) {
            Ok(_) => {
                self.trashed += size;
                Toast::done(trashed_message(&name, size))
            }
            Err(err) => Toast::problem(format!("Could not move {name} to the Trash: {err}")),
        });
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let [top, body, bottom] = screen(frame.area());
        let crumbs = self.crumbs();
        let of = crumbs.last().map_or("", String::as_str);
        let (body, side) = detail::split(body, || self.detail(of));
        header(frame, top, &crumbs, &format::size(self.current().size));
        let count = self.count();
        let room = usize::from(body.width.saturating_sub(width(&count) + 10));
        let name = fit_end(crumbs.last().map_or("", String::as_str), room);
        let trouble = self.trouble();
        let mut block = theme::counted(&name, &count).title_bottom(theme::aside(&trouble));
        if self.origin.is_some() {
            let room = usize::from(body.width.saturating_sub(width(&trouble) + 10));
            let path = fit_start(&format::path(&self.path()), room);
            block = block.title_bottom(theme::note(&path));
        }
        let inner = framed(frame, body, block);
        let [list, strip] = make_room(inner, self.toast.as_ref());
        self.render_rows(frame, list, side.is_none());
        if let Some((side, detail)) = &side {
            detail::render(frame, *side, detail);
        }
        footer(frame, bottom, &KEYS);
        if let Some(toast) = &self.toast {
            show_toast(frame, strip, toast);
        }
        if let Some(child) = self.pending {
            let node = &self.current().children[child];
            let path = format::path(&self.path().join(&*node.name));
            confirm_trash(frame, &node.label(), &path, node.size, node.items);
        }
        if self.is_helping {
            help::render(frame, help::BROWSER);
        }
    }

    /// How many entries the folder shows, and the order when it is not by size.
    fn count(&self) -> String {
        let count = format::items(self.rows.len() as u64);
        match self.sort {
            Sort::Size => count,
            sort => format!("{count} · by {}", sort.label()),
        }
    }

    /// The pane for the selected entry, as part of the folder called `of`.
    fn detail(&self, of: &str) -> Option<Detail> {
        let node = self.selected()?;
        Some(detail::of_node(node, node.size, of, self.current().size))
    }

    fn render_rows(&mut self, frame: &mut Frame, area: Rect, has_items: bool) {
        let current = self.root.descend(&self.trail);
        if self.rows.is_empty() {
            let note = match current {
                n if n.is_denied => "No access",
                n if n.is_unreadable => "Could not be read",
                _ => "Empty folder",
            };
            let [middle] = Layout::vertical([Constraint::Length(1)])
                .flex(Flex::Center)
                .areas(area);
            frame.render_widget(Paragraph::new(note).centered().style(DIM), middle);
            return;
        }
        let visible = self.cursor.window(area, self.rows.len());
        let columns = Columns::fit(row_width(area), current, has_items);
        let rows = self.rows[visible]
            .iter()
            .map(|&child| row(&current.children[child], current.size, &columns));
        let table = list(rows, columns.widths());
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

pub(crate) fn trash_refusal(node: &Node, path: &Path) -> Option<String> {
    let name = node.label();
    if actions::is_protected(path, &volumes::home()) {
        Some(format!(
            "roomy will not move {name}: your home folder lives there."
        ))
    } else if node.is_denied {
        Some(format!(
            "roomy cannot see inside {name}, so it leaves it alone."
        ))
    } else if node.is_mount {
        Some(format!(
            "{name} is another disk; roomy does not move disks."
        ))
    } else {
        None
    }
}

pub(crate) fn trashed_message(name: &str, size: u64) -> String {
    format!(
        "Moved {name} ({}) to the Trash. Empty the Trash to free the space.",
        format::size(size)
    )
}

pub(crate) fn is_yes(code: KeyCode) -> bool {
    matches!(code, KeyCode::Char('y' | 'Y'))
}

pub(crate) fn confirm_trash(frame: &mut Frame, name: &str, path: &str, size: u64, items: u64) {
    let area = frame.area();
    let width = area.width.saturating_sub(4).min(72);
    let popup = centered(area, width, 7);
    let lines = popup_lines(
        name,
        path,
        size,
        items,
        usize::from(width.saturating_sub(6)),
    );
    let block = theme::alert("Move to the Trash?").padding(Padding::horizontal(1));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn popup_lines(name: &str, path: &str, size: u64, items: u64, room: usize) -> Vec<Line<'static>> {
    let mut amount = vec![Span::styled(format::size(size), theme::size(size))];
    if items > 0 {
        let items = format!(" · {}", format::items(items));
        amount.push(Span::styled(items, Style::new().fg(DIM)));
    }
    vec![
        Line::styled(
            fit_start(name, room),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Line::styled(fit_start(path, room), Style::new().fg(DIM)),
        Line::from(amount),
        Line::raw(""),
        Line::from(vec![
            theme::keycap("y"),
            Span::styled(" move to Trash  ", Style::new().fg(DIM)),
            theme::keycap("any other key"),
            Span::styled(" keep it", Style::new().fg(DIM)),
        ]),
    ]
}

fn refusal(node: &Node) -> Option<String> {
    let name = node.label();
    if node.is_denied {
        Some(format!("No access to {name}. {}", format::ACCESS_HINT))
    } else if node.is_unreadable && node.children.is_empty() {
        Some(format!("{name} could not be read."))
    } else if node.is_mount {
        Some(format!(
            "{name} is another disk. Run roomy on it to look inside."
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
    fn fit(width: u16, node: &Node, has_items: bool) -> Self {
        let bar = bar_width(width);
        let most = node
            .children
            .iter()
            .map(|child| child.items)
            .max()
            .unwrap_or(0);
        let items = if !has_items || bar == 0 || most == 0 {
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

/// What a `list` in `area` leaves for its columns once the marker has its own.
pub(crate) fn row_width(area: Rect) -> u16 {
    area.width.saturating_sub(width(theme::MARKER))
}

/// A table whose selected row keeps its colors and gets a marker in the left margin.
pub(crate) fn list<'a, R, C>(rows: R, widths: C) -> Table<'a>
where
    R: IntoIterator,
    R::Item: Into<Row<'a>>,
    C: IntoIterator,
    C::Item: Into<Constraint>,
{
    Table::new(rows, widths)
        .column_spacing(1)
        .row_highlight_style(SELECTION)
        .highlight_symbol(theme::marker())
        .highlight_spacing(HighlightSpacing::Always)
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
        Cell::from(Line::styled(format::size(node.size), theme::size(node.size)).right_aligned()),
        Cell::from(Line::from(format::percent(share)).right_aligned()).style(dim),
        Cell::from(bar_line(share, columns.bar)),
        Cell::from(name(node)),
        Cell::from(Line::from(items).right_aligned()).style(dim),
    ])
}

pub(crate) fn bar_line(share: f64, width: u16) -> Line<'static> {
    Line::styled(bar(share, width), Style::new().fg(theme::heat(share)))
}

fn name(node: &Node) -> Line<'_> {
    let label = node.label();
    let (slash, glyph) = if node.is_dir() {
        ("/", Span::styled("▸ ", Style::new().fg(ACCENT)))
    } else {
        ("", Span::raw("  "))
    };
    let dir = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
    let warn = Style::new().fg(WARN);
    let note = |text: &'static str| Span::styled(text, warn.add_modifier(Modifier::DIM));
    let rest = match node.kind {
        _ if node.is_denied => vec![
            Span::styled(format!("{label}{slash}"), warn),
            note("  no access"),
        ],
        _ if node.is_unreadable && node.children.is_empty() => vec![
            Span::styled(format!("{label}{slash}"), warn),
            note("  unreadable"),
        ],
        _ if node.is_unreadable => vec![
            Span::styled(format!("{label}/"), dir),
            note("  partly unreadable"),
        ],
        _ if node.is_mount => vec![
            Span::styled(format!("{label}/"), dir),
            Span::styled("  other disk", Style::new().fg(DIM)),
        ],
        Kind::Dir => vec![Span::styled(format!("{label}/"), dir)],
        Kind::Link => vec![
            Span::styled(label, theme::file(&node.name)),
            Span::styled(" →", Style::new().fg(DIM)),
        ],
        Kind::File | Kind::Other => vec![Span::styled(label, theme::file(&node.name))],
    };
    Line::from([vec![glyph], rest].concat())
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

pub(crate) fn bar(share: f64, width: u16) -> String {
    let eighths = (share.clamp(0.0, 1.0) * f64::from(width) * 8.0).round() as usize;
    let mut bar = "█".repeat(eighths / 8);
    if !eighths.is_multiple_of(8) {
        bar.push(PARTIAL[eighths % 8]);
    }
    bar
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

/// Keeps the start of `text`, cut after a whole word when one fits.
pub(crate) fn fit_end(text: &str, room: usize) -> String {
    if text.chars().count() <= room {
        return text.to_string();
    }
    if room == 0 {
        return String::new();
    }
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(room.saturating_sub(1)).collect();
    let is_mid_word = chars.next().is_some_and(|next| !next.is_whitespace());
    let head = match head.rfind(' ') {
        Some(space) if is_mid_word => &head[..space],
        _ => &head,
    };
    format!("{}…", head.trim_end_matches([' ', ',', ':', ';']))
}

/// Splits text into lines of at most `room` characters, breaking between words.
pub(crate) fn wrap(text: &str, room: usize) -> Vec<String> {
    let room = room.max(1);
    let words = text.split_whitespace().flat_map(|word| {
        let chars: Vec<char> = word.chars().collect();
        let pieces = chars
            .chunks(room)
            .map(|piece| piece.iter().collect::<String>());
        pieces.collect::<Vec<_>>()
    });
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in words {
        let needed = line.chars().count() + 1 + word.chars().count();
        if !line.is_empty() && needed > room {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn toast_lines(toast: &Toast, width: u16) -> Vec<String> {
    wrap(&toast.text, usize::from(width.saturating_sub(10)).min(56))
}

/// Splits a strip for the toast off the bottom of `area`, so the toast covers no rows.
pub(crate) fn make_room(area: Rect, toast: Option<&Toast>) -> [Rect; 2] {
    let height = toast.map_or(0, |toast| toast_lines(toast, area.width).len() as u16 + 2);
    Layout::vertical([Constraint::Fill(1), Constraint::Length(height)]).areas(area)
}

/// The header row, the panel between and the key row.
pub(crate) fn screen(area: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area)
}

/// Draws the block and returns the area inside it.
pub(crate) fn framed(frame: &mut Frame, area: Rect, block: Block) -> Rect {
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

pub(crate) fn path_crumbs(path: &str) -> Vec<String> {
    let mut crumbs = Vec::new();
    if path.starts_with('/') {
        crumbs.push("/".to_string());
    }
    let parts = path.split('/').filter(|part| !part.is_empty());
    crumbs.extend(parts.map(str::to_string));
    crumbs
}

pub(crate) fn header(frame: &mut Frame, area: Rect, crumbs: &[String], summary: &str) {
    let summary = format!("{summary} ");
    let [left, right] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(width(&summary))]).areas(area);
    let badge = " roomy ";
    let mut spans = vec![Span::styled(badge, BADGE), Span::raw("  ")];
    // The badge and its gap come first; 3 more columns keep the crumbs off the summary.
    let room = left.width.saturating_sub(width(badge) + 2 + 3);
    spans.extend(crumb_spans(crumbs, usize::from(room)));
    frame.render_widget(Paragraph::new(Line::from(spans)), left);
    frame.render_widget(Paragraph::new(summary).right_aligned(), right);
}

/// Shows as many of the last crumbs as fit, behind a "…" when the first ones do not.
fn crumb_spans(crumbs: &[String], room: usize) -> Vec<Span<'static>> {
    let Some(last) = crumbs.last() else {
        return Vec::new();
    };
    let count = crumbs.len();
    let shown = (1..=count)
        .rev()
        .find(|&shown| crumbs_width(crumbs, shown) <= room)
        .unwrap_or(1);
    let separator = Span::styled(SEPARATOR, Style::new().fg(TRACK));
    let mut spans = Vec::new();
    if shown < count {
        spans.extend([Span::styled("…", Style::new().fg(DIM)), separator.clone()]);
    }
    for crumb in &crumbs[count - shown..count - 1] {
        spans.extend([
            Span::styled(crumb.clone(), Style::new().fg(DIM)),
            separator.clone(),
        ]);
    }
    let before = crumbs_width(crumbs, shown) - last.chars().count();
    let last = fit_start(last, room.saturating_sub(before));
    spans.push(Span::styled(
        last,
        Style::new().add_modifier(Modifier::BOLD),
    ));
    spans
}

/// The width of the last `shown` crumbs, with the "… › " that stands in for the others.
fn crumbs_width(crumbs: &[String], shown: usize) -> usize {
    let separator = usize::from(width(SEPARATOR));
    let names: usize = crumbs[crumbs.len() - shown..]
        .iter()
        .map(|crumb| crumb.chars().count())
        .sum();
    let hidden = if shown < crumbs.len() {
        1 + separator
    } else {
        0
    };
    names + separator * (shown - 1) + hidden
}

/// Key hints as keycaps: as many as fit, and always the last one, at the right end.
pub(crate) fn footer(frame: &mut Frame, area: Rect, keys: &[(&str, &str)]) {
    let Some((last, rest)) = keys.split_last() else {
        return;
    };
    let cost = |(key, action): &(&str, &str)| width(key) + width(action) + 5;
    let room = area.width.saturating_sub(1 + cost(last));
    let mut shown: Vec<&(&str, &str)> = rest.iter().collect();
    while shown.iter().map(|hint| cost(hint)).sum::<u16>() > room {
        let keep_help = shown.iter().rposition(|(key, _)| *key != HELP_KEY);
        shown.remove(keep_help.unwrap_or(shown.len() - 1));
    }
    let mut spans = vec![Span::raw(" ")];
    for hint in shown {
        spans.extend(key_hint(hint, "  "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    if area.width >= cost(last) {
        let last = Line::from(Vec::from(key_hint(last, " "))).right_aligned();
        frame.render_widget(Paragraph::new(last), area);
    }
}

fn key_hint((key, action): &(&str, &str), gap: &str) -> [Span<'static>; 2] {
    [
        theme::keycap(key),
        Span::styled(format!(" {action}{gap}"), Style::new().fg(DIM)),
    ]
}

/// Draws the toast at the right end of the strip `make_room` kept for it.
pub(crate) fn show_toast(frame: &mut Frame, strip: Rect, toast: &Toast) {
    let (glyph, color) = match toast.tone {
        Tone::Done => ("✓", DONE),
        Tone::Info => ("•", ACCENT),
        Tone::Problem => ("!", WARN),
    };
    let lines = toast_lines(toast, strip.width);
    let longest = lines.iter().map(|line| width(line)).max().unwrap_or(0);
    let popup_width = longest + 6;
    let popup = Rect {
        x: strip.right().saturating_sub(popup_width + 1),
        width: popup_width,
        ..strip
    }
    .intersection(strip);
    let lead = Style::new().fg(color).add_modifier(Modifier::BOLD);
    let marks = std::iter::once(format!("{glyph} ")).chain(std::iter::repeat(String::from("  ")));
    let text: Vec<Line> = marks
        .zip(lines)
        .map(|(mark, line)| Line::from(vec![Span::styled(mark, lead), Span::raw(line)]))
        .collect();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(color))
        .padding(Padding::horizontal(1));
    frame.render_widget(Paragraph::new(text).block(block), popup);
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
    let crumbs = path_crumbs(&label);
    let scanned = wait(terminal, progress, scanner, |frame, elapsed| {
        let [top, body, bottom] = screen(frame.area());
        header(frame, top, &crumbs, "");
        scanning(frame, body, &label, progress, elapsed, |width| {
            Some(sweep(elapsed, width))
        });
        footer(frame, bottom, &STOP_KEYS);
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

/// While an overlay is open, the next key press or click only closes it; Ctrl+C still quits.
pub(crate) fn close_overlay(is_open: &mut bool, event: &Event) -> Outcome {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press && is_ctrl_c(*key) => Outcome::Quit,
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            *is_open = false;
            Outcome::Continue
        }
        Event::Mouse(mouse) if is_click(mouse) => {
            *is_open = false;
            Outcome::Continue
        }
        _ => Outcome::Continue,
    }
}

/// A `width` by `height` rectangle in the middle of `area`, cut to fit inside it.
pub(crate) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [row] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    let [middle] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(row);
    middle
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
    is_ctrl_c(key) || key.code == KeyCode::Char('q')
}

fn is_ctrl_c(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// A card in the middle of `area` that counts what the scan has seen so far, with the
/// line `gauge` draws for the room inside the card, if any.
pub(crate) fn scanning(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    progress: &Progress,
    elapsed: Duration,
    gauge: impl FnOnce(u16) -> Option<Line<'static>>,
) {
    let card_width = area.width.saturating_sub(4).min(68);
    let room = card_width.saturating_sub(6);
    let mut lines = progress_lines(progress, elapsed, room);
    if let Some(gauge) = gauge(room) {
        lines.splice(3..3, [gauge, Line::default()]);
    }
    let card = centered(area, card_width, lines.len() as u16 + 2);
    let label = fit_start(label, usize::from(card_width.saturating_sub(16)));
    let block = theme::panel(&format!("Scanning {label}")).padding(Padding::horizontal(2));
    frame.render_widget(Paragraph::new(lines).block(block), card);
}

/// A block that slides to and fro while a scan has no end in sight.
fn sweep(elapsed: Duration, width: u16) -> Line<'static> {
    let width = usize::from(width);
    let block = (width / 6).max(1);
    let room = width.saturating_sub(block).max(1);
    let step = 2 * (elapsed.as_millis() / TICK.as_millis()) as usize % (2 * room);
    let start = if step > room { 2 * room - step } else { step };
    Line::from(vec![
        Span::styled("·".repeat(start), Style::new().fg(TRACK)),
        Span::styled("█".repeat(block), Style::new().fg(ACCENT)),
        Span::styled(
            "·".repeat(width.saturating_sub(start + block)),
            Style::new().fg(TRACK),
        ),
    ])
}

fn progress_lines(progress: &Progress, elapsed: Duration, room: u16) -> Vec<Line<'static>> {
    let spinner = SPINNER[(elapsed.as_millis() / TICK.as_millis()) as usize % SPINNER.len()];
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let dim = Style::new().fg(DIM);
    let counts = Line::from(vec![
        Span::styled(format!("{spinner}  "), Style::new().fg(ACCENT)),
        Span::styled(format::count(progress.items()), bold),
        Span::styled(" items   ", dim),
        Span::styled(format::size(progress.bytes()), bold),
        Span::styled(format!("   {:.1} s", elapsed.as_secs_f64()), dim),
    ]);
    let current = fit_start(&format::path(&progress.current()), usize::from(room));
    vec![
        Line::default(),
        counts,
        Line::default(),
        Line::styled(current, dim),
        Line::default(),
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
        eprintln!("roomy: could not release the mouse: {err}");
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
    fn fit_end_keeps_the_start() {
        assert_eq!(fit_end("short", 10), "short");
        assert_eq!(fit_end("what apps keep in ~/Library", 12), "what apps…");
    }

    #[test]
    fn wrap_breaks_between_words() {
        assert_eq!(
            wrap("No access to locked. Give your terminal access.", 20),
            ["No access to locked.", "Give your terminal", "access."]
        );
        assert_eq!(
            wrap("Could not move /a/very/long/path", 10),
            ["Could not", "move", "/a/very/lo", "ng/path"]
        );
    }

    #[test]
    fn crumbs_drop_the_first_ones_when_room_runs_out() {
        let crumbs = path_crumbs("~/Documents/Panenco/qit");
        assert_eq!(crumbs, ["~", "Documents", "Panenco", "qit"]);
        assert_eq!(path_crumbs("/data"), ["/", "data"]);
        let text = |room| -> String {
            crumb_spans(&crumbs, room)
                .iter()
                .map(|span| span.content.to_string())
                .collect()
        };
        assert_eq!(text(40), "~ › Documents › Panenco › qit");
        assert_eq!(text(20), "… › Panenco › qit");
        assert_eq!(text(10), "… › qit");
    }

    #[test]
    fn scrolling_keeps_the_cursor_in_view() {
        assert_eq!(scrolled(0, 3, 10), 0);
        assert_eq!(scrolled(0, 10, 10), 1);
        assert_eq!(scrolled(5, 2, 10), 2);
        assert_eq!(scrolled(5, 14, 10), 5);
    }
}
