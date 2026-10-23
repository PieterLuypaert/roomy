use std::io;
use std::path::Path;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, MouseEvent};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row};
use ratatui::{DefaultTerminal, Frame};

use crate::actions::{self, Trasher};
use crate::categories::{self, Group, Place, Summary};
use crate::format;
use crate::scan::{self, Progress};
use crate::theme::{self, ACCENT, DIM, TRACK, WARN};
use crate::tree::Node;
use crate::ui::{self, Browser, Cursor, Outcome, Pointer, Toast};
use crate::volumes::{self, Disk};

const PALETTE: [Color; 7] = [
    Color::Indexed(68),
    Color::Indexed(166),
    Color::Indexed(35),
    Color::Indexed(136),
    Color::Indexed(168),
    Color::Indexed(28),
    Color::Indexed(104),
];
const OTHER: Color = Color::Indexed(245);
const SYSTEM: Color = Color::Indexed(250);
const HIDDEN: Color = Color::Indexed(242);
const OVERVIEW_KEYS: [(&str, &str); 4] = [
    ("↑↓", "move"),
    ("→", "open"),
    ("b", "browse all"),
    ("q", "quit"),
];
const PLACE_KEYS: [(&str, &str); 7] = [
    ("↑↓", "move"),
    ("→", "open"),
    ("←", "back"),
    ("o", "finder"),
    ("y", "copy path"),
    ("d", "trash"),
    ("q", "quit"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Group(Group),
    Hidden,
    System,
    Reserved,
    Free,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
    pub part: Part,
    pub size: u64,
    pub places: usize,
    pub denied: u64,
    pub is_blocked: bool,
}

impl Slice {
    pub fn name(&self) -> &'static str {
        match self.part {
            Part::Group(group) => group.name(),
            Part::Hidden => "Hidden from ruim",
            Part::System => "macOS",
            Part::Reserved if cfg!(target_os = "macos") => "System volumes",
            Part::Reserved => "Reserved",
            Part::Free => "Free",
        }
    }

    pub fn about(&self, disk: &Disk) -> String {
        let text = match self.part {
            Part::Group(group) => group.about(),
            Part::Hidden if disk.is_apfs => {
                "folders that need Full Disk Access, local snapshots and purgeable space"
            }
            Part::Hidden => "folders ruim may not read and filesystem metadata",
            Part::System => "the sealed, read-only system volume",
            Part::Reserved if disk.is_apfs => {
                "Preboot, VM (swap and sleep image), Update and Recovery"
            }
            Part::Reserved => "blocks the filesystem keeps for root",
            Part::Free => {
                return disk.purgeable.map_or_else(String::new, |purgeable| {
                    format!(
                        "{} more can be freed by macOS when needed",
                        format::size(purgeable)
                    )
                });
            }
        };
        text.to_string()
    }
}

pub fn slices(disk: &Disk, summary: &Summary, moved: u64) -> Vec<Slice> {
    let mut slices = group_slices(summary, moved);
    let fixed = [
        (
            Part::Hidden,
            disk.data.saturating_sub(summary.size() + moved),
        ),
        (Part::System, disk.system),
        (Part::Reserved, disk.other()),
        (Part::Free, disk.free),
    ];
    for (part, size) in fixed {
        if size > 0 || part == Part::Free {
            slices.push(Slice {
                part,
                size,
                places: 0,
                denied: 0,
                is_blocked: false,
            });
        }
    }
    slices
}

pub fn overcount(disk: &Disk, summary: &Summary, moved: u64) -> u64 {
    (summary.size() + moved).saturating_sub(disk.data)
}

pub fn scale(disk: &Disk, slices: &[Slice]) -> u64 {
    disk.total.max(slices.iter().map(|slice| slice.size).sum())
}

pub fn overcount_note(extra: u64) -> String {
    format!("{} counted twice (APFS clones)", format::size(extra))
}

