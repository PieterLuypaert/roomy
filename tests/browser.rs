use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use roomy::tree::{Kind, Node};
use roomy::ui::{Browser, Outcome};

fn sample() -> Browser {
    let mut locked = Node::dir("locked", 0);
    locked.is_denied = true;
    let documents = Node::with_children(
        "Documents",
        vec![
            Node::leaf("report.pdf", Kind::File, 3_000),
            Node::leaf("notes.txt", Kind::File, 1_000),
        ],
    );
    let root = Node::with_children(
        "/data",
        vec![
            documents,
            Node::leaf("movie.mp4", Kind::File, 6_000),
            locked,
        ],
    );
    Browser::new(root, PathBuf::from("/data"))
}

fn screen(browser: &mut Browser) -> Vec<String> {
    screen_of(browser, 90, 10)
}

fn screen_of(browser: &mut Browser, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| browser.render(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn press(browser: &mut Browser, code: KeyCode) -> Outcome {
    browser.handle(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn click(browser: &mut Browser, row: u16) {
    browser.handle(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 20,
        row,
        modifiers: KeyModifiers::NONE,
    }));
}

#[test]
fn shows_the_folder_biggest_first() {
    let mut browser = sample();

    let lines = screen(&mut browser);

    assert!(lines[0].contains("/ › data"), "{}", lines[0]);
    assert!(lines[0].contains("10.0 kB"), "{}", lines[0]);
    assert!(lines[1].contains("data ─ 3 items"), "{}", lines[1]);
    assert!(
        lines[2].contains("6.0 kB") && lines[2].contains("60%") && lines[2].contains("movie.mp4")
    );
    assert!(lines[3].contains("Documents/") && lines[3].contains("2 items"));
    assert!(lines[4].contains("locked/  no access"));
    assert!(lines[8].contains("1 no access"), "{}", lines[8]);
}

#[test]
fn opens_a_folder_and_comes_back_to_it() {
    let mut browser = sample();

    press(&mut browser, KeyCode::Down);
    press(&mut browser, KeyCode::Enter);

    assert_eq!(browser.path(), PathBuf::from("/data/Documents"));
    let lines = screen(&mut browser);
    assert!(lines[2].contains("report.pdf"));
    assert!(lines[3].contains("notes.txt"));

    press(&mut browser, KeyCode::Left);

    assert_eq!(browser.path(), PathBuf::from("/data"));
    assert_eq!(browser.selected().unwrap().label(), "Documents");
}

#[test]
fn explains_why_a_locked_folder_does_not_open() {
    let mut browser = sample();

    press(&mut browser, KeyCode::End);
    press(&mut browser, KeyCode::Enter);

    assert_eq!(browser.path(), PathBuf::from("/data"));
    assert!(browser.status().unwrap().contains("No access to locked"));
    let mut lines = screen(&mut browser);
    let footer = lines.pop().unwrap();
    assert!(lines.join("\n").contains("No access to locked"));
    assert!(footer.contains("quit"), "{footer}");
}

#[test]
fn sorting_by_name_keeps_the_selection() {
    let mut browser = sample();
    press(&mut browser, KeyCode::Down);

    press(&mut browser, KeyCode::Char('s'));

    let lines = screen(&mut browser);
    assert!(lines[2].contains("Documents/"));
    assert!(lines[4].contains("movie.mp4"));
    assert_eq!(browser.selected().unwrap().label(), "Documents");
    assert!(lines[1].contains("by name"), "{}", lines[1]);
}

#[test]
fn a_click_selects_and_a_second_click_opens() {
    let mut browser = sample();
    screen(&mut browser);

    click(&mut browser, 3);
    assert_eq!(browser.selected().unwrap().label(), "Documents");

    click(&mut browser, 3);
    assert_eq!(browser.path(), PathBuf::from("/data/Documents"));
}

#[test]
fn q_quits() {
    let mut browser = sample();

    assert_eq!(press(&mut browser, KeyCode::Down), Outcome::Continue);
    assert_eq!(press(&mut browser, KeyCode::Char('q')), Outcome::Quit);
}

#[test]
fn long_item_counts_are_not_cut_off() {
    let mut many = Node::dir("node_modules", 5_000);
    many.items = 1_982_471;
    let root = Node::with_children("/data", vec![many, Node::leaf("a.txt", Kind::File, 1)]);
    let mut browser = Browser::new(root, PathBuf::from("/data"));

    let lines = screen(&mut browser);

    assert!(lines[2].contains("1,982,471 items"), "{}", lines[2]);
}

#[test]
fn a_long_folder_name_leaves_room_for_the_count() {
    let long = "a-folder-with-a-name-far-too-long-for-the-border-of-the-panel-that-it-gives-a-title-to-here";
    let inside = Node::with_children(long, vec![Node::leaf("a.txt", Kind::File, 1)]);
    let mut browser = Browser::new(
        Node::with_children("/data", vec![inside]),
        PathBuf::from("/data"),
    );
    press(&mut browser, KeyCode::Enter);

    let lines = screen(&mut browser);

    assert!(lines[1].contains("… ─ 1 item"), "{}", lines[1]);
}

#[test]
fn scrolls_to_keep_the_cursor_on_screen() {
    let files = (0..50)
        .map(|i| Node::leaf(format!("file-{i:02}"), Kind::File, 1_000 + i))
        .collect();
    let mut browser = Browser::new(Node::with_children("/data", files), PathBuf::from("/data"));
    screen(&mut browser);

    press(&mut browser, KeyCode::End);
    let lines = screen(&mut browser);

    assert!(lines[7].contains("file-00"), "{}", lines[7]);
    assert_eq!(browser.selected().unwrap().label(), "file-00");

    click(&mut browser, 2);
    assert_eq!(browser.selected().unwrap().label(), "file-05");
}

#[test]
fn jumping_to_a_locked_folder_shows_why_from_its_parent() {
    let mut browser = sample();

    browser.jump(vec![2], None);

    assert_eq!(browser.path(), PathBuf::from("/data"));
    assert_eq!(browser.selected().unwrap().label(), "locked");
    assert!(browser.status().unwrap().contains("No access to locked"));
}

#[test]
fn a_click_clears_the_last_message() {
    let mut browser = sample();
    screen(&mut browser);
    press(&mut browser, KeyCode::End);
    press(&mut browser, KeyCode::Enter);
    assert!(browser.status().is_some());

    click(&mut browser, 2);

    assert!(browser.status().is_none());
}

static MOVED: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

fn record(path: &std::path::Path) -> std::io::Result<()> {
    MOVED.lock().unwrap().push(path.to_path_buf());
    Ok(())
}

fn refuse(_: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::Error::other("the disk is read-only"))
}

#[test]
fn trash_asks_first_and_then_moves_the_entry() {
    let mut browser = sample().with_trasher(record);

    press(&mut browser, KeyCode::Char('d'));
    assert!(
        screen(&mut browser)
            .join("\n")
            .contains("Move to the Trash?")
    );
    press(&mut browser, KeyCode::Char('y'));

    assert!(
        MOVED
            .lock()
            .unwrap()
            .contains(&PathBuf::from("/data/movie.mp4"))
    );
    assert_eq!(browser.current().size, 4_000);
    assert_eq!(browser.selected().unwrap().label(), "Documents");
    assert!(
        browser
            .status()
            .unwrap()
            .contains("Moved movie.mp4 (6.0 kB) to the Trash")
    );
    assert_eq!(browser.take_trashed(), 6_000);
    assert!(!screen(&mut browser).join("\n").contains("movie.mp4  "));
}

#[test]
fn any_other_key_keeps_the_entry() {
    let mut browser = sample().with_trasher(refuse);

    press(&mut browser, KeyCode::Char('d'));
    press(&mut browser, KeyCode::Char('n'));

    assert_eq!(browser.current().size, 10_000);
    assert_eq!(browser.selected().unwrap().label(), "movie.mp4");
    assert_eq!(browser.take_trashed(), 0);
}

#[test]
fn a_failed_move_leaves_the_tree_alone() {
    let mut browser = sample().with_trasher(refuse);

    press(&mut browser, KeyCode::Char('d'));
    press(&mut browser, KeyCode::Char('y'));

    assert_eq!(browser.current().size, 10_000);
    assert!(browser.status().unwrap().contains("the disk is read-only"));
}

#[test]
fn locked_folders_are_not_offered_for_the_trash() {
    let mut browser = sample().with_trasher(refuse);

    press(&mut browser, KeyCode::End);
    press(&mut browser, KeyCode::Char('d'));

    assert!(
        !screen(&mut browser)
            .join("\n")
            .contains("Move to the Trash?")
    );
    assert!(
        browser
            .status()
            .unwrap()
            .contains("cannot see inside locked")
    );
}

fn keep(_: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[test]
fn enter_does_not_confirm_a_move() {
    let mut browser = sample().with_trasher(keep);

    press(&mut browser, KeyCode::Char('d'));
    press(&mut browser, KeyCode::Enter);

    assert_eq!(browser.current().size, 10_000);
    assert_eq!(browser.take_trashed(), 0);
}

#[test]
fn size_order_follows_a_move() {
    let big = Node::with_children(
        "big",
        vec![
            Node::leaf("huge.bin", Kind::File, 9_000),
            Node::leaf("small.bin", Kind::File, 1_000),
        ],
    );
    let root = Node::with_children("/data", vec![big, Node::leaf("mid.bin", Kind::File, 5_000)]);
    let mut browser = Browser::new(root, PathBuf::from("/data")).with_trasher(keep);

    press(&mut browser, KeyCode::Enter);
    press(&mut browser, KeyCode::Char('d'));
    press(&mut browser, KeyCode::Char('y'));
    press(&mut browser, KeyCode::Left);

    let lines = screen(&mut browser);
    assert!(lines[2].contains("mid.bin"), "{}", lines[2]);
    assert!(lines[3].contains("big/"), "{}", lines[3]);
}

#[test]
fn a_folder_emptied_by_moves_says_so() {
    let only = Node::with_children("box", vec![Node::leaf("thing", Kind::File, 10)]);
    let mut browser = Browser::new(
        Node::with_children("/data", vec![only]),
        PathBuf::from("/data"),
    )
    .with_trasher(keep);

    press(&mut browser, KeyCode::Enter);
    press(&mut browser, KeyCode::Char('d'));
    press(&mut browser, KeyCode::Char('y'));

    assert!(screen(&mut browser).join("\n").contains("Empty folder"));
}

#[test]
fn question_mark_shows_every_key_and_any_key_closes_it() {
    let mut browser = sample();

    press(&mut browser, KeyCode::Char('?'));
    let open = screen_of(&mut browser, 90, 24).join("\n");
    assert!(open.contains("All keys"), "{open}");
    assert!(open.contains("show in the Finder"), "{open}");
    let short = screen_of(&mut browser, 90, 14).join("\n");
    assert!(
        short.contains("up and down") && short.contains("quit"),
        "{short}"
    );
    assert!(short.contains("trash, after asking"), "{short}");
    let tiny = screen_of(&mut browser, 90, 10).join("\n");
    assert!(tiny.contains("a taller window shows them all"), "{tiny}");

    assert_eq!(press(&mut browser, KeyCode::Char('q')), Outcome::Continue);
    assert!(!screen(&mut browser).join("\n").contains("All keys"));
    assert_eq!(browser.selected().unwrap().label(), "movie.mp4");
    assert_eq!(press(&mut browser, KeyCode::Char('q')), Outcome::Quit);
}

#[test]
fn the_key_row_keeps_all_keys_and_quit_when_it_runs_out_of_room() {
    let mut browser = sample();

    let wide = screen(&mut browser).pop().unwrap();
    let narrow = screen_of(&mut browser, 50, 10).pop().unwrap();

    assert!(wide.contains("trash") && !wide.contains("finder"), "{wide}");
    assert!(
        narrow.contains("all keys") && narrow.contains("quit"),
        "{narrow}"
    );
    assert!(!narrow.contains("trash"), "{narrow}");
}

#[test]
fn ctrl_c_quits_even_with_the_keys_open() {
    let mut browser = sample();
    press(&mut browser, KeyCode::Char('?'));

    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

    assert_eq!(browser.handle(&Event::Key(ctrl_c)), Outcome::Quit);
}
