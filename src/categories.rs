use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::tree::Node;
use crate::volumes;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Builds,
    Tools,
    Vms,
    Apps,
    Caches,
    Photos,
    Mail,
    Cloud,
    Backups,
    AppData,
    Documents,
    Downloads,
    Media,
    Trash,
    Home,
    Users,
    System,
}

impl Group {
    pub fn name(self) -> &'static str {
        match self {
            Group::Builds => "Project dependencies & builds",
            Group::Tools => "Developer tools",
            Group::Vms => "Docker & virtual machines",
            Group::Apps => "Apps",
            Group::Caches => "Caches, logs & temporary files",
            Group::Photos => "Photos",
            Group::Mail => "Mail & Messages",
            Group::Cloud => "iCloud & cloud drives",
            Group::Backups => "iPhone & iPad backups",
            Group::AppData => "App data",
            Group::Documents => "Documents & Desktop",
            Group::Downloads => "Downloads",
            Group::Media => "Music, movies & pictures",
            Group::Trash => "Trash",
            Group::Home => "Other files in your home",
            Group::Users => "Other users & Shared",
            Group::System => "System & other",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Group::Builds => "node_modules, build output and virtualenvs that tools can rebuild",
            Group::Tools => "toolchains, package caches, Homebrew, Xcode and editor extensions",
            Group::Vms => "disk images of Docker, OrbStack, UTM and other virtual machines",
            Group::Apps => "applications in /Applications and ~/Applications",
            Group::Caches => "caches, logs and temporary files that get recreated when needed",
            Group::Photos => "Photos libraries",
            Group::Mail => "mailboxes and message attachments",
            Group::Cloud => "files that iCloud Drive and other cloud drives keep on this Mac",
            Group::Backups => "device backups made by Finder",
            Group::AppData => "what apps keep in ~/Library: settings, databases and containers",
            Group::Documents => "your Documents and Desktop folders",
            Group::Downloads => "your Downloads folder",
            Group::Media => "your Music, Movies and Pictures folders",
            Group::Trash => "deleted files that take space until the Trash is emptied",
            Group::Home => "the rest of your home folder, such as hidden tool folders",
            Group::Users => "the home folders of other users and /Users/Shared",
            Group::System => "data outside your home: /Library, /private, /usr and more",
        }
    }

    fn hosts_projects(self) -> bool {
        matches!(
            self,
            Group::Documents
                | Group::Downloads
                | Group::Media
                | Group::Cloud
                | Group::Home
                | Group::Users
                | Group::System
        )
    }
}

