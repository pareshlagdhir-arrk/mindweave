//! Rich text: paragraphs of styled runs, and the editing operations on it.
//!
//! Positions are byte offsets into [`RichText::plain`], where paragraphs are
//! joined with `'\n'` and a line break inside a paragraph is `'\u{2028}'`.
//! Every public operation snaps positions to grapheme cluster boundaries, so
//! emoji sequences and combining marks are never split.

use std::ops::Range;

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::color::normalize_hex_color;

pub const MIN_FONT_SIZE: f32 = 1.0;
pub const MAX_FONT_SIZE: f32 = 1000.0;
pub const DEFAULT_FONT_SIZE: f32 = 16.0;
pub const LABEL_FONT_SIZE: f32 = 14.0;

/// A line break inside a paragraph (Shift+Return).
pub const LINE_BREAK: char = '\u{2028}';

/// How a run of text looks. `font: None` is the app default font;
/// `color: None` is Auto contrast; colours are uppercase `#RRGGBB`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunStyle {
    pub font: Option<String>,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: Option<String>,
}

impl Default for RunStyle {
    fn default() -> Self {
        RunStyle {
            font: None,
            size: DEFAULT_FONT_SIZE,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: None,
        }
    }
}

impl RunStyle {
    /// Clamps the size (non-finite becomes the default), drops invalid
    /// colours and empty font names.
    fn sanitize(&mut self) {
        self.size = if self.size.is_finite() {
            self.size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
        } else {
            DEFAULT_FONT_SIZE
        };
        self.color = self.color.as_deref().and_then(normalize_hex_color);
        if self.font.as_deref().is_some_and(|f| f.trim().is_empty()) {
            self.font = None;
        }
    }
}

/// Text in one style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    pub style: RunStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// A paragraph: styled runs plus alignment. `style` is what typing uses
/// while the paragraph is empty; [`RichText::normalize`] keeps it equal to
/// the first run's style otherwise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    #[serde(default)]
    pub align: HAlign,
    #[serde(default)]
    pub style: RunStyle,
}

impl Paragraph {
    fn empty(style: RunStyle, align: HAlign) -> Self {
        Paragraph {
            runs: Vec::new(),
            align,
            style,
        }
    }

    fn len(&self) -> usize {
        self.runs.iter().map(|r| r.text.len()).sum()
    }
}

/// Rich text. Invariant: at least one paragraph (deserialisation and
/// [`RichText::normalize`] restore it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "RichTextRepr")]
pub struct RichText {
    pub paragraphs: Vec<Paragraph>,
}

#[derive(Deserialize)]
struct RichTextRepr {
    paragraphs: Vec<Paragraph>,
}

impl From<RichTextRepr> for RichText {
    fn from(repr: RichTextRepr) -> Self {
        let mut text = RichText {
            paragraphs: repr.paragraphs,
        };
        text.normalize();
        text
    }
}

/// One property set on a range of text.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleChange {
    Bold(bool),
    Italic(bool),
    Underline(bool),
    Strike(bool),
    Font(Option<String>),
    Size(f32),
    Color(Option<String>),
}

impl StyleChange {
    fn apply_to(&self, style: &mut RunStyle) {
        match self {
            StyleChange::Bold(v) => style.bold = *v,
            StyleChange::Italic(v) => style.italic = *v,
            StyleChange::Underline(v) => style.underline = *v,
            StyleChange::Strike(v) => style.strike = *v,
            StyleChange::Font(v) => style.font = v.clone(),
            StyleChange::Size(v) => style.size = *v,
            StyleChange::Color(v) => style.color = v.clone(),
        }
        style.sanitize();
    }
}

/// One value shared by a whole range, or several.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mixed<T> {
    Same(T),
    Mixed,
}

impl<T: PartialEq> Mixed<T> {
    fn of(mut values: impl Iterator<Item = T>) -> Self {
        let Some(first) = values.next() else {
            return Mixed::Mixed;
        };
        if values.all(|v| v == first) {
            Mixed::Same(first)
        } else {
            Mixed::Mixed
        }
    }
}

/// The styles of a range, for the sidebar and toggle commands.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleSummary {
    pub font: Mixed<Option<String>>,
    pub size: Mixed<f32>,
    pub bold: Mixed<bool>,
    pub italic: Mixed<bool>,
    pub underline: Mixed<bool>,
    pub strike: Mixed<bool>,
    pub color: Mixed<Option<String>>,
    pub align: Mixed<HAlign>,
}

impl StyleSummary {
    /// The value a toggle (Cmd+B etc.) sets: on unless the whole range is on.
    pub fn toggle_value(m: &Mixed<bool>) -> bool {
        !matches!(m, Mixed::Same(true))
    }
}

