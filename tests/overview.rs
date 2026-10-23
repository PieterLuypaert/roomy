use std::path::{Path, PathBuf};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use roomy::categories::{Group, summarize};
use roomy::overview::{App, Part, apportion, overcount, scale, slices};
use roomy::print;
use roomy::tree::{Kind, Node};
use roomy::ui::Outcome;
use roomy::volumes::Disk;

fn disk(data: u64) -> Disk {
    Disk {
        name: "Test HD".to_string(),
        total: 100_000,
        free: 60_000,
        purgeable: Some(2_000),
        system: 5_000,
        data,
        scan_root: PathBuf::from("/"),
        is_apfs: true,
    }
}

fn tree() -> Node {
    let modules = Node::with_children("node_modules", vec![Node::leaf("a.js", Kind::File, 3_000)]);
    let project = Node::with_children(
        "app",
        vec![modules, Node::leaf("package.json", Kind::File, 500)],
    );
    let documents = Node::with_children("Documents", vec![project]);
    let home = Node::with_children("ann", vec![documents]);
    let users = Node::with_children("Users", vec![home]);
    let apps = Node::with_children(
        "Applications",
        vec![Node::with_children(
            "Big.app",
            vec![Node::leaf("bin", Kind::File, 20_000)],
        )],
    );
    Node::with_children("/", vec![users, apps])
}

fn app() -> App {
    let root = tree();
    let summary = summarize(&root, Path::new("/"), Path::new("/Users/ann"));
    App::new(disk(30_500), root, summary)
}