fn group_slices(summary: &Summary, moved: u64) -> Vec<Slice> {
    let mut slices: Vec<Slice> = summary
        .totals
        .iter()
        .map(|total| Slice {
            part: Part::Group(total.group),
            size: total.size,
            places: total.places,
            denied: total.denied,
            is_blocked: total.is_blocked,
        })
        .collect();
    if moved == 0 {
        return slices;
    }
    let trash = Part::Group(Group::Trash);
    match slices.iter_mut().find(|slice| slice.part == trash) {
        Some(slice) => slice.size += moved,
        None => slices.push(Slice {
            part: trash,
            size: moved,
            places: 0,
            denied: 0,
            is_blocked: false,
        }),
    }
    slices.sort_by_key(|slice| std::cmp::Reverse(slice.size));
    slices
}

pub fn disk_summary(disk: &Disk) -> String {
    format!(
        "{} · {} used · {} free",
        format::size(disk.total),
        format::size(disk.used()),
        format::size(disk.free)
    )
}

pub struct Scanned {
    pub root: Node,
    pub summary: Summary,
    pub purgeable: Option<u64>,
}

pub fn scan_disk(disk: &Disk, progress: &Progress) -> io::Result<Scanned> {
    let root = scan::scan(&disk.scan_root, progress)?;
    let summary = categories::summarize(&root, &disk.scan_root, &volumes::home());
    let purgeable = if disk.is_apfs {
        volumes::purgeable(disk.free)
    } else {
        None
    };
    Ok(Scanned {
        root,
        summary,
        purgeable,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Overview,
    Places,
    Browser,
}

pub struct App {
    disk: Disk,
    summary: Summary,
    slices: Vec<Slice>,
    colors: Vec<Group>,
    browser: Browser,
    overview: Cursor,
    places: Cursor,
    shown: Vec<usize>,
    group: Option<Group>,
    screen: Screen,
    came_from: Screen,
    toast: Option<Toast>,
    moved: u64,
    pending: Option<usize>,
}

impl App {
    pub fn new(disk: Disk, root: Node, summary: Summary) -> Self {
        let slices = slices(&disk, &summary, 0);
        let colors = summary.totals.iter().map(|total| total.group).collect();
        let browser = Browser::new(root, disk.scan_root.clone());
        App {
            disk,
            summary,
            slices,
            colors,
            browser,
            overview: Cursor::default(),
            places: Cursor::default(),
            shown: Vec::new(),
            group: None,
            screen: Screen::Overview,
            came_from: Screen::Overview,
            toast: None,
            moved: 0,
            pending: None,
        }
    }

    pub fn with_trasher(mut self, trasher: Trasher) -> Self {
        self.browser = self.browser.with_trasher(trasher);
        self
    }

    pub fn browser(&self) -> Option<&Browser> {
        (self.screen == Screen::Browser).then_some(&self.browser)
    }

    pub fn group(&self) -> Option<Group> {
        self.group.filter(|_| self.screen == Screen::Places)
    }

    pub fn status(&self) -> Option<&str> {
        self.toast.as_ref().map(Toast::text)
    }

    pub fn handle(&mut self, event: &Event) -> Outcome {
        if self.screen == Screen::Browser {
            return self.handle_browser(event);
        }
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(*key),
            Event::Mouse(mouse) if self.pending.is_none() => {
                if ui::is_click(mouse) {
                    self.toast = None;
                }
                self.on_pointer(*mouse);
                Outcome::Continue
            }
            _ => Outcome::Continue,
        }
    }

    fn handle_browser(&mut self, event: &Event) -> Outcome {
        let outcome = self.browser.handle(event);
        let moved = self.browser.take_trashed();
        if moved > 0 {
            self.moved += moved;
            self.refresh();
        }
        if outcome == Outcome::Leave {
            self.screen = self.came_from;
            return Outcome::Continue;
        }
        outcome
    }

    fn handle_key(&mut self, key: KeyEvent) -> Outcome {
        self.toast = None;
        if let Some(index) = self.pending.take() {
            if ui::is_yes(key.code) {
                self.confirm_place(index);
            }
            return Outcome::Continue;
        }
        if ui::is_quit_key(key) {
            return Outcome::Quit;
        }
        self.on_key(key.code);
        Outcome::Continue
    }

    fn on_key(&mut self, code: KeyCode) {
        let is_moved = match self.screen {
            Screen::Overview => self.overview.key(code, self.slices.len()),
            Screen::Places => self.places.key(code, self.shown.len()),
            Screen::Browser => false,
        };
        if is_moved {
            return;
        }
        match (self.screen, code) {
            (_, KeyCode::Char('b')) => self.browse_all(),
            (Screen::Overview, KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')) => {
                self.open_slice();
            }
            (Screen::Places, KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')) => {
                self.open_place();
            }
            (
                Screen::Places,
                KeyCode::Left | KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('h'),
            ) => self.screen = Screen::Overview,
            (Screen::Places, KeyCode::Char('o')) => {
                self.act(actions::reveal, "Showed it in the Finder.");
            }
            (Screen::Places, KeyCode::Char('y')) => self.act(actions::copy, "Copied the path."),
            (Screen::Places, KeyCode::Char('d') | KeyCode::Delete) => self.ask(),
            _ => {}
        }
    }

    fn selected_place(&self) -> Option<usize> {
        self.shown.get(self.places.index).copied()
    }

    fn act(&mut self, action: fn(&Path) -> io::Result<()>, done: &str) {
        let Some(index) = self.selected_place() else {
            return;
        };
        self.toast = Some(match action(&self.summary.places[index].path) {
            Ok(()) => Toast::done(done),
            Err(err) => Toast::problem(format!("That did not work: {err}")),
        });
    }

    fn ask(&mut self) {
        let Some(index) = self.selected_place() else {
            return;
        };
        let place = &self.summary.places[index];
        let node = self.browser.root().descend(&place.trail);
        let refusal = if place.is_container {
            Some(
                "This row holds the loose files of a folder; open it to pick what to move."
                    .to_string(),
            )
        } else {
            ui::trash_refusal(node, &place.path)
        };
        match refusal {
            Some(reason) => self.toast = Some(Toast::problem(reason)),
            None => self.pending = Some(index),
        }
    }

    fn confirm_place(&mut self, index: usize) {
        let place = &self.summary.places[index];
        let (trail, path) = (place.trail.clone(), place.path.clone());
        let name = name_of(&path);
        self.toast = Some(match self.browser.trash(&trail, &path) {
            Ok(size) => {
                self.moved += size;
                self.refresh();
                Toast::done(ui::trashed_message(&name, size))
            }
            Err(err) => Toast::problem(format!("Could not move {name} to the Trash: {err}")),
        });
    }

    fn refresh(&mut self) {
        let root = self.browser.root();
        self.summary = categories::summarize(root, &self.disk.scan_root, &volumes::home());
        self.slices = slices(&self.disk, &self.summary, self.moved);
        if let Some(group) = self.group {
            self.shown = self.summary.place_indices(group);
        }
        self.overview.move_by(0, self.slices.len());
        self.places.move_by(0, self.shown.len());
    }

    fn on_pointer(&mut self, mouse: MouseEvent) {
        let pointer = match self.screen {
            Screen::Overview => self.overview.mouse(mouse, self.slices.len()),
            Screen::Places => self.places.mouse(mouse, self.shown.len()),
            Screen::Browser => Pointer::Nothing,
        };
        match (self.screen, pointer) {
            (Screen::Overview, Pointer::Activate) => self.open_slice(),
            (Screen::Places, Pointer::Activate) => self.open_place(),
            (Screen::Places, Pointer::Back) => self.screen = Screen::Overview,
            _ => {}
        }
    }

    fn open_slice(&mut self) {
        let Some(slice) = self.slices.get(self.overview.index) else {
            return;
        };
        match slice.part {
            Part::Group(group) => {
                self.shown = self.summary.place_indices(group);
                self.group = Some(group);
                self.places.reset(0);
                self.screen = Screen::Places;
            }
            Part::Hidden => self.toast = Some(Toast::info(hidden_reason(&self.disk))),
            Part::System | Part::Reserved => {
                self.toast = Some(Toast::info("The system manages this space itself."));
            }
            Part::Free => {}
        }
    }

    fn open_place(&mut self) {
        let Some(&index) = self.shown.get(self.places.index) else {
            return;
        };
        self.browser.jump(self.summary.places[index].trail.clone());
        self.came_from = Screen::Places;
        self.screen = Screen::Browser;
    }

    fn browse_all(&mut self) {
        self.browser.jump(Vec::new());
        self.came_from = self.screen;
        self.screen = Screen::Browser;
    }

    pub fn render(&mut self, frame: &mut Frame) {
        match self.screen {
            Screen::Overview => self.render_overview(frame),
            Screen::Places => self.render_places(frame),
            Screen::Browser => self.browser.render(frame),
        }
    }

    fn color(&self, part: Part) -> Color {
        match part {
            Part::Group(group) => {
                let rank = self.colors.iter().position(|&g| g == group);
                rank.and_then(|rank| PALETTE.get(rank))
                    .copied()
                    .unwrap_or(OTHER)
            }
            Part::Hidden => HIDDEN,
            Part::System | Part::Reserved => SYSTEM,
            Part::Free => TRACK,
        }
    }

    fn render_overview(&mut self, frame: &mut Frame) {
        let [top, body, bottom] = ui::screen(frame.area());
        let crumbs = std::slice::from_ref(&self.disk.name);
        ui::header(frame, top, crumbs, &disk_summary(&self.disk));
        let block = theme::panel("Where the space goes").title_bottom(theme::aside(&self.notes()));
        let inner = ui::framed(frame, body, block);
        let [bar, pointer, rest] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(inner);
        let [list, strip] = ui::make_room(rest, self.toast.as_ref());
        let bar = bar.inner(Margin::new(1, 0));
        let parts: Vec<(u64, Color, bool)> = self
            .slices
            .iter()
            .map(|slice| (slice.size, self.color(slice.part), slice.part == Part::Free))
            .collect();
        let line = stacked(&parts, bar.width);
        frame.render_widget(Paragraph::new(vec![line.clone(), line]), bar);
        let pointer = pointer.inner(Margin::new(1, 0));
        frame.render_widget(Paragraph::new(self.pointer(&parts, pointer.width)), pointer);
        self.render_slices(frame, list);
        ui::footer(frame, bottom, &OVERVIEW_KEYS);
        if let Some(toast) = &self.toast {
            ui::show_toast(frame, strip, toast);
        }
    }

    /// Underlines the selected part of the disk bar.
    fn pointer(&self, parts: &[(u64, Color, bool)], width: u16) -> Line<'static> {
        let Some(slice) = self.slices.get(self.overview.index) else {
            return Line::default();
        };
        let sizes: Vec<u64> = parts.iter().map(|part| part.0).collect();
        let cells = apportion(&sizes, usize::from(width));
        let start: usize = cells[..self.overview.index].iter().sum();
        Line::from(vec![
            Span::raw(" ".repeat(start)),
            Span::styled(
                "▔".repeat(cells[self.overview.index]),
                Style::new().fg(self.color(slice.part)),
            ),
        ])
    }

    fn notes(&self) -> String {
        let denied: u64 = self.summary.totals.iter().map(|total| total.denied).sum();
        let mut notes = Vec::new();
        if denied > 0 {
            notes.push(format!("{} no access", format::count(denied)));
        }
        let extra = overcount(&self.disk, &self.summary, self.moved);
        if extra > 0 {
            notes.push(overcount_note(extra));
        }
        notes.join(" · ")
    }

    fn render_slices(&mut self, frame: &mut Frame, area: Rect) {
        let visible = self.overview.window(area, self.slices.len());
        let name_width = self
            .slices
            .iter()
            .map(|slice| {
                let note = access_note(slice.is_blocked, slice.denied).map_or(0, |n| n.width());
                ui::width(slice.name()) + note as u16
            })
            .max()
            .unwrap_or(0);
        let about = area
            .width
            .saturating_sub(1 + 9 + 4 + 1 + name_width + 13 + 5);
        let rows: Vec<Row> = self.slices[visible]
            .iter()
            .map(|slice| self.slice_row(slice, usize::from(about)))
            .collect();
        let widths = [
            Constraint::Length(9),
            Constraint::Length(4),
            Constraint::Length(1),
            Constraint::Length(name_width),
            Constraint::Fill(1),
            Constraint::Length(13),
        ];
        frame.render_stateful_widget(ui::list(rows, widths), area, &mut self.overview.state());
    }

    fn slice_row(&self, slice: &Slice, about: usize) -> Row<'static> {
        let share = format::share(slice.size, scale(&self.disk, &self.slices));
        let swatch = match slice.part {
            Part::Free => " ",
            _ => "■",
        };
        let places = match slice.places {
            0 => String::new(),
            n => format::places(n),
        };
        let mut name = vec![Span::raw(slice.name())];
        if let Some(note) = access_note(slice.is_blocked, slice.denied) {
            name.push(note);
        }
        let dim = Style::new().fg(DIM);
        Row::new([
            Cell::from(
                Line::styled(format::size(slice.size), theme::size(slice.size)).right_aligned(),
            ),
            Cell::from(Line::from(format::percent(share)).right_aligned()).style(dim),
            Cell::from(Span::styled(
                swatch,
                Style::new().fg(self.color(slice.part)),
            )),
            Cell::from(Line::from(name)),
            Cell::from(ui::fit_end(&slice.about(&self.disk), about)).style(dim),
            Cell::from(Line::from(places).right_aligned()).style(dim),
        ])
    }

    fn render_places(&mut self, frame: &mut Frame) {
        let Some(group) = self.group else {
            return;
        };
        let [top, body, bottom] = ui::screen(frame.area());
        let total: u64 = self
            .shown
            .iter()
            .map(|&i| self.summary.places[i].size)
            .sum();
        let summary = format!(
            "{} · {}",
            format::size(total),
            format::places(self.shown.len())
        );
        let crumbs = [self.disk.name.clone(), group.name().to_string()];
        ui::header(frame, top, &crumbs, &summary);
        let mut block = theme::panel(group.name());
        let room = body.width.saturating_sub(ui::width(group.name()) + 10);
        if ui::width(group.about()) <= room {
            block = block.title(theme::aside(group.about()));
        }
        let inner = ui::framed(frame, body, block);
        let [list, strip] = ui::make_room(inner, self.toast.as_ref());
        self.render_place_rows(frame, list, total);
        ui::footer(frame, bottom, &PLACE_KEYS);
        if let Some(toast) = &self.toast {
            ui::show_toast(frame, strip, toast);
        }
        if let Some(index) = self.pending {
            let place = &self.summary.places[index];
            let node = self.browser.root().descend(&place.trail);
            let path = format::path(&place.path);
            ui::confirm_trash(frame, &name_of(&place.path), &path, node.size, node.items);
        }
    }

    fn render_place_rows(&mut self, frame: &mut Frame, area: Rect, total: u64) {
        let visible = self.places.window(area, self.shown.len());
        let label = self
            .shown
            .iter()
            .map(|&i| ui::width(self.summary.places[i].label))
            .max();
        let columns = PlaceColumns::fit(area.width.saturating_sub(1), label.unwrap_or(0));
        let rows: Vec<Row> = self.shown[visible]
            .iter()
            .map(|&index| place_row(&self.summary.places[index], total, &columns))
            .collect();
        let widths = [
            Constraint::Length(9),
            Constraint::Length(4),
            Constraint::Length(columns.bar),
            Constraint::Fill(1),
            Constraint::Length(columns.label),
        ];
        frame.render_stateful_widget(ui::list(rows, widths), area, &mut self.places.state());
    }
}

