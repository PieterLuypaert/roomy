use std::path::{Path, PathBuf};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use ruim::categories::summarize;
use ruim::overview::App;
use ruim::tree::{Kind, Node};
use ruim::ui::Browser;
use ruim::volumes::Disk;

fn browser() -> Browser {
    let modules = Node::with_children(
        "node_modules",
        vec![
            Node::leaf("react.js", Kind::File, 6_000),
            Node::leaf("tiny.js", Kind::File, 1_000),
        ],
    );
    let app = Node::with_children(
        "app",
        vec![modules, Node::leaf("logo.png", Kind::File, 2_000)],
    );
    let root = Node::with_children(
        "/data",
        vec![app, Node::leaf("notes.txt", Kind::File, 1_000)],
    );
    Browser::new(root, PathBuf::from("/data"))
}

fn text(width: u16, height: u16, draw: impl FnOnce(&mut ratatui::Frame)) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(draw).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect()
}

fn press(browser: &mut Browser, code: KeyCode) {
    browser.handle(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

#[test]
fn a_wide_screen_shows_what_the_selected_folder_holds() {
    let mut browser = browser();

    let lines = text(120, 14, |frame| browser.render(frame));
    let screen = lines.join("\n");

    assert!(lines[1].contains("─ app/ ─"), "{}", lines[1]);
    assert!(screen.contains("9.0 kB · 90% of data"), "{screen}");
    assert!(screen.contains("4 items"), "{screen}");
    assert!(screen.contains("biggest inside"), "{screen}");
    let modules = lines.iter().position(|line| line.contains("node_modules/"));
    let logo = lines.iter().position(|line| line.contains("logo.png"));
    assert!(modules < logo, "{screen}");
}

#[test]
fn the_pane_follows_the_cursor_to_a_file() {
    let mut browser = browser();
    press(&mut browser, KeyCode::Down);

    let screen = text(120, 14, |frame| browser.render(frame)).join("\n");

    assert!(screen.contains("─ notes.txt ─"), "{screen}");
    assert!(screen.contains("1.0 kB · 10% of data"), "{screen}");
    assert!(screen.contains("file"), "{screen}");
    assert!(!screen.contains("biggest inside"), "{screen}");
}

#[test]
fn the_list_hands_its_item_counts_to_the_pane() {
    let mut browser = browser();

    let wide = text(120, 14, |frame| browser.render(frame));
    let narrow = text(90, 14, |frame| browser.render(frame));

    assert!(!wide[2].contains("4 items"), "{}", wide[2]);
    assert!(narrow[2].contains("4 items"), "{}", narrow[2]);
    assert!(!narrow.join("\n").contains("biggest inside"));
}

fn disk() -> Disk {
    Disk {
        name: "Test HD".to_string(),
        total: 100_000,
        free: 60_000,
        purgeable: None,
        system: 5_000,
        data: 30_500,
        scan_root: PathBuf::from("/"),
        is_apfs: true,
    }
}

fn app() -> App {
    let modules = Node::with_children("node_modules", vec![Node::leaf("a.js", Kind::File, 3_000)]);
    let project = Node::with_children(
        "app",
        vec![modules, Node::leaf("package.json", Kind::File, 500)],
    );
    let home = Node::with_children("ann", vec![Node::with_children("Documents", vec![project])]);
    let root = Node::with_children("/", vec![Node::with_children("Users", vec![home])]);
    let summary = summarize(&root, Path::new("/"), Path::new("/Users/ann"));
    App::new(disk(), root, summary)
}

#[test]
fn the_overview_pane_describes_a_group_and_its_biggest_places() {
    let mut app = app();

    let screen = text(120, 16, |frame| app.render(frame)).join("\n");

    assert!(
        screen.contains("─ Project dependencies & builds ─"),
        "{screen}"
    );
    assert!(screen.contains("of the disk"), "{screen}");
    assert!(screen.contains("node_modules, build output"), "{screen}");
    assert!(screen.contains("biggest places"), "{screen}");
    assert!(screen.contains("app/node_modules"), "{screen}");
}

#[test]
fn the_places_pane_shows_what_a_place_holds() {
    let mut app = app();
    app.handle(&Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));

    let screen = text(120, 16, |frame| app.render(frame)).join("\n");

    assert!(screen.contains("─ node_modules/ ─"), "{screen}");
    assert!(screen.contains("100% of Project"), "{screen}");
    assert!(screen.contains("a.js"), "{screen}");
}

#[test]
fn a_loose_files_row_is_not_described_as_its_whole_folder() {
    let mut ann = Node::dir("ann", 4_096);
    ann.children.push(Node::leaf("notes.txt", Kind::File, 50));
    ann.finish();
    let root = Node::with_children("/", vec![Node::with_children("Users", vec![ann])]);
    let summary = summarize(&root, Path::new("/"), Path::new("/Users/ann"));
    let mut app = App::new(disk(), root, summary);
    app.handle(&Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));

    let screen = text(120, 16, |frame| app.render(frame)).join("\n");

    assert!(
        screen.contains("the loose files in this folder"),
        "{screen}"
    );
    assert!(!screen.contains("1 item"), "{screen}");
    assert!(!screen.contains("biggest inside"), "{screen}");
}