fn press(app: &mut App, code: KeyCode) -> Outcome {
    app.handle(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn screen(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(110, 16)).unwrap();
    terminal.draw(|frame| app.render(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..16)
        .map(|y| {
            (0..110)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn slices_add_up_to_the_whole_disk() {
    let root = tree();
    let summary = summarize(&root, Path::new("/"), Path::new("/Users/ann"));

    let parts = slices(&disk(30_500), &summary, 0);

    let sizes: Vec<(Part, u64)> = parts.iter().map(|s| (s.part, s.size)).collect();
    assert_eq!(
        sizes,
        [
            (Part::Group(Group::Apps), 20_000),
            (Part::Group(Group::Builds), 3_000),
            (Part::Group(Group::Documents), 500),
            (Part::Hidden, 7_000),
            (Part::System, 5_000),
            (Part::Reserved, 4_500),
            (Part::Free, 60_000),
        ]
    );
    assert_eq!(parts.iter().map(|s| s.size).sum::<u64>(), 100_000);
}

#[test]
fn nothing_is_hidden_when_the_scan_saw_everything() {
    let root = tree();
    let summary = summarize(&root, Path::new("/"), Path::new("/Users/ann"));

    let parts = slices(&disk(20_000), &summary, 0);

    assert!(parts.iter().all(|slice| slice.part != Part::Hidden));
    assert_eq!(overcount(&disk(20_000), &summary, 0), 3_500);
    assert_eq!(scale(&disk(20_000), &parts), 103_500);
}

#[test]
fn apportion_fills_the_width_in_proportion() {
    assert_eq!(apportion(&[50, 25, 25], 8), [4, 2, 2]);
    assert_eq!(apportion(&[1, 1, 1], 10).iter().sum::<usize>(), 10);
    assert_eq!(apportion(&[0, 0], 5), [0, 0]);
}

#[test]
fn the_overview_lists_groups_and_disk_parts() {
    let mut app = app();

    let text = screen(&mut app);

    assert!(text.contains("Test HD"));
    assert!(text.contains("Apps"));
    assert!(text.contains("Project dependencies & builds"));
    assert!(text.contains("Hidden from roomy"));
    press(&mut app, KeyCode::End);
    assert!(screen(&mut app).contains("2.0 kB more can be freed"));
}

#[test]
fn opening_a_group_then_a_place_reaches_the_folder_and_back() {
    let mut app = app();

    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.group(), Some(Group::Builds));
    assert!(screen(&mut app).contains("/Users/ann/Documents/app/node_modules"));

    press(&mut app, KeyCode::Enter);
    let browser = app.browser().unwrap();
    assert_eq!(
        browser.path(),
        PathBuf::from("/Users/ann/Documents/app/node_modules")
    );
    let text = screen(&mut app);
    assert!(
        text.contains("Test HD › Project dependencies & builds › node_modules"),
        "{text}"
    );
    assert!(
        text.contains("─ /Users/ann/Documents/app/node_modules ─"),
        "{text}"
    );

    press(&mut app, KeyCode::Left);
    assert_eq!(app.group(), Some(Group::Builds));

    press(&mut app, KeyCode::Esc);
    assert_eq!(app.group(), None);
    assert!(app.browser().is_none());
}

#[test]
fn b_browses_the_whole_disk_and_back_returns_to_the_overview() {
    let mut app = app();

    press(&mut app, KeyCode::Char('b'));
    assert_eq!(app.browser().unwrap().path(), PathBuf::from("/"));
    assert!(screen(&mut app).contains("roomy   Test HD "));
    press(&mut app, KeyCode::Enter);
    assert!(screen(&mut app).contains("Test HD › Applications"));
    press(&mut app, KeyCode::Left);

    assert_eq!(press(&mut app, KeyCode::Left), Outcome::Continue);
    assert!(app.browser().is_none());
    assert_eq!(press(&mut app, KeyCode::Char('q')), Outcome::Quit);
}

#[test]
fn disk_parts_explain_themselves_instead_of_opening() {
    let mut app = app();

    for _ in 0..3 {
        press(&mut app, KeyCode::Down);
    }
    press(&mut app, KeyCode::Enter);

    assert!(app.group().is_none());
    assert!(app.status().unwrap().contains("Snapshots"));
}

#[test]
fn the_text_summary_lists_parts_and_top_places() {
    let root = tree();
    let summary = summarize(&root, Path::new("/"), Path::new("/Users/ann"));

    let out = print::summary_text(&disk(30_500), &summary, 2);

    assert!(out.starts_with("Test HD  100 kB · 40.0 kB used · 60.0 kB free (2.0 kB purgeable)\n"));
    assert!(out.contains("  20.0 kB  20%  Apps  (1 place)\n"), "{out}");
    assert!(
        out.contains("/Users/ann/Documents/app/node_modules  (node_modules)"),
        "{out}"
    );
    assert!(out.contains("   7.0 kB   7%  Hidden from roomy\n"), "{out}");
}

#[test]
fn the_text_summary_flags_space_counted_twice() {
    let root = tree();
    let summary = summarize(&root, Path::new("/"), Path::new("/Users/ann"));

    let out = print::summary_text(&disk(20_000), &summary, 1);

    assert!(
        out.ends_with("Note: 3.5 kB counted twice (APFS clones).\n"),
        "{out}"
    );
}

fn keep_quiet(_: &Path) -> std::io::Result<()> {
    Ok(())
}

#[test]
fn trashing_a_place_moves_its_bytes_to_the_trash_group() {
    let mut app = app().with_trasher(keep_quiet);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);

    press(&mut app, KeyCode::Char('d'));
    assert!(screen(&mut app).contains("Move to the Trash?"));
    press(&mut app, KeyCode::Char('y'));

    assert!(
        app.status()
            .unwrap()
            .contains("Moved node_modules (3.0 kB)")
    );
    assert!(!screen(&mut app).contains("node_modules  "));
    press(&mut app, KeyCode::Esc);
    let text = screen(&mut app);
    assert!(text.contains("Trash"), "{text}");
    assert!(
        text.contains("   7.0 kB   7% ■ Hidden from roomy"),
        "{text}"
    );
}

#[test]
fn rows_that_stand_for_loose_files_are_not_trashed() {
    let mut ann = Node::dir("ann", 4_096);
    ann.children.push(Node::leaf("notes.txt", Kind::File, 50));
    ann.finish();
    let root = Node::with_children("/", vec![Node::with_children("Users", vec![ann])]);
    let summary = summarize(&root, Path::new("/"), Path::new("/Users/ann"));
    let mut app = App::new(disk(30_500), root, summary).with_trasher(keep_quiet);
    press(&mut app, KeyCode::Enter);

    press(&mut app, KeyCode::Char('d'));

    assert!(
        app.status().unwrap().contains("loose files"),
        "{:?}",
        app.status()
    );
}

#[test]
fn moving_a_folder_the_browser_was_inside_does_not_break_it() {
    let mut app = app().with_trasher(keep_quiet);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter);
    assert!(app.browser().is_some());
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Esc);

    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.group(), Some(Group::Documents));
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Char('y'));
    assert!(
        app.status().unwrap().contains("Moved app"),
        "{:?}",
        app.status()
    );

    press(&mut app, KeyCode::Char('b'));
    screen(&mut app);
    assert_eq!(app.browser().unwrap().path(), PathBuf::from("/"));
}

#[test]
fn question_mark_lists_the_overview_keys_and_the_closing_key_does_nothing_else() {
    let mut app = app();

    press(&mut app, KeyCode::Char('?'));
    assert!(screen(&mut app).contains("browse the whole disk"));
    press(&mut app, KeyCode::Down);
    assert!(!screen(&mut app).contains("browse the whole disk"));

    press(&mut app, KeyCode::Enter);
    assert_eq!(app.group(), Some(Group::Apps));
}
