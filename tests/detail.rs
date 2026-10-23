use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use ruim::tree::{Kind, Node};
use ruim::ui::Browser;

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