struct PlaceColumns {
    bar: u16,
    label: u16,
    path: usize,
}

impl PlaceColumns {
    fn fit(width: u16, label: u16) -> Self {
        let bar = ui::bar_width(width);
        let label = if bar == 0 { 0 } else { label };
        let path = usize::from(width.saturating_sub(9 + 4 + bar + label + 4));
        PlaceColumns { bar, label, path }
    }
}

fn place_row(place: &Place, total: u64, columns: &PlaceColumns) -> Row<'static> {
    let share = format::share(place.size, total);
    let note = access_note(place.is_blocked, place.denied);
    let room = columns
        .path
        .saturating_sub(note.as_ref().map_or(0, Span::width));
    let mut path = vec![Span::raw(ui::fit_start(&format::path(&place.path), room))];
    path.extend(note);
    let dim = Style::new().fg(DIM);
    Row::new([
        Cell::from(Line::styled(format::size(place.size), theme::size(place.size)).right_aligned()),
        Cell::from(Line::from(format::percent(share)).right_aligned()).style(dim),
        Cell::from(ui::bar_line(share, columns.bar)),
        Cell::from(Line::from(path)),
        Cell::from(Line::from(place.label).right_aligned()).style(dim),
    ])
}

fn access_note(is_blocked: bool, denied: u64) -> Option<Span<'static>> {
    let note = format::access(is_blocked, denied)?;
    let style = if is_blocked {
        Style::new().fg(WARN).add_modifier(Modifier::DIM)
    } else {
        Style::new().fg(DIM)
    };
    Some(Span::styled(format!("  {note}"), style))
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn hidden_reason(disk: &Disk) -> String {
    if disk.is_apfs {
        format!(
            "Snapshots and purgeable space stay hidden. {}",
            format::ACCESS_HINT
        )
    } else {
        format!("ruim could not read this. {}", format::ACCESS_HINT)
    }
}

