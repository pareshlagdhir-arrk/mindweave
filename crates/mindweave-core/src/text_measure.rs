//! Approximate text layout without a font engine.
//!
//! The desktop app measures text with real fonts; tools that work on
//! files without it (the command layer and picture export) use this
//! estimate instead: each character gets a typical advance for a sans
//! serif face (a fraction of the font size), lines are
//! [`LINE_HEIGHT`] times their largest font size, and text wraps at
//! spaces (a word longer than the wrap width breaks between characters).
//! The app refits the real sizes when it opens the file.

use crate::text::{HAlign, ListKind, RichText, RunStyle, LINE_BREAK};

/// Line height as a multiple of the largest font size on the line (as
/// the app lays text out).
pub const LINE_HEIGHT: f32 = 1.25;
/// How far a list item is indented per level (level 0 is one step in).
pub const LIST_INDENT: f32 = 18.0;
/// Bullet glyphs by list level.
const BULLETS: [&str; 3] = ["•", "◦", "▪"];
/// Bold text is about this much wider.
const BOLD_FACTOR: f32 = 1.07;

/// A piece of a laid-out line in one style.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub text: String,
    pub style: RunStyle,
}

/// One laid-out line.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    pub segments: Vec<Segment>,
    /// Width of the text, without the indent.
    pub width: f32,
    /// The line's height ([`LINE_HEIGHT`] × its largest font size).
    pub height: f32,
    /// The largest font size on the line.
    pub size: f32,
    pub align: HAlign,
    /// Space before the text (list items).
    pub indent: f32,
    /// A list marker drawn in the indent, on a paragraph's first line.
    pub marker: Option<String>,
}

/// Typical advance of `c` in em for a sans-serif face.
pub fn char_advance(c: char) -> f32 {
    match c {
        'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '\'' | '!' | '|' | 'I' => 0.28,
        ' ' => 0.28,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '-' | '"' => 0.36,
        'm' | 'w' => 0.82,
        'M' | 'W' | '@' | '%' => 0.88,
        'A'..='Z' => 0.66,
        '0'..='9' => 0.56,
        c if (c as u32) >= 0x2E80 => 1.0,
        _ => 0.53,
    }
}

/// Estimated width of `text` in `style`.
pub fn text_width(text: &str, style: &RunStyle) -> f32 {
    let bold = if style.bold { BOLD_FACTOR } else { 1.0 };
    text.chars().map(char_advance).sum::<f32>() * style.size * bold
}

/// A word (with the spaces after it) in one style.
#[derive(Clone, Debug)]
struct Piece {
    text: String,
    style: RunStyle,
}

/// Splits a run into words, each keeping its trailing spaces.
fn words(text: &str, style: &RunStyle) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let mut cur = String::new();
    let mut in_space = false;
    for c in text.chars() {
        if c == ' ' {
            in_space = true;
        } else if in_space {
            out.push(Piece {
                text: std::mem::take(&mut cur),
                style: style.clone(),
            });
            in_space = false;
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(Piece {
            text: cur,
            style: style.clone(),
        });
    }
    out
}

/// Builds lines from `pieces` wrapped at `wrap` (minus `indent`).
struct LineBuilder {
    lines: Vec<TextLine>,
    cur: Vec<Segment>,
    width: f32,
    size: f32,
    align: HAlign,
    indent: f32,
    marker: Option<String>,
    base_size: f32,
}

impl LineBuilder {
    fn push(&mut self, text: &str, style: &RunStyle) {
        self.width += text_width(text, style);
        self.size = self.size.max(style.size);
        match self.cur.last_mut() {
            Some(last) if last.style == *style => last.text.push_str(text),
            _ => self.cur.push(Segment {
                text: text.to_string(),
                style: style.clone(),
            }),
        }
    }

    fn finish(&mut self) {
        // Trailing spaces take no room at a wrap.
        if let Some(last) = self.cur.last_mut() {
            let trimmed = last.text.trim_end_matches(' ').len();
            let removed = &last.text[trimmed..];
            self.width -= text_width(removed, &last.style);
            last.text.truncate(trimmed);
        }
        let size = if self.size > 0.0 {
            self.size
        } else {
            self.base_size
        };
        self.lines.push(TextLine {
            segments: std::mem::take(&mut self.cur),
            width: self.width.max(0.0),
            height: size * LINE_HEIGHT,
            size,
            align: self.align,
            indent: self.indent,
            marker: self.marker.take(),
        });
        self.width = 0.0;
        self.size = 0.0;
    }
}

