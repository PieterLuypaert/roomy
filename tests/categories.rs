use std::path::Path;

use ruim::categories::{Group, Summary, summarize};
use ruim::tree::{Kind, Node};

fn file(name: &str, size: u64) -> Node {
    Node::leaf(name, Kind::File, size)
}

fn dir(name: &str, children: Vec<Node>) -> Node {
    Node::with_children(name, children)
}

fn data_volume() -> Node {
    let mut mail = Node::dir("Mail", 0);
    mail.is_denied = true;
    let project = dir(
        "proj",
        vec![
            dir(
                "node_modules",
                vec![
                    file("lib.js", 2_000),
                    dir("node_modules", vec![file("dep.js", 1_000)]),
                ],
            ),
            file("package.json", 100),
        ],
    );
    let library = dir(
        "Library",
        vec![
            dir("Caches", vec![dir("com.app", vec![file("blob", 300)])]),
            mail,
            dir(
                "Application Support",
                vec![dir(
                    "Code",
                    vec![dir("node_modules", vec![file("x.js", 400)])],
                )],
            ),
        ],
    );
    let ann = dir(
        "ann",
        vec![dir("Documents", vec![project]), library, file(".zshrc", 10)],
    );
    dir(
        "/System/Volumes/Data",
        vec![
            dir(
                "Applications",
                vec![dir("Big.app", vec![file("binary", 5_000)])],
            ),
            dir("Users", vec![ann, dir("bob", vec![file("notes", 50)])]),
            dir("private", vec![dir("var", vec![file("log.db", 70)])]),
        ],
    )
}

fn total(summary: &Summary, group: Group) -> u64 {
    summary
        .totals
        .iter()
        .find(|total| total.group == group)
        .map_or(0, |total| total.size)
}

fn place_paths(summary: &Summary, group: Group) -> Vec<String> {
    summary
        .place_indices(group)
        .into_iter()
        .map(|index| summary.places[index].path.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn every_byte_lands_in_exactly_one_group() {
    let root = data_volume();

    let summary = summarize(
        &root,
        Path::new("/System/Volumes/Data"),
        Path::new("/Users/ann"),
    );

    assert_eq!(summary.size(), root.size);
    assert_eq!(total(&summary, Group::Builds), 3_000);
    assert_eq!(total(&summary, Group::Documents), 100);
    assert_eq!(total(&summary, Group::Apps), 5_000);
    assert_eq!(total(&summary, Group::Caches), 300);
    assert_eq!(total(&summary, Group::AppData), 400);
    assert_eq!(total(&summary, Group::Home), 10);
    assert_eq!(total(&summary, Group::Users), 50);
    assert_eq!(total(&summary, Group::System), 70);
}

#[test]
fn nested_dependencies_stay_inside_their_project_folder() {
    let root = data_volume();

    let summary = summarize(
        &root,
        Path::new("/System/Volumes/Data"),
        Path::new("/Users/ann"),
    );

    assert_eq!(
        place_paths(&summary, Group::Builds),
        ["/System/Volumes/Data/Users/ann/Documents/proj/node_modules"]
    );
    let index = summary.place_indices(Group::Builds)[0];
    let node = root.descend(&summary.places[index].trail);
    assert_eq!(node.label(), "node_modules");
    assert_eq!(node.size, 3_000);
}

#[test]
fn split_folders_list_each_child_as_a_place() {
    let root = data_volume();

    let summary = summarize(
        &root,
        Path::new("/System/Volumes/Data"),
        Path::new("/Users/ann"),
    );

    assert_eq!(
        place_paths(&summary, Group::Apps),
        ["/System/Volumes/Data/Applications/Big.app"]
    );
    assert_eq!(
        place_paths(&summary, Group::AppData),
        ["/System/Volumes/Data/Users/ann/Library/Application Support/Code"]
    );
    assert_eq!(
        place_paths(&summary, Group::Home),
        ["/System/Volumes/Data/Users/ann/.zshrc"]
    );
}

#[test]
fn locked_places_are_kept_and_flagged() {
    let root = data_volume();

    let summary = summarize(
        &root,
        Path::new("/System/Volumes/Data"),
        Path::new("/Users/ann"),
    );

    let mail = summary
        .totals
        .iter()
        .find(|t| t.group == Group::Mail)
        .unwrap();
    assert_eq!((mail.size, mail.places, mail.denied), (0, 1, 1));
    assert!(mail.is_blocked);
    let caches = summary
        .totals
        .iter()
        .find(|t| t.group == Group::Caches)
        .unwrap();
    assert!(!caches.is_blocked);
}

#[test]
fn only_marked_target_folders_count_as_rust_builds() {
    let built = dir(
        "proj",
        vec![dir(
            "target",
            vec![file("CACHEDIR.TAG", 10), file("debug", 1_000)],
        )],
    );
    let plain = dir("other", vec![dir("target", vec![file("aim.txt", 500)])]);
    let root = dir("/", vec![built, plain]);

    let summary = summarize(&root, Path::new("/"), Path::new("/home/ann"));

    assert_eq!(total(&summary, Group::Builds), 1_010);
    assert_eq!(total(&summary, Group::System), 500);
}

#[test]
fn global_installs_are_not_project_dependencies() {
    let global = dir(
        "lib",
        vec![dir(
            "node_modules",
            vec![dir("npm", vec![file("cli.js", 700)])],
        )],
    );
    let project = dir(
        "site",
        vec![
            file("package.json", 1),
            dir("node_modules", vec![file("react.js", 300)]),
        ],
    );
    let root = dir("/", vec![dir("opt", vec![global]), project]);

    let summary = summarize(&root, Path::new("/"), Path::new("/home/ann"));

    assert_eq!(total(&summary, Group::Builds), 300);
    assert_eq!(total(&summary, Group::Tools), 700);
    assert_eq!(total(&summary, Group::System), 1);
}