pub fn stacked(parts: &[(u64, Color, bool)], width: u16) -> Line<'static> {
    let sizes: Vec<u64> = parts.iter().map(|part| part.0).collect();
    let cells = apportion(&sizes, usize::from(width));
    let spans: Vec<Span> = parts
        .iter()
        .zip(cells)
        .filter(|(_, cells)| *cells > 0)
        .map(|(&(_, color, is_free), cells)| {
            if is_free {
                Span::styled("·".repeat(cells), Style::new().fg(TRACK))
            } else {
                Span::styled("█".repeat(cells), Style::new().fg(color))
            }
        })
        .collect();
    Line::from(spans)
}

pub fn apportion(sizes: &[u64], width: usize) -> Vec<usize> {
    let total: u64 = sizes.iter().sum();
    if total == 0 {
        return vec![0; sizes.len()];
    }
    let exact: Vec<f64> = sizes
        .iter()
        .map(|&size| size as f64 * width as f64 / total as f64)
        .collect();
    let mut cells: Vec<usize> = exact.iter().map(|share| share.floor() as usize).collect();
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by(|&a, &b| (exact[b] - exact[b].floor()).total_cmp(&(exact[a] - exact[a].floor())));
    let left = width.saturating_sub(cells.iter().sum());
    for &index in order.iter().take(left) {
        cells[index] += 1;
    }
    cells
}