enum Pattern {
    Path(&'static str),
    Name(&'static str),
    Marked(&'static str, &'static str),
    Beside(&'static str, &'static str),
    Suffix(&'static str),
}

struct Rule {
    pattern: Pattern,
    group: Group,
    label: &'static str,
    is_split: bool,
}

const fn at(path: &'static str, group: Group, label: &'static str) -> Rule {
    Rule {
        pattern: Pattern::Path(path),
        group,
        label,
        is_split: false,
    }
}

const fn each(path: &'static str, group: Group, label: &'static str) -> Rule {
    Rule {
        pattern: Pattern::Path(path),
        group,
        label,
        is_split: true,
    }
}

const fn named(pattern: Pattern, group: Group, label: &'static str) -> Rule {
    Rule {
        pattern,
        group,
        label,
        is_split: false,
    }
}

const RULES: &[Rule] = &[
    each("/Applications", Group::Apps, "app"),
    each("~/Applications", Group::Apps, "app"),
    named(
        Pattern::Beside("node_modules", "package.json"),
        Group::Builds,
        "node_modules",
    ),
    named(
        Pattern::Name("node_modules"),
        Group::Tools,
        "installed packages",
    ),
    named(
        Pattern::Marked("target", "CACHEDIR.TAG"),
        Group::Builds,
        "Rust build output",
    ),
    named(
        Pattern::Marked("vendor", "autoload.php"),
        Group::Builds,
        "Composer vendor",
    ),
    named(
        Pattern::Marked(".venv", "pyvenv.cfg"),
        Group::Builds,
        "Python virtualenv",
    ),
    named(
        Pattern::Marked("venv", "pyvenv.cfg"),
        Group::Builds,
        "Python virtualenv",
    ),
    named(Pattern::Name(".next"), Group::Builds, "Next.js build"),
    named(Pattern::Name(".nuxt"), Group::Builds, "Nuxt build"),
    named(
        Pattern::Name(".svelte-kit"),
        Group::Builds,
        "SvelteKit build",
    ),
    named(Pattern::Name(".turbo"), Group::Builds, "Turborepo cache"),
    named(
        Pattern::Name(".parcel-cache"),
        Group::Builds,
        "Parcel cache",
    ),
    named(
        Pattern::Name(".gradle"),
        Group::Builds,
        "Gradle build cache",
    ),
    named(
        Pattern::Name(".terraform"),
        Group::Builds,
        "Terraform providers",
    ),
    at(
        "~/Library/Developer/Xcode/DerivedData",
        Group::Builds,
        "Xcode DerivedData",
    ),
    each("~/Library/Developer", Group::Tools, "Xcode"),
    at("/Library/Developer", Group::Tools, "Command Line Tools"),
    at("~/.rustup", Group::Tools, "Rust toolchains"),
    at("~/.cargo", Group::Tools, "Cargo"),
    at("~/.npm", Group::Tools, "npm cache"),
    at("~/.pnpm-store", Group::Tools, "pnpm store"),
    at("~/Library/pnpm", Group::Tools, "pnpm store"),
    at("~/.yarn", Group::Tools, "Yarn cache"),
    at("~/.bun", Group::Tools, "Bun"),
    at("~/.deno", Group::Tools, "Deno"),
    at("~/.nvm", Group::Tools, "Node versions"),
    at("~/.local/share/fnm", Group::Tools, "Node versions"),
    at(
        "~/Library/Application Support/fnm",
        Group::Tools,
        "Node versions",
    ),
    at("~/.volta", Group::Tools, "Node versions"),
    at("~/.pyenv", Group::Tools, "Python versions"),
    at("~/.gradle", Group::Tools, "Gradle"),
    at("~/.m2", Group::Tools, "Maven"),
    at("~/go", Group::Tools, "Go"),
    at("~/.composer", Group::Tools, "Composer"),
    at("~/.vscode", Group::Tools, "VS Code extensions"),
    at("~/.cursor", Group::Tools, "Cursor extensions"),
    at("/opt/homebrew", Group::Tools, "Homebrew"),
    at("/usr/local/Homebrew", Group::Tools, "Homebrew"),
    at("/usr/local/Cellar", Group::Tools, "Homebrew"),
    at("/usr/local/Caskroom", Group::Tools, "Homebrew"),
    at(
        "~/Library/Containers/com.docker.docker",
        Group::Vms,
        "Docker Desktop",
    ),
    at("~/.docker", Group::Vms, "Docker"),
    at("~/.orbstack", Group::Vms, "OrbStack"),
    at(
        "~/Library/Group Containers/HUAQ24HBR6.dev.orbstack",
        Group::Vms,
        "OrbStack",
    ),
    at("~/.colima", Group::Vms, "Colima"),
    at("~/.lima", Group::Vms, "Lima"),
    at("~/Library/Containers/com.utmapp.UTM", Group::Vms, "UTM"),
    at("~/Parallels", Group::Vms, "Parallels"),
    at("/var/lib/docker", Group::Vms, "Docker"),
    each("~/Library/Caches", Group::Caches, "app cache"),
    at("/Library/Caches", Group::Caches, "system caches"),
    each("~/.cache", Group::Caches, "tool cache"),
    at("/private/var/folders", Group::Caches, "temporary files"),
    at("/private/tmp", Group::Caches, "temporary files"),
    at("~/Library/Logs", Group::Caches, "logs"),
    at("/Library/Logs", Group::Caches, "logs"),
    at("/private/var/log", Group::Caches, "logs"),
    at("/var/cache", Group::Caches, "system caches"),
    at("/var/log", Group::Caches, "logs"),
    named(
        Pattern::Suffix(".photoslibrary"),
        Group::Photos,
        "Photos library",
    ),
    at("~/Library/Mail", Group::Mail, "Mail"),
    at("~/Library/Containers/com.apple.mail", Group::Mail, "Mail"),
    at("~/Library/Messages", Group::Mail, "Messages"),
    each("~/Library/Mobile Documents", Group::Cloud, "iCloud Drive"),
    each("~/Library/CloudStorage", Group::Cloud, "cloud drive"),
    at(
        "~/Library/Application Support/MobileSync",
        Group::Backups,
        "device backups",
    ),
    each(
        "~/Library/Application Support",
        Group::AppData,
        "app support",
    ),
    each("~/Library/Containers", Group::AppData, "app container"),
    each(
        "~/Library/Group Containers",
        Group::AppData,
        "shared container",
    ),
    each("~/Library", Group::AppData, "library"),
    each("~/Documents", Group::Documents, "documents"),
    each("~/Desktop", Group::Documents, "desktop"),
    each("~/Downloads", Group::Downloads, "download"),
    each("~/Music", Group::Media, "music"),
    each("~/Movies", Group::Media, "movies"),
    each("~/Pictures", Group::Media, "pictures"),
    at("~/.Trash", Group::Trash, "Trash"),
    at("~/.local/share/Trash", Group::Trash, "Trash"),
    each("~", Group::Home, "home"),
    each("/Users", Group::Users, "user"),
    each("/home", Group::Users, "user"),
    at("/macOS Install Data", Group::System, "macOS update"),
    each("/", Group::System, "system"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub group: Group,
    pub label: &'static str,
    pub path: PathBuf,
    pub trail: Vec<usize>,
    pub size: u64,
    pub denied: u64,
    pub is_blocked: bool,
    pub is_container: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Total {
    pub group: Group,
    pub size: u64,
    pub places: usize,
    pub denied: u64,
    pub is_blocked: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub totals: Vec<Total>,
    pub places: Vec<Place>,
}

impl Summary {
    pub fn place_indices(&self, group: Group) -> Vec<usize> {
        (0..self.places.len())
            .filter(|&index| self.places[index].group == group)
            .collect()
    }

    pub fn size(&self) -> u64 {
        self.totals.iter().map(|total| total.size).sum()
    }
}

pub fn summarize(root: &Node, root_path: &Path, home: &Path) -> Summary {
    let mut walk = Walk {
        rules: Rules::new(home),
        places: Vec::new(),
        path: root_path.to_path_buf(),
        trail: Vec::new(),
    };
    walk.visit(root, &[], None, None);
    finish(walk.places)
}

fn finish(places: Vec<Place>) -> Summary {
    let mut places: Vec<Place> = places
        .into_iter()
        .filter(|place| place.size > 0 || place.denied > 0)
        .collect();
    places.sort_by_key(|place| std::cmp::Reverse(place.size));
    let mut totals: Vec<Total> = Vec::new();
    for place in &places {
        let index = match totals.iter().position(|total| total.group == place.group) {
            Some(index) => index,
            None => {
                totals.push(Total {
                    group: place.group,
                    size: 0,
                    places: 0,
                    denied: 0,
                    is_blocked: true,
                });
                totals.len() - 1
            }
        };
        let total = &mut totals[index];
        total.size += place.size;
        total.places += 1;
        total.denied += place.denied;
        total.is_blocked &= place.is_blocked;
    }
    totals.sort_by_key(|total| std::cmp::Reverse(total.size));
    Summary { totals, places }
}

struct Rules {
    paths: HashMap<PathBuf, &'static Rule>,
    names: HashMap<&'static str, Vec<&'static Rule>>,
    suffixes: Vec<&'static Rule>,
}

impl Rules {
    fn new(home: &Path) -> Self {
        let mut rules = Rules {
            paths: HashMap::new(),
            names: HashMap::new(),
            suffixes: Vec::new(),
        };
        for rule in RULES {
            match rule.pattern {
                Pattern::Path(path) => {
                    rules.paths.insert(expand(path, home), rule);
                }
                Pattern::Name(name) | Pattern::Marked(name, _) | Pattern::Beside(name, _) => {
                    rules.names.entry(name).or_default().push(rule);
                }
                Pattern::Suffix(_) => rules.suffixes.push(rule),
            }
        }
        rules
    }

    fn find(&self, node: &Node, logical: &Path, siblings: &[Node]) -> Option<&'static Rule> {
        if let Some(rule) = self.paths.get(logical) {
            return Some(rule);
        }
        let name = node.name.to_str()?;
        let named = self.names.get(name).and_then(|rules| {
            rules
                .iter()
                .copied()
                .find(|rule| marks(rule, node, siblings))
        });
        named.or_else(|| {
            self.suffixes.iter().copied().find(
                |rule| matches!(rule.pattern, Pattern::Suffix(suffix) if name.ends_with(suffix)),
            )
        })
    }
}

fn marks(rule: &Rule, node: &Node, siblings: &[Node]) -> bool {
    let has = |nodes: &[Node], marker: &str| nodes.iter().any(|n| &*n.name == marker);
    match rule.pattern {
        Pattern::Marked(_, marker) => has(&node.children, marker),
        Pattern::Beside(_, marker) => has(siblings, marker),
        _ => true,
    }
}

fn expand(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix('~') {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(path),
    }
}

struct Walk {
    rules: Rules,
    places: Vec<Place>,
    path: PathBuf,
    trail: Vec<usize>,
}

impl Walk {
    fn visit(
        &mut self,
        node: &Node,
        siblings: &[Node],
        owner: Option<usize>,
        split: Option<&'static Rule>,
    ) {
        let (owner, split) = self.claim(node, siblings, owner, split);
        let Some(owner) = owner else {
            let index = self.start(Group::System, "other", node.is_denied, true);
            return self.visit_contents(node, index, split);
        };
        self.visit_contents(node, owner, split);
    }

    fn visit_contents(&mut self, node: &Node, owner: usize, split: Option<&'static Rule>) {
        let children: u64 = node.children.iter().map(|child| child.size).sum();
        self.places[owner].size += node.size.saturating_sub(children);
        self.places[owner].denied += u64::from(node.is_denied);
        for (index, child) in node.children.iter().enumerate() {
            self.path.push(&*child.name);
            self.trail.push(index);
            self.visit(child, &node.children, Some(owner), split);
            self.trail.pop();
            self.path.pop();
        }
    }

    fn claim(
        &mut self,
        node: &Node,
        siblings: &[Node],
        owner: Option<usize>,
        split: Option<&'static Rule>,
    ) -> (Option<usize>, Option<&'static Rule>) {
        let host = owner.map(|index| self.places[index].group);
        let rule = node
            .is_dir()
            .then(|| {
                self.rules
                    .find(node, volumes::logical(&self.path), siblings)
            })
            .flatten()
            .filter(|rule| {
                matches!(rule.pattern, Pattern::Path(_)) || host.is_none_or(Group::hosts_projects)
            });
        if let Some(rule) = rule {
            let index = self.start(rule.group, rule.label, node.is_denied, rule.is_split);
            return (Some(index), rule.is_split.then_some(rule));
        }
        match split {
            Some(rule) => (
                Some(self.start(rule.group, rule.label, node.is_denied, false)),
                None,
            ),
            None => (owner, None),
        }
    }

    fn start(
        &mut self,
        group: Group,
        label: &'static str,
        is_blocked: bool,
        is_container: bool,
    ) -> usize {
        self.places.push(Place {
            group,
            label,
            path: self.path.clone(),
            trail: self.trail.clone(),
            size: 0,
            denied: 0,
            is_blocked,
            is_container,
        });
        self.places.len() - 1
    }
}