/// Splits `runs` at byte offset `at` (a char boundary within them).
fn split_runs(runs: Vec<Run>, at: usize) -> (Vec<Run>, Vec<Run>) {
    let mut head = Vec::new();
    let mut tail = Vec::new();
    let mut start = 0;
    for mut run in runs {
        let end = start + run.text.len();
        if end <= at {
            head.push(run);
        } else if start >= at {
            tail.push(run);
        } else {
            let rest = run.text.split_off(at - start);
            tail.push(Run {
                text: rest,
                style: run.style.clone(),
            });
            head.push(run);
        }
        start = end;
    }
    (head, tail)
}

/// Splits `runs` into the parts before, inside and after `range`.
fn split_runs3(runs: Vec<Run>, range: Range<usize>) -> (Vec<Run>, Vec<Run>, Vec<Run>) {
    let (head, rest) = split_runs(runs, range.start);
    let (mid, tail) = split_runs(rest, range.end - range.start);
    (head, mid, tail)
}

fn is_word(segment: &str) -> bool {
    segment.chars().any(char::is_alphanumeric)
}

impl RichText {
    /// Empty text that types in `style`.
    pub fn new(style: RunStyle, align: HAlign) -> Self {
        let mut text = RichText {
            paragraphs: vec![Paragraph::empty(style, align)],
        };
        text.normalize();
        text
    }

