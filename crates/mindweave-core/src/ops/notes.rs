//! Rich text from simple Markdown-like source, for topic notes and text.
//!
//! Each source line is a paragraph. `- `, `* ` or `+ ` starts a bullet,
//! `1. ` a numbered item; two leading spaces per level indent it (up to
//! level 2). Inline, `**bold**`, `*italic*` and `_italic_` are styled;
//! everything else is kept as written.

use crate::text::{HAlign, ListItem, ListKind, Paragraph, RichText, Run, RunStyle, MAX_LIST_LEVEL};

/// `src` as rich text in `base` style, aligned left.
pub fn rich_text_from_markdown(src: &str, base: &RunStyle) -> RichText {
    let paragraphs = src
        .replace("\r\n", "\n")
        .split('\n')
        .map(|line| paragraph(line, base))
        .collect();
    let mut text = RichText { paragraphs };
    text.normalize();
    text
}

/// Plain `src` (no Markdown) as rich text in `base` style, `align`ed:
/// each line a paragraph.
pub fn rich_text_plain(src: &str, base: &RunStyle, align: HAlign) -> RichText {
    let mut text = RichText::new(base.clone(), align);
    text.insert(0, &src.replace("\r\n", "\n"), base);
    text
}

/// One source line as a paragraph.
fn paragraph(line: &str, base: &RunStyle) -> Paragraph {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = line.trim_start_matches(' ');
    let level = u8::try_from(indent / 2)
        .unwrap_or(MAX_LIST_LEVEL)
        .min(MAX_LIST_LEVEL);
    let (list, body) = list_prefix(rest)
        .map(|(kind, body)| (Some(ListItem { kind, level }), body))
        .unwrap_or((None, line));
    Paragraph {
        runs: inline_runs(body, base),
        align: HAlign::Left,
        style: base.clone(),
        list,
    }
}

/// The list kind `line` starts with and the text after its marker.
fn list_prefix(line: &str) -> Option<(ListKind, &str)> {
    for bullet in ["- ", "* ", "+ "] {
        if let Some(body) = line.strip_prefix(bullet) {
            return Some((ListKind::Bullet, body));
        }
    }
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits > 0 {
        if let Some(body) = line[digits..].strip_prefix(". ") {
            return Some((ListKind::Numbered, body));
        }
    }
    None
}

/// `text` split into runs at `**`, `*` and `_` toggles.
fn inline_runs(text: &str, base: &RunStyle) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut style = base.clone();
    let mut cur = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let toggle = if rest.starts_with("**") && (style.bold || rest[2..].contains("**")) {
            Some((2, true))
        } else if (c == '*' || c == '_') && (style.italic || rest[1..].contains(c)) {
            Some((1, false))
        } else {
            None
        };
        match toggle {
            Some((len, bold)) => {
                if !cur.is_empty() {
                    runs.push(Run {
                        text: std::mem::take(&mut cur),
                        style: style.clone(),
                    });
                }
                if bold {
                    style.bold = !style.bold;
                } else {
                    style.italic = !style.italic;
                }
                rest = &rest[len..];
            }
            None => {
                cur.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    if !cur.is_empty() {
        runs.push(Run { text: cur, style });
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_become_paragraphs_with_lists_and_levels() {
        let t = rich_text_from_markdown("Intro\n- one\n  - nested\n1. first", &RunStyle::default());
        assert_eq!(t.paragraphs.len(), 4);
        assert_eq!(t.paragraphs[0].list, None);
        let item = |i: usize| t.paragraphs[i].list.unwrap();
        assert_eq!((item(1).kind, item(1).level), (ListKind::Bullet, 0));
        assert_eq!((item(2).kind, item(2).level), (ListKind::Bullet, 1));
        assert_eq!(item(3).kind, ListKind::Numbered);
        assert_eq!(t.plain(), "Intro\none\nnested\nfirst");
    }

    #[test]
    fn bold_and_italic_spans_are_styled() {
        let t = rich_text_from_markdown("a **b** *c* _d_ 2*3", &RunStyle::default());
        assert_eq!(t.plain(), "a b c d 2*3");
        let runs = &t.paragraphs[0].runs;
        let find = |s: &str| runs.iter().find(|r| r.text == s).unwrap().style.clone();
        assert!(find("b").bold);
        assert!(find("c").italic);
        assert!(find("d").italic);
    }

    #[test]
    fn plain_text_keeps_markers() {
        let t = rich_text_plain("**x**\ny", &RunStyle::default(), HAlign::Center);
        assert_eq!(t.plain(), "**x**\ny");
        assert_eq!(t.paragraphs[1].align, HAlign::Center);
    }
}