pub fn run() -> io::Result<()> {
    let disk = volumes::boot_disk()?;
    let progress = Arc::new(Progress::default());
    let worker: JoinHandle<io::Result<Scanned>> = {
        let (disk, progress) = (disk.clone(), Arc::clone(&progress));
        thread::spawn(move || scan_disk(&disk, &progress))
    };
    let mut terminal = ui::start()?;
    let result = session(&mut terminal, disk, &progress, worker);
    ui::stop();
    result
}

fn session(
    terminal: &mut DefaultTerminal,
    mut disk: Disk,
    progress: &Progress,
    worker: JoinHandle<io::Result<Scanned>>,
) -> io::Result<()> {
    let scanned = ui::wait(terminal, progress, worker, |frame, elapsed| {
        waiting(frame, &disk, progress, elapsed);
    })?;
    let Some(scanned) = scanned else {
        return Ok(());
    };
    disk.purgeable = scanned.purgeable;
    let mut app = App::new(disk, scanned.root, scanned.summary);
    loop {
        terminal.draw(|frame| app.render(frame))?;
        if app.handle(&ui::next_event()?) == Outcome::Quit {
            return Ok(());
        }
    }
}

fn waiting(frame: &mut Frame, disk: &Disk, progress: &Progress, elapsed: Duration) {
    let [top, body, bottom] = ui::screen(frame.area());
    ui::header(
        frame,
        top,
        std::slice::from_ref(&disk.name),
        &disk_summary(disk),
    );
    let scanned = progress.bytes().min(disk.data);
    let parts = [
        (scanned, ACCENT, false),
        (disk.data - scanned, TRACK, false),
        (disk.system, SYSTEM, false),
        (disk.other(), SYSTEM, false),
        (disk.free, TRACK, true),
    ];
    ui::scanning(frame, body, "the whole disk", progress, elapsed, |width| {
        Some(stacked(&parts, width))
    });
    ui::footer(frame, bottom, &ui::STOP_KEYS);
}