    /// The text without styles; positions index into this.
    pub fn plain(&self) -> String {
        let mut out = String::with_capacity(self.len());
        for (i, p) in self.paragraphs.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            for run in &p.runs {
                out.push_str(&run.text);
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        let text: usize = self.paragraphs.iter().map(Paragraph::len).sum();
        text + self.paragraphs.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Inserts `text` in `style` at `at`; returns the caret after it (on a
    /// grapheme boundary, even if the text joins a following combining mark).
    /// `'\n'` (and `"\r\n"`, `'\r'`) starts a new paragraph with the same
    /// alignment.
    pub fn insert(&mut self, at: usize, text: &str, style: &RunStyle) -> usize {
        let at = self.floor_boundary(at);
        if text.is_empty() {
            return at;
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let before = self.len();
        let (pi, off) = self.locate(at);
        let para = &mut self.paragraphs[pi];
        let align = para.align;
        let (head, tail) = split_runs(std::mem::take(&mut para.runs), off);
        para.runs = head;
        let mut lines = text.split('\n');
        let run = |s: &str| Run {
            text: s.to_string(),
            style: style.clone(),
        };
        para.runs.push(run(lines.next().unwrap_or_default()));
        let mut last = pi;
        for line in lines {
            last += 1;
            let mut p = Paragraph::empty(style.clone(), align);
            p.runs.push(run(line));
            self.paragraphs.insert(last, p);
        }
        self.paragraphs[last].runs.extend(tail);
        self.normalize();
        self.ceil_boundary(at + self.len() - before)
    }

    /// Removes `range`, joining the paragraphs at its ends.
    pub fn delete(&mut self, range: Range<usize>) {
        let range = self.snap_range(range);
        if range.is_empty() {
            return;
        }
        let (ps, os) = self.locate(range.start);
        let (pe, oe) = self.locate(range.end);
        // `ps` and `pe` may be the same paragraph, so the tail is cloned.
        let (_, tail) = split_runs(self.paragraphs[pe].runs.clone(), oe);
        let (head, _) = split_runs(std::mem::take(&mut self.paragraphs[ps].runs), os);
        self.paragraphs[ps].runs = head;
        self.paragraphs[ps].runs.extend(tail);
        self.paragraphs.drain(ps + 1..=pe);
        self.normalize();
    }

    /// A copy of `range` with its styles and paragraph alignment.
    pub fn slice(&self, range: Range<usize>) -> RichText {
        let range = self.snap_range(range);
        let mut paragraphs = Vec::new();
        for (start, p) in self.spans() {
            let end = start + p.len();
            if start > range.end || end < range.start {
                continue;
            }
            let local = range.start.max(start) - start..range.end.min(end) - start;
            let (_, runs, _) = split_runs3(p.runs.clone(), local);
            paragraphs.push(Paragraph {
                runs,
                align: p.align,
                style: p.style.clone(),
            });
        }
        let mut out = RichText { paragraphs };
        out.normalize();
        out
    }

    /// Inserts `other` (normalised) at `at`; returns the caret after it, on a
    /// grapheme boundary. The paragraph at `at` keeps its alignment;
    /// `other`'s later paragraphs keep theirs.
    pub fn insert_rich(&mut self, at: usize, other: &RichText) -> usize {
        let at = self.floor_boundary(at);
        let before = self.len();
        let (pi, off) = self.locate(at);
        let (head, tail) = split_runs(std::mem::take(&mut self.paragraphs[pi].runs), off);
        let mut incoming = other.paragraphs.iter();
        let first = incoming.next().map(|p| p.runs.clone()).unwrap_or_default();
        self.paragraphs[pi].runs = head;
        self.paragraphs[pi].runs.extend(first);
        let mut last = pi;
        for p in incoming {
            last += 1;
            self.paragraphs.insert(last, p.clone());
        }
        self.paragraphs[last].runs.extend(tail);
        self.normalize();
        self.ceil_boundary(at + self.len() - before)
    }

    /// Sets one property on `range`. A caret in an empty paragraph, or a
    /// range across empty paragraphs, changes their typing style too.
    pub fn apply(&mut self, range: Range<usize>, change: &StyleChange) {
        let range = self.snap_range(range);
        let touched = self.touched_paragraphs(&range);
        let starts: Vec<usize> = self.spans().map(|(start, _)| start).collect();
        for pi in touched {
            let p = &mut self.paragraphs[pi];
            if p.runs.is_empty() {
                change.apply_to(&mut p.style);
                continue;
            }
            let start = starts[pi];
            let end = start + p.len();
            let local = range.start.max(start) - start..range.end.min(end) - start;
            let (head, mut mid, tail) = split_runs3(std::mem::take(&mut p.runs), local);
            for run in &mut mid {
                change.apply_to(&mut run.style);
            }
            p.runs = head;
            p.runs.extend(mid);
            p.runs.extend(tail);
        }
        self.normalize();
    }

    /// Sets one property on every run, keeping the others.
    pub fn apply_all(&mut self, change: &StyleChange) {
        for p in &mut self.paragraphs {
            change.apply_to(&mut p.style);
            for run in &mut p.runs {
                change.apply_to(&mut run.style);
            }
        }
        self.normalize();
    }

    /// Aligns every paragraph `range` touches (a caret: its paragraph).
    pub fn set_align(&mut self, range: Range<usize>, align: HAlign) {
        let range = self.snap_range(range);
        for pi in self.touched_paragraphs(&range) {
            self.paragraphs[pi].align = align;
        }
    }

    /// The style typing at `at` uses: the character before the caret's,
    /// else (start of a paragraph) the paragraph's.
    pub fn style_at(&self, at: usize) -> RunStyle {
        let (pi, off) = self.locate(self.floor_boundary(at));
        let p = &self.paragraphs[pi];
        let mut end = 0;
        for run in &p.runs {
            end += run.text.len();
            if off > 0 && off <= end {
                return run.style.clone();
            }
        }
        p.style.clone()
    }

    /// The styles and alignment over `range`; a caret reports
    /// [`RichText::style_at`] and its paragraph's alignment.
    pub fn summary(&self, range: Range<usize>) -> StyleSummary {
        let range = self.snap_range(range);
        let touched = self.touched_paragraphs(&range);
        let mut styles: Vec<RunStyle> = Vec::new();
        if !range.is_empty() {
            for (pi, (start, p)) in self.spans().enumerate() {
                if p.runs.is_empty() && touched.contains(&pi) {
                    styles.push(p.style.clone());
                }
                let mut run_start = start;
                for run in &p.runs {
                    let run_end = run_start + run.text.len();
                    if run_start < range.end && run_end > range.start {
                        styles.push(run.style.clone());
                    }
                    run_start = run_end;
                }
            }
        }
        if styles.is_empty() {
            styles.push(self.style_at(range.start));
        }
        StyleSummary {
            font: Mixed::of(styles.iter().map(|s| s.font.clone())),
            size: Mixed::of(styles.iter().map(|s| s.size)),
            bold: Mixed::of(styles.iter().map(|s| s.bold)),
            italic: Mixed::of(styles.iter().map(|s| s.italic)),
            underline: Mixed::of(styles.iter().map(|s| s.underline)),
            strike: Mixed::of(styles.iter().map(|s| s.strike)),
            color: Mixed::of(styles.iter().map(|s| s.color.clone())),
            align: Mixed::of(touched.iter().map(|&pi| self.paragraphs[pi].align)),
        }
    }

    /// Restores the invariants: at least one paragraph, no empty runs,
    /// adjacent identical runs merged, sizes clamped to
    /// `MIN_FONT_SIZE..=MAX_FONT_SIZE` (non-finite → default), invalid
    /// colours dropped, stray `'\r'`/`'\n'` inside runs turned into line
    /// breaks, and each non-empty paragraph's typing style equal to its
    /// first run's.
    pub fn normalize(&mut self) {
        if self.paragraphs.is_empty() {
            self.paragraphs
                .push(Paragraph::empty(RunStyle::default(), HAlign::Left));
        }
        for p in &mut self.paragraphs {
            p.style.sanitize();
            let mut runs: Vec<Run> = Vec::with_capacity(p.runs.len());
            for mut run in std::mem::take(&mut p.runs) {
                if run.text.is_empty() {
                    continue;
                }
                if run.text.contains(['\r', '\n']) {
                    run.text = run
                        .text
                        .replace("\r\n", "\n")
                        .replace(['\r', '\n'], LINE_BREAK.encode_utf8(&mut [0; 4]));
                }
                run.style.sanitize();
                match runs.last_mut() {
                    Some(prev) if prev.style == run.style => prev.text.push_str(&run.text),
                    _ => runs.push(run),
                }
            }
            if let Some(first) = runs.first() {
                p.style = first.style.clone();
            }
            p.runs = runs;
        }
    }

    /// The grapheme boundary before `at` (0 at the start).
    pub fn prev_grapheme(&self, at: usize) -> usize {
        let b = self.boundaries();
        b.iter().rev().copied().find(|&x| x < at).unwrap_or(0)
    }

    /// The grapheme boundary after `at` (the end at the end).
    pub fn next_grapheme(&self, at: usize) -> usize {
        let b = self.boundaries();
        b.iter().copied().find(|&x| x > at).unwrap_or(self.len())
    }

    /// The start of the word before `at` (Alt+Left); punctuation, spaces
    /// and emoji are skipped.
    pub fn prev_word(&self, at: usize) -> usize {
        let plain = self.plain();
        plain
            .split_word_bound_indices()
            .rev()
            .find(|(start, s)| *start < at && is_word(s))
            .map_or(0, |(start, _)| start)
    }

    /// The end of the word after `at` (Alt+Right); punctuation, spaces and
    /// emoji are skipped.
    pub fn next_word(&self, at: usize) -> usize {
        let plain = self.plain();
        plain
            .split_word_bound_indices()
            .map(|(start, s)| (start + s.len(), s))
            .find(|(end, s)| *end > at && is_word(s))
            .map_or(plain.len(), |(end, _)| end)
    }

    /// The word (or run of spaces, punctuation or emoji) at `at`, for
    /// double-click. Just after a word, that word; never a paragraph break.
    pub fn word_range(&self, at: usize) -> Range<usize> {
        let plain = self.plain();
        let at = at.min(plain.len());
        let mut before = None;
        let mut under = None;
        for (start, s) in plain.split_word_bound_indices() {
            let range = start..start + s.len();
            if range.end == at {
                before = Some((range.clone(), s));
            }
            if range.contains(&at) {
                under = Some((range, s));
                break;
            }
        }
        let pick = match (under, before) {
            (Some((_, u)), Some((r, b))) if !is_word(u) && is_word(b) => Some((r, b)),
            (None, before) => before,
            (under, _) => under,
        };
        match pick {
            Some((r, s)) if s != "\n" => r,
            _ => at..at,
        }
    }

    /// The paragraph containing `at`, without its `'\n'`.
    pub fn paragraph_range(&self, at: usize) -> Range<usize> {
        let (pi, _) = self.locate(at);
        let (start, p) = self.spans().nth(pi).expect("located paragraph");
        start..start + p.len()
    }

    /// Each paragraph with its start offset in [`RichText::plain`].
    fn spans(&self) -> impl Iterator<Item = (usize, &Paragraph)> {
        self.paragraphs.iter().scan(0, |start, p| {
            let here = *start;
            *start += p.len() + 1;
            Some((here, p))
        })
    }

    /// The paragraph index and offset within it for `at` (clamped).
    fn locate(&self, at: usize) -> (usize, usize) {
        let mut last = (0, 0);
        for (pi, (start, p)) in self.spans().enumerate() {
            let end = start + p.len();
            if at <= end {
                return (pi, at.saturating_sub(start));
            }
            last = (pi, p.len());
        }
        last
    }

    /// Indices of the paragraphs `range` touches: a caret touches its own;
    /// a selection touches those it overlaps, including empty ones inside it
    /// or at either end, or (when it holds only a paragraph break between
    /// non-empty paragraphs) the paragraph it starts in.
    fn touched_paragraphs(&self, range: &Range<usize>) -> Vec<usize> {
        let touched: Vec<usize> = self
            .spans()
            .enumerate()
            .filter(|(_, (start, p))| {
                let end = start + p.len();
                if *start == end {
                    (range.start..=range.end).contains(start)
                } else {
                    *start < range.end && end > range.start
                }
            })
            .map(|(pi, _)| pi)
            .collect();
        if touched.is_empty() {
            vec![self.locate(range.start).0]
        } else {
            touched
        }
    }

    /// Grapheme cluster boundaries of the plain text, including 0 and the end.
    fn boundaries(&self) -> Vec<usize> {
        let plain = self.plain();
        let mut b: Vec<usize> = plain.grapheme_indices(true).map(|(i, _)| i).collect();
        b.push(plain.len());
        b
    }

    /// `at` clamped, moved back to a grapheme boundary.
    fn floor_boundary(&self, at: usize) -> usize {
        let b = self.boundaries();
        b.iter().rev().copied().find(|&x| x <= at).unwrap_or(0)
    }

    /// `at` clamped, moved forward to a grapheme boundary.
    fn ceil_boundary(&self, at: usize) -> usize {
        let b = self.boundaries();
        b.iter().copied().find(|&x| x >= at).unwrap_or(self.len())
    }

    /// `range` ordered, clamped and widened to grapheme boundaries.
    fn snap_range(&self, range: Range<usize>) -> Range<usize> {
        let (lo, hi) = (range.start.min(range.end), range.start.max(range.end));
        self.floor_boundary(lo)..self.ceil_boundary(hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_style() -> RunStyle {
        RunStyle::default()
    }

    fn bold_style() -> RunStyle {
        RunStyle {
            bold: true,
            ..RunStyle::default()
        }
    }

    fn text(s: &str) -> RichText {
        let mut t = RichText::new(plain_style(), HAlign::Left);
        t.insert(0, s, &plain_style());
        t
    }

    fn run_texts(t: &RichText, paragraph: usize) -> Vec<&str> {
        t.paragraphs[paragraph]
            .runs
            .iter()
            .map(|r| r.text.as_str())
            .collect()
    }

    #[test]
    fn insert_and_delete_roundtrip() {
        let mut t = RichText::new(plain_style(), HAlign::Left);
        assert!(t.is_empty());
        assert_eq!(t.insert(0, "Hello", &plain_style()), 5);
        assert_eq!(t.insert(5, " world", &plain_style()), 11);
        assert_eq!(t.insert(0, ">", &plain_style()), 1);
        assert_eq!(t.plain(), ">Hello world");
        assert_eq!(t.len(), 12);
        t.delete(0..1);
        t.delete(5..11);
        assert_eq!(t.plain(), "Hello");
        t.delete(0..5);
        assert!(t.is_empty());
        assert_eq!(t.paragraphs.len(), 1);
        assert!(t.paragraphs[0].runs.is_empty());
    }

    #[test]
    fn newline_splits_paragraph_and_inherits_align() {
        let mut t = RichText::new(plain_style(), HAlign::Center);
        t.insert(0, "abcdef", &plain_style());
        assert_eq!(t.insert(3, "\n", &plain_style()), 4);
        assert_eq!(t.paragraphs.len(), 2);
        assert_eq!(run_texts(&t, 0), ["abc"]);
        assert_eq!(run_texts(&t, 1), ["def"]);
        assert_eq!(t.paragraphs[1].align, HAlign::Center);
        assert_eq!(t.plain(), "abc\ndef");
        assert_eq!(t.paragraph_range(5), 4..7);
        assert_eq!(t.paragraph_range(2), 0..3);
    }

    #[test]
    fn soft_break_stays_in_paragraph() {
        let mut t = text("ab");
        assert_eq!(
            t.insert(1, "\u{2028}", &plain_style()),
            1 + '\u{2028}'.len_utf8()
        );
        assert_eq!(t.paragraphs.len(), 1);
        assert_eq!(t.plain(), "a\u{2028}b");
        assert_eq!(t.paragraph_range(0), 0..t.len());
    }

    #[test]
    fn delete_across_paragraphs_merges() {
        let mut t = RichText::new(plain_style(), HAlign::Right);
        t.insert(0, "one\ntwo\nthree", &plain_style());
        assert_eq!(t.paragraphs.len(), 3);
        t.set_align(8..8, HAlign::Center);
        t.delete(2..9); // "e\ntwo\nt"
        assert_eq!(t.plain(), "onhree");
        assert_eq!(t.paragraphs.len(), 1);
        assert_eq!(t.paragraphs[0].align, HAlign::Right);
        // Backspace at the start of a paragraph joins it with the previous one.
        let mut t = text("ab\ncd");
        t.delete(t.prev_grapheme(3)..3);
        assert_eq!(t.plain(), "abcd");
        assert_eq!(t.paragraphs.len(), 1);
    }

    #[test]
    fn apply_bold_to_middle_splits_runs() {
        let mut t = text("Hello");
        t.apply(1..4, &StyleChange::Bold(true));
        assert_eq!(run_texts(&t, 0), ["H", "ell", "o"]);
        let bold: Vec<bool> = t.paragraphs[0].runs.iter().map(|r| r.style.bold).collect();
        assert_eq!(bold, [false, true, false]);
    }

    #[test]
    fn normalize_merges_identical_runs() {
        let mut t = text("Hello");
        t.apply(1..4, &StyleChange::Bold(true));
        t.apply(1..4, &StyleChange::Bold(false));
        assert_eq!(run_texts(&t, 0), ["Hello"]);

        let mut t = RichText {
            paragraphs: vec![Paragraph {
                runs: vec![
                    Run {
                        text: "a".into(),
                        style: plain_style(),
                    },
                    Run {
                        text: String::new(),
                        style: bold_style(),
                    },
                    Run {
                        text: "b".into(),
                        style: plain_style(),
                    },
                ],
                align: HAlign::Left,
                style: plain_style(),
            }],
        };
        t.normalize();
        assert_eq!(run_texts(&t, 0), ["ab"]);

        let mut t = RichText { paragraphs: vec![] };
        t.normalize();
        assert_eq!(t.paragraphs.len(), 1);
    }

    #[test]
    fn toggle_on_mixed_range_sets_all_then_clears() {
        let mut t = text("Hello");
        t.apply(0..2, &StyleChange::Bold(true));
        let summary = t.summary(0..5);
        assert_eq!(summary.bold, Mixed::Mixed);
        let on = StyleSummary::toggle_value(&summary.bold);
        assert!(on);
        t.apply(0..5, &StyleChange::Bold(on));
        assert_eq!(t.summary(0..5).bold, Mixed::Same(true));
        assert_eq!(run_texts(&t, 0), ["Hello"]);
        let off = StyleSummary::toggle_value(&t.summary(0..5).bold);
        assert!(!off);
        t.apply(0..5, &StyleChange::Bold(off));
        assert_eq!(t.summary(0..5).bold, Mixed::Same(false));
        assert!(StyleSummary::toggle_value(&Mixed::Same(false)));
    }

    #[test]
    fn summary_reports_mixed_font_size_colour() {
        let mut t = RichText::new(plain_style(), HAlign::Left);
        t.insert(0, "ab\ncd", &plain_style());
        t.apply(1..2, &StyleChange::Font(Some("Menlo".into())));
        t.apply(1..2, &StyleChange::Size(24.0));
        t.apply(1..2, &StyleChange::Color(Some("#ff0000".into())));
        t.set_align(4..4, HAlign::Right);

        let all = t.summary(0..t.len());
        assert_eq!(all.font, Mixed::Mixed);
        assert_eq!(all.size, Mixed::Mixed);
        assert_eq!(all.color, Mixed::Mixed);
        assert_eq!(all.bold, Mixed::Same(false));
        assert_eq!(all.align, Mixed::Mixed);

        let b = t.summary(1..2);
        assert_eq!(b.font, Mixed::Same(Some("Menlo".to_string())));
        assert_eq!(b.size, Mixed::Same(24.0));
        assert_eq!(b.color, Mixed::Same(Some("#FF0000".to_string())));
        assert_eq!(b.align, Mixed::Same(HAlign::Left));

        // A caret reports the style typing would use.
        let caret = t.summary(2..2);
        assert_eq!(caret.size, Mixed::Same(24.0));
        assert_eq!(t.summary(5..5).align, Mixed::Same(HAlign::Right));
        // A selection of just the paragraph break reports where it starts.
        assert_eq!(t.summary(2..3).align, Mixed::Same(HAlign::Left));
    }

    #[test]
    fn style_at_caret_uses_previous_char() {
        let mut t = text("Hello");
        t.apply(0..2, &StyleChange::Bold(true));
        assert!(t.style_at(1).bold);
        assert!(t.style_at(2).bold);
        assert!(!t.style_at(3).bold);
        // At the start of a paragraph: the paragraph's style (its first run).
        assert!(t.style_at(0).bold);
    }

    #[test]
    fn empty_paragraph_keeps_typing_style() {
        let mut t = RichText::new(plain_style(), HAlign::Left);
        t.insert(0, "Hi", &bold_style());
        t.delete(0..2);
        assert!(t.is_empty());
        assert!(t.style_at(0).bold);
        assert!(t.paragraphs[0].style.bold);

        // Enter after bold text gives an empty paragraph that types bold.
        let mut t = text("ab");
        t.apply(0..2, &StyleChange::Bold(true));
        let caret = t.insert(2, "\n", &t.style_at(2));
        assert!(t.style_at(caret).bold);

        // Styling a caret in an empty paragraph changes what typing uses.
        let mut t = RichText::new(plain_style(), HAlign::Left);
        t.apply(0..0, &StyleChange::Italic(true));
        assert!(t.style_at(0).italic);
        // Selecting across an empty paragraph styles it too.
        let mut t = text("a\n\nb");
        t.apply(0..t.len(), &StyleChange::Size(30.0));
        assert_eq!(t.style_at(2).size, 30.0);
    }

    #[test]
    fn graphemes_are_never_split() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let flag = "\u{1F1EE}\u{1F1F3}";
        let accent = "e\u{301}";
        let s = format!("{family}{flag}{accent}");
        let t = text(&s);
        let (f, g, a) = (family.len(), flag.len(), accent.len());

        assert_eq!(t.next_grapheme(0), f);
        assert_eq!(t.next_grapheme(f), f + g);
        assert_eq!(t.next_grapheme(f + g), f + g + a);
        assert_eq!(t.next_grapheme(t.len()), t.len());
        assert_eq!(t.prev_grapheme(t.len()), f + g);
        assert_eq!(t.prev_grapheme(f + g), f);
        assert_eq!(t.prev_grapheme(f), 0);
        assert_eq!(t.prev_grapheme(0), 0);
        // From inside a cluster, moves go to the cluster's edges.
        assert_eq!(t.next_grapheme(4), f);
        assert_eq!(t.prev_grapheme(4), 0);

        // Backspace-equivalent deletes remove whole clusters.
        let mut d = t.clone();
        let end = d.len();
        d.delete(d.prev_grapheme(end)..end);
        assert_eq!(d.plain(), format!("{family}{flag}"));
        let end = d.len();
        d.delete(d.prev_grapheme(end)..end);
        assert_eq!(d.plain(), family);
        d.delete(d.prev_grapheme(f)..f);
        assert!(d.is_empty());

        // Positions inside a cluster snap: a range inside one removes it whole,
        // an insert lands before it, styling covers it whole.
        let mut d = t.clone();
        d.delete(f + 1..f + 2);
        assert_eq!(d.plain(), format!("{family}{accent}"));
        let mut d = t.clone();
        assert_eq!(d.insert(f + g + 1, "x", &plain_style()), f + g + 1);
        assert_eq!(d.plain(), format!("{family}{flag}x{accent}"));
        let mut d = t.clone();
        d.apply(3..4, &StyleChange::Bold(true));
        assert_eq!(d.paragraphs[0].runs[0].text, family);
        assert!(d.paragraphs[0].runs[0].style.bold);
        assert_eq!(d.slice(f + g + 1..f + g + 2).plain(), accent);
    }

    #[test]
    fn word_navigation_skips_punctuation_and_emoji() {
        let s = "Hi, \u{1F600} there! end";
        let t = text(s);
        let there = s.find("there").unwrap();
        let end = s.find("end").unwrap();
        assert_eq!(t.next_word(0), 2);
        assert_eq!(t.next_word(2), there + 5);
        assert_eq!(t.next_word(there + 1), there + 5);
        assert_eq!(t.next_word(there + 5), t.len());
        assert_eq!(t.next_word(t.len()), t.len());
        assert_eq!(t.prev_word(t.len()), end);
        assert_eq!(t.prev_word(end), there);
        assert_eq!(t.prev_word(there), 0);
        assert_eq!(t.prev_word(0), 0);

        assert_eq!(t.word_range(there + 2), there..there + 5);
        // Just after a word, the word before is chosen.
        assert_eq!(t.word_range(there + 5), there..there + 5);
        assert_eq!(t.word_range(t.len()), end..t.len());

        // Words cross paragraph breaks.
        let t = text("ab\ncd");
        assert_eq!(t.next_word(2), 5);
        assert_eq!(t.prev_word(3), 0);
        // An empty paragraph has an empty word.
        let t = text("a\n\nb");
        assert_eq!(t.word_range(2), 2..2);
    }

    #[test]
    fn slice_and_insert_rich_preserve_styles() {
        let mut src = RichText::new(plain_style(), HAlign::Left);
        src.insert(0, "abc\ndef", &plain_style());
        src.apply(1..2, &StyleChange::Bold(true));
        src.set_align(5..5, HAlign::Center);

        let piece = src.slice(1..6); // "bc\nde"
        assert_eq!(piece.plain(), "bc\nde");
        assert_eq!(run_texts(&piece, 0), ["b", "c"]);
        assert!(piece.paragraphs[0].runs[0].style.bold);
        assert_eq!(piece.paragraphs[1].align, HAlign::Center);
        assert_eq!(src.slice(3..4).plain(), "\n");
        assert_eq!(src.slice(2..2).plain(), "");

        let mut dst = text("XY");
        assert_eq!(dst.insert_rich(1, &piece), 1 + piece.len());
        assert_eq!(dst.plain(), "Xbc\ndeY");
        assert_eq!(dst.paragraphs[0].align, HAlign::Left);
        assert_eq!(dst.paragraphs[1].align, HAlign::Center);
        assert_eq!(run_texts(&dst, 0), ["X", "b", "c"]);
        assert!(dst.paragraphs[0].runs[1].style.bold);
        assert_eq!(run_texts(&dst, 1), ["deY"]);

        // A single-paragraph insert merges into the paragraph.
        let mut dst = text("XY");
        dst.insert_rich(1, &src.slice(0..2));
        assert_eq!(dst.plain(), "XabY");
        assert_eq!(dst.paragraphs.len(), 1);
        assert!(dst.style_at(3).bold);
    }

    #[test]
    fn size_and_colour_are_clamped_on_normalize() {
        let style = |size: f32, color: &str| RunStyle {
            size,
            color: Some(color.into()),
            ..RunStyle::default()
        };
        let mut t = RichText {
            paragraphs: vec![Paragraph {
                runs: vec![
                    Run {
                        text: "a".into(),
                        style: style(0.0, "#abcdef"),
                    },
                    Run {
                        text: "b".into(),
                        style: style(1e9, "red"),
                    },
                    Run {
                        text: "c".into(),
                        style: style(f32::NAN, "#000000"),
                    },
                ],
                align: HAlign::Left,
                style: style(-5.0, "nope"),
            }],
        };
        t.normalize();
        let runs = &t.paragraphs[0].runs;
        assert_eq!(runs[0].style.size, MIN_FONT_SIZE);
        assert_eq!(runs[0].style.color.as_deref(), Some("#ABCDEF"));
        assert_eq!(runs[1].style.size, MAX_FONT_SIZE);
        assert_eq!(runs[1].style.color, None);
        assert_eq!(runs[2].style.size, DEFAULT_FONT_SIZE);

        let mut t = text("x");
        t.apply(0..1, &StyleChange::Size(5000.0));
        assert_eq!(t.style_at(1).size, MAX_FONT_SIZE);
        t.apply_all(&StyleChange::Color(Some("bad".into())));
        assert_eq!(t.style_at(1).color, None);
    }

    #[test]
    fn apply_all_changes_one_property_everywhere() {
        let mut t = text("ab\ncd");
        t.apply(0..1, &StyleChange::Bold(true));
        t.apply_all(&StyleChange::Font(Some("Georgia".into())));
        assert!(t.style_at(1).bold);
        assert!(!t.style_at(2).bold);
        assert_eq!(
            t.summary(0..t.len()).font,
            Mixed::Same(Some("Georgia".into()))
        );
    }

    #[test]
    fn select_all_reaches_trailing_empty_paragraph() {
        let mut t = text("a\n");
        assert_eq!(t.paragraphs.len(), 2);
        let all = 0..t.len();
        t.set_align(all.clone(), HAlign::Center);
        assert_eq!(t.paragraphs[1].align, HAlign::Center);
        t.apply(all.clone(), &StyleChange::Bold(true));
        assert!(t.style_at(t.len()).bold);
        assert_eq!(t.summary(all.clone()).align, Mixed::Same(HAlign::Center));
        t.paragraphs[1].style.italic = true;
        assert_eq!(t.summary(all).italic, Mixed::Mixed);
    }

    #[test]
    fn insert_rich_caret_counts_normalised_text() {
        let other = RichText {
            paragraphs: vec![Paragraph {
                runs: vec![Run {
                    text: "x\ny".into(),
                    style: plain_style(),
                }],
                align: HAlign::Left,
                style: plain_style(),
            }],
        };
        let mut t = text("ab");
        let caret = t.insert_rich(1, &other);
        assert_eq!(t.plain(), "ax\u{2028}yb");
        assert_eq!(caret, t.len() - 1);
    }

    #[test]
    fn insert_caret_never_lands_inside_a_cluster() {
        let mut t = text("\u{301}");
        let caret = t.insert(0, "e", &plain_style());
        assert_eq!(t.plain(), "e\u{301}");
        assert_eq!(caret, t.len());
        let mut t = text("\u{301}");
        let caret = t.insert_rich(0, &text("e"));
        assert_eq!(caret, t.len());
    }

    #[test]
    fn serde_roundtrip_and_repairs_invariant() {
        let mut t = text("a\nb");
        t.apply(0..1, &StyleChange::Underline(true));
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("\"align\":\"left\""));
        let back: RichText = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
        let empty: RichText = serde_json::from_str(r#"{"paragraphs":[]}"#).unwrap();
        assert_eq!(empty.paragraphs.len(), 1);
        let style: RunStyle = serde_json::from_str("{}").unwrap();
        assert_eq!(style, RunStyle::default());
        assert_eq!(style.size, DEFAULT_FONT_SIZE);
    }
}
