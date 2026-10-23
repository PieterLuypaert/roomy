use std::borrow::Cow;
use std::ffi::OsStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    Link,
    Other,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: Box<OsStr>,
    pub kind: Kind,
    pub size: u64,
    pub items: u64,
    pub is_denied: bool,
    pub is_unreadable: bool,
    pub is_mount: bool,
    pub is_trashed: bool,
    pub children: Vec<Node>,
}

impl Node {
    pub fn leaf(name: impl AsRef<OsStr>, kind: Kind, size: u64) -> Self {
        Node {
            name: name.as_ref().into(),
            kind,
            size,
            items: 0,
            is_denied: false,
            is_unreadable: false,
            is_mount: false,
            is_trashed: false,
            children: Vec::new(),
        }
    }

    pub fn dir(name: impl AsRef<OsStr>, size: u64) -> Self {
        Node::leaf(name, Kind::Dir, size)
    }

    pub fn with_children(name: impl AsRef<OsStr>, children: Vec<Node>) -> Self {
        let mut node = Node::dir(name, 0);
        node.children = children;
        node.finish();
        node
    }

    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir
    }

    pub fn label(&self) -> Cow<'_, str> {
        self.name.to_string_lossy()
    }

    pub fn finish(&mut self) {
        self.size += self.children.iter().map(|child| child.size).sum::<u64>();
        self.items =
            self.children.len() as u64 + self.children.iter().map(|c| c.items).sum::<u64>();
        self.children
            .sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
        self.children.shrink_to_fit();
    }

    pub fn get(&self, trail: &[usize]) -> Option<&Node> {
        trail
            .iter()
            .try_fold(self, |node, &index| node.children.get(index))
    }

    pub fn descend(&self, trail: &[usize]) -> &Node {
        trail
            .iter()
            .fold(self, |node, &index| &node.children[index])
    }

    /// Empties the entry at `trail` in place, so every other trail into the tree stays valid.
    pub fn trash(&mut self, trail: &[usize]) -> Option<u64> {
        let (&last, parents) = trail.split_last()?;
        let mut node = &mut *self;
        for &index in parents {
            node = node.children.get_mut(index)?;
        }
        let target = node
            .children
            .get_mut(last)
            .filter(|target| !target.is_trashed)?;
        let (size, items) = (target.size, target.items + 1);
        *target = Node::leaf(&*target.name, target.kind, 0);
        target.is_trashed = true;
        let mut node = &mut *self;
        node.shrink(size, items);
        for &index in parents {
            node = &mut node.children[index];
            node.shrink(size, items);
        }
        Some(size)
    }

    fn shrink(&mut self, size: u64, items: u64) {
        self.size = self.size.saturating_sub(size);
        self.items = self.items.saturating_sub(items);
    }

    pub fn count_where(&self, test: fn(&Node) -> bool) -> u64 {
        let below: u64 = self
            .children
            .iter()
            .map(|child| child.count_where(test))
            .sum();
        u64::from(test(self)) + below
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_totals_sizes_and_sorts_biggest_first() {
        let node = Node::with_children(
            "root",
            vec![
                Node::leaf("small", Kind::File, 10),
                Node::with_children("sub", vec![Node::leaf("big", Kind::File, 500)]),
                Node::leaf("mid", Kind::File, 40),
            ],
        );

        assert_eq!(node.size, 550);
        assert_eq!(node.items, 4);
        let names: Vec<_> = node
            .children
            .iter()
            .map(|c| c.label().into_owned())
            .collect();
        assert_eq!(names, ["sub", "mid", "small"]);
    }

    #[test]
    fn trash_empties_the_entry_and_shrinks_its_folders() {
        let mut node = Node::with_children(
            "root",
            vec![
                Node::with_children(
                    "a",
                    vec![
                        Node::with_children("b", vec![Node::leaf("c", Kind::File, 30)]),
                        Node::leaf("d", Kind::File, 5),
                    ],
                ),
                Node::leaf("e", Kind::File, 1),
            ],
        );

        assert_eq!(node.trash(&[0, 0]), Some(30));

        assert_eq!((node.size, node.items), (6, 3));
        assert_eq!((node.children[0].size, node.children[0].items), (5, 1));
        let b = &node.children[0].children[0];
        assert!(b.is_trashed && b.children.is_empty() && b.size == 0);
        assert_eq!(node.trash(&[0, 0]), None);
        assert_eq!(node.trash(&[]), None);
        assert_eq!(node.trash(&[7]), None);
    }

    #[test]
    fn descend_follows_child_indices() {
        let node = Node::with_children(
            "root",
            vec![Node::with_children(
                "a",
                vec![Node::leaf("b", Kind::File, 1)],
            )],
        );

        assert_eq!(node.descend(&[0, 0]).label(), "b");
        assert_eq!(node.descend(&[]).label(), "root");
    }
}