/// The marker of paragraph `i` of `text`, if it is a list item.
fn marker(text: &RichText, i: usize) -> Option<String> {
    let item = text.paragraphs.get(i)?.list?;
    match item.kind {
        ListKind::Bullet => Some(BULLETS[usize::from(item.level).min(2)].to_string()),
        ListKind::Numbered => text.list_number(i).map(|n| format!("{n}.")),
    }
}

/// Lays `text` out in lines, wrapping at `wrap` when given.
pub fn layout(text: &RichText, wrap: Option<f32>) -> Vec<TextLine> {
    let mut b = LineBuilder {
        lines: Vec::new(),
        cur: Vec::new(),
        width: 0.0,
        size: 0.0,
        align: HAlign::Left,
        indent: 0.0,
        marker: None,
        base_size: 0.0,
    };
    for (i, p) in text.paragraphs.iter().enumerate() {
        b.align = p.align;
        b.indent = p
            .list
            .map_or(0.0, |l| LIST_INDENT * (f32::from(l.level) + 1.0));
        b.marker = marker(text, i);
        b.base_size = p.style.size;
        let limit = wrap.map(|w| (w - b.indent).max(1.0));
        for run in &p.runs {
            for (k, hard) in run.text.split(LINE_BREAK).enumerate() {
                if k > 0 {
                    b.finish();
                }
                for piece in words(hard, &run.style) {
                    place_piece(&mut b, piece, limit);
                }
            }
        }
        b.finish();
    }
    b.lines
}

/// Adds a word to the line, wrapping first when it does not fit, and
/// breaking a word too long for a line of its own.
fn place_piece(b: &mut LineBuilder, piece: Piece, limit: Option<f32>) {
    let Some(limit) = limit else {
        b.push(&piece.text, &piece.style);
        return;
    };
    let word = piece.text.trim_end_matches(' ');
    let w = text_width(word, &piece.style);
    if b.width > 0.0 && b.width + w > limit {
        b.finish();
    }
    if w <= limit || b.width > 0.0 {
        b.push(&piece.text, &piece.style);
        return;
    }
    // Longer than a whole line: break between characters.
    let mut chunk = String::new();
    for c in piece.text.chars() {
        let next = text_width(&format!("{chunk}{c}"), &piece.style);
        if !chunk.is_empty() && next > limit && c != ' ' {
            b.push(&chunk, &piece.style);
            b.finish();
            chunk.clear();
        }
        chunk.push(c);
    }
    b.push(&chunk, &piece.style);
}

/// The size of `text` laid out (wrapped at `wrap` when given): the
/// widest line with its indent, and the lines' total height.
pub fn measure(text: &RichText, wrap: Option<f32>) -> (f32, f32) {
    let lines = layout(text, wrap);
    let w = lines
        .iter()
        .map(|l| l.width + l.indent)
        .fold(0.0_f32, f32::max);
    let h = lines.iter().map(|l| l.height).sum();
    (w.ceil(), h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{HAlign, RichText, RunStyle};

    fn text(s: &str, size: f32) -> RichText {
        let style = RunStyle {
            size,
            ..RunStyle::default()
        };
        let mut t = RichText::new(style.clone(), HAlign::Left);
        t.insert(0, s, &style);
        t
    }

    #[test]
    fn one_line_is_as_high_as_its_font_and_grows_with_its_text() {
        let (w1, h1) = measure(&text("Hello", 16.0), None);
        let (w2, h2) = measure(&text("Hello world", 16.0), None);
        assert_eq!(h1, 20.0);
        assert_eq!(h1, h2);
        assert!(w2 > w1 && w1 > 20.0);
    }

    #[test]
    fn paragraphs_stack_and_wrapping_adds_lines() {
        let (_, h) = measure(&text("a\nb\nc", 10.0), None);
        assert_eq!(h, 3.0 * 12.5);
        let long = text("one two three four five six seven eight", 16.0);
        let (w, h) = measure(&long, Some(80.0));
        assert!(w <= 80.0, "{w}");
        assert!(h >= 4.0 * 20.0, "{h}");
    }

    #[test]
    fn a_word_longer_than_the_wrap_breaks_between_characters() {
        let lines = layout(&text("abcdefghijklmnopqrstuvwxyz", 16.0), Some(50.0));
        assert!(lines.len() > 3);
        assert!(lines.iter().all(|l| l.width <= 50.0 + 16.0));
    }

    #[test]
    fn empty_text_is_one_line_high() {
        let (w, h) = measure(&text("", 16.0), None);
        assert_eq!((w, h), (0.0, 20.0));
    }
}
