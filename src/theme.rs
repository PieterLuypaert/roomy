use std::ffi::OsStr;
use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType};

pub(crate) const ACCENT: Color = Color::Indexed(75);
pub(crate) const DIM: Color = Color::Indexed(244);
pub(crate) const WARN: Color = Color::Indexed(203);
pub(crate) const DONE: Color = Color::Indexed(114);
pub(crate) const TRACK: Color = Color::Indexed(240);
pub(crate) const BORDER: Color = Color::Indexed(238);
pub(crate) const SELECTION: Style = Style::new()
    .bg(Color::Indexed(236))
    .add_modifier(Modifier::BOLD);
pub(crate) const BADGE: Style = Style::new()
    .fg(Color::Black)
    .bg(ACCENT)
    .add_modifier(Modifier::BOLD);
const KEYCAP: Style = Style::new()
    .fg(Color::Indexed(252))
    .bg(Color::Indexed(238))
    .add_modifier(Modifier::BOLD);
const BIG: Color = Color::Indexed(255);
const IMAGE: Color = Color::Indexed(176);
const VIDEO: Color = Color::Indexed(141);
const AUDIO: Color = Color::Indexed(80);
const ARCHIVE: Color = Color::Indexed(209);
const DOCUMENT: Color = Color::Indexed(187);
const CODE: Color = Color::Indexed(150);

pub(crate) fn panel<'a>(title: &str) -> Block<'a> {
    let title = Line::from(vec![
        Span::styled("─", Style::new().fg(BORDER)),
        Span::styled(
            format!(" {title} "),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
    ]);
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(BORDER))
        .title(title)
}

/// A quiet title for the right end of a panel's border.
pub(crate) fn aside(text: &str) -> Line<'static> {
    if text.is_empty() {
        return Line::default();
    }
    Line::from(vec![
        Span::styled(format!(" {text} "), Style::new().fg(DIM)),
        Span::styled("─", Style::new().fg(BORDER)),
    ])
    .right_aligned()
}

pub(crate) fn keycap(key: &str) -> Span<'static> {
    Span::styled(format!(" {key} "), KEYCAP)
}

pub(crate) fn marker() -> Line<'static> {
    Line::styled("▌", Style::new().fg(ACCENT))
}

/// Brighter the more of its folder an entry takes.
pub(crate) fn heat(share: f64) -> Color {
    match share {
        s if s >= 0.5 => Color::Indexed(203),
        s if s >= 0.2 => Color::Indexed(209),
        s if s >= 0.05 => Color::Indexed(221),
        _ => Color::Indexed(110),
    }
}

/// Gigabytes stand out and kilobytes step back, whatever folder they sit in.
pub(crate) fn size(bytes: u64) -> Style {
    match bytes {
        b if b >= 1_000_000_000 => Style::new().fg(BIG).add_modifier(Modifier::BOLD),
        b if b >= 1_000_000 => Style::new(),
        _ => Style::new().fg(DIM),
    }
}

pub(crate) fn file(name: &OsStr) -> Style {
    let extension = Path::new(name)
        .extension()
        .map(|extension| extension.to_string_lossy().to_lowercase());
    let color = match extension.as_deref() {
        Some(
            "png" | "jpg" | "jpeg" | "gif" | "heic" | "webp" | "svg" | "tif" | "tiff" | "psd"
            | "raw" | "dng" | "cr2" | "nef",
        ) => IMAGE,
        Some("mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" | "wmv") => VIDEO,
        Some("mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "aiff" | "aif") => AUDIO,
        Some(
            "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "zst" | "7z" | "rar" | "dmg" | "iso"
            | "pkg" | "img" | "sparseimage" | "sparsebundle" | "ipa" | "apk",
        ) => ARCHIVE,
        Some(
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "key" | "pages" | "numbers"
            | "txt" | "md" | "rtf" | "csv" | "epub",
        ) => DOCUMENT,
        Some(
            "rs" | "js" | "mjs" | "ts" | "tsx" | "jsx" | "py" | "go" | "rb" | "java" | "c" | "h"
            | "cpp" | "swift" | "kt" | "json" | "yaml" | "yml" | "toml" | "html" | "css" | "sh"
            | "sql" | "lock",
        ) => CODE,
        _ => return Style::new(),
    };
    Style::new().fg(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_get_the_color_of_their_kind() {
        assert_eq!(file(OsStr::new("Movie.MOV")).fg, Some(VIDEO));
        assert_eq!(file(OsStr::new("backup.tar.gz")).fg, Some(ARCHIVE));
        assert_eq!(file(OsStr::new("Makefile")).fg, None);
        assert_eq!(file(OsStr::new(".zshrc")).fg, None);
    }

    #[test]
    fn sizes_dim_below_a_megabyte_and_stand_out_from_a_gigabyte() {
        assert_eq!(size(999_999).fg, Some(DIM));
        assert_eq!(size(5_000_000).fg, None);
        assert_eq!(size(1_000_000_000).fg, Some(BIG));
    }
}
