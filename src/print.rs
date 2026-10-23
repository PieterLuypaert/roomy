use std::fmt::Write;
use std::path::Path;

use crate::format;
use crate::tree::{Kind, Node};

pub fn text(root: &Node, path: &Path, depth: usize) -> String {
    let mut out = format!("{:>9}  {}\n", format::size(root.size), format::path(path));
    text_children(&mut out, root, depth, 1);
    out
}

fn text_children(out: &mut String, node: &Node, depth: usize, level: usize) {
    if level > depth {
        return;
    }
    for child in &node.children {
        let share = format::percent(format::share(child.size, node.size));
        let indent = "  ".repeat(level - 1);
        let _ = writeln!(
            out,
            "{:>9} {share:>4}  {indent}{}",
            format::size(child.size),
            name(child)
        );
        text_children(out, child, depth, level + 1);
    }
}

fn name(node: &Node) -> String {
    let label = clean(&node.label());
    let slash = if node.is_dir() { "/" } else { "" };
    match node {
        n if n.is_denied => format!("{label}{slash}  (no access)"),
        n if n.is_unreadable && n.children.is_empty() => format!("{label}{slash}  (unreadable)"),
        n if n.is_mount => format!("{label}/  (other disk)"),
        _ => format!("{label}{slash}"),
    }
}

fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

pub fn json(root: &Node, path: &Path, depth: usize) -> String {
    let mut out = String::new();
    json_node(&mut out, root, Some(path), depth);
    out.push('\n');
    out
}

fn json_node(out: &mut String, node: &Node, path: Option<&Path>, depth: usize) {
    out.push('{');
    match path {
        Some(path) => field(out, "path", &path.to_string_lossy()),
        None => field(out, "name", &node.label()),
    }
    let _ = write!(
        out,
        ",\"kind\":\"{}\",\"size\":{}",
        kind(node.kind),
        node.size
    );
    if node.is_dir() {
        let _ = write!(out, ",\"items\":{}", node.items);
    }
    flag(out, "denied", node.is_denied);
    flag(out, "unreadable", node.is_unreadable);
    flag(out, "otherDisk", node.is_mount);
    if depth > 0 && !node.children.is_empty() {
        out.push_str(",\"children\":[");
        for (i, child) in node.children.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            json_node(out, child, None, depth - 1);
        }
        out.push(']');
    }
    out.push('}');
}

fn field(out: &mut String, key: &str, value: &str) {
    let _ = write!(out, "\"{key}\":\"{}\"", escape(value));
}

fn flag(out: &mut String, key: &str, is_set: bool) {
    if is_set {
        let _ = write!(out, ",\"{key}\":true");
    }
}

fn kind(kind: Kind) -> &'static str {
    match kind {
        Kind::Dir => "dir",
        Kind::File => "file",
        Kind::Link => "link",
        Kind::Other => "other",
    }
}

pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Node {
        let mut locked = Node::dir("locked", 0);
        locked.is_denied = true;
        Node::with_children(
            "root",
            vec![
                Node::with_children("photos", vec![Node::leaf("a.jpg", Kind::File, 3000)]),
                Node::leaf("notes.txt", Kind::File, 1000),
                locked,
            ],
        )
    }

    #[test]
    fn text_lists_children_biggest_first_with_shares() {
        let out = text(&sample(), Path::new("/data"), 1);

        assert_eq!(
            out,
            "   4.0 kB  /data\n   3.0 kB  75%  photos/\n   1.0 kB  25%  notes.txt\n      0 B       locked/  (no access)\n"
        );
    }

    #[test]
    fn text_indents_deeper_levels() {
        let out = text(&sample(), Path::new("/data"), 2);

        assert!(out.contains("\n   3.0 kB 100%    a.jpg\n"), "{out}");
    }

    #[test]
    fn json_stops_at_the_requested_depth() {
        let out = json(&sample(), Path::new("/data"), 1);

        assert!(out.starts_with("{\"path\":\"/data\",\"kind\":\"dir\",\"size\":4000,\"items\":4,"));
        assert!(out.contains("{\"name\":\"photos\",\"kind\":\"dir\",\"size\":3000,\"items\":1}"));
        assert!(out.contains(
            "\"name\":\"locked\",\"kind\":\"dir\",\"size\":0,\"items\":0,\"denied\":true"
        ));
    }

    #[test]
    fn text_replaces_control_characters_in_names() {
        let root = Node::with_children("root", vec![Node::leaf("a\u{1b}[31m\nb", Kind::File, 10)]);

        let out = text(&root, Path::new("/data"), 1);

        assert!(out.ends_with("  a?[31m?b\n"), "{out:?}");
    }

    #[test]
    fn escape_handles_quotes_and_control_characters() {
        assert_eq!(escape("a\"b\\c\nd\u{1}"), "a\\\"b\\\\c\\nd\\u0001");
    }
}
