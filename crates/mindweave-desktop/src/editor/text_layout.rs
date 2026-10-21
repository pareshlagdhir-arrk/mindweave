//! Lays out rich text with gpui's text shaping.
//!
//! gpui shapes a line at a single font size, so each line is shaped in
//! pieces of equal size and wrapped here: break opportunities follow
//! whitespace, and a word wider than the wrap width breaks between
//! graphemes. Layouts are computed in screen pixels at a zoom and report
//! geometry in canvas units, relative to the text's top-left corner.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, RwLock};

use gpui_kit::*;
use mindweave_core::geometry::{Pt, Rect};
use mindweave_core::text::{HAlign, RichText, RunStyle, LINE_BREAK};
use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;

use super::hex_to_hsla;
use super::text_paint::RotatedPaths;

/// Thickness of underlines and strikethroughs, as a fraction of the font
/// size (at least one pixel).
const DECORATION_THICKNESS: f32 = 1. / 16.;
/// Line height as a multiple of the largest font size on the line.
pub const LINE_HEIGHT: f32 = 1.25;
/// Width of the caret, in pixels.
const CARET_WIDTH_PX: f32 = 1.;
/// Width added to a selection that continues past the end of a line, as a
/// fraction of the line height.
const LINE_END_SELECTION: f32 = 0.25;

/// Text of one font size on one line, shaped by gpui.
pub(super) struct Piece {
    /// Byte offset of the piece's start in the text's `plain()`.
    pub start: usize,
    /// Pen position of the piece's start from the line's start, in pixels.
    pub x: f32,
    pub shaped: ShapedLine,
    /// The runs it was shaped with: colours and decorations by byte.
    pub runs: Vec<TextRun>,
}

impl Piece {
    fn end(&self) -> usize {
        self.start + self.shaped.len()
    }

    fn width(&self) -> f32 {
        f32::from(self.shaped.width())
    }

    /// Pen position of byte `offset` (in `plain()`) from the line's start.
    pub(super) fn x_of(&self, offset: usize) -> f32 {
        let i = offset.clamp(self.start, self.end()) - self.start;
        self.x + f32::from(self.shaped.x_for_index(i))
    }
}

/// One visual line. Pixel values are at the layout's zoom.
pub(super) struct Line {
    /// Bytes on the line, in `plain()`. A soft-wrapped line ends where the
    /// next one starts; a hard one before its line or paragraph break.
    pub range: Range<usize>,
    /// The last caret position on the line a click can give: `range.end`,
    /// or for a soft-wrapped line the start of its last character.
    click_end: usize,
    /// Alignment offset from the layout's left edge.
    pub x: f32,
    /// Top, from the layout's top.
    pub y: f32,
    pub height: f32,
    /// Baseline, from the line's top.
    pub baseline: f32,
    pub pieces: Vec<Piece>,
    /// Width of the visible text (trailing whitespace hangs).
    visible_width: f32,
    align: HAlign,
}

impl Line {
    /// Pen position of byte `offset` from the layout's left edge.
    fn x_of(&self, offset: usize) -> f32 {
        let piece = self.pieces.iter().rev().find(|p| p.start <= offset);
        self.x + piece.map_or(0., |p| p.x_of(offset))
    }
}

/// Rich text laid out in lines. Geometry is in canvas units relative to
/// the text's top-left corner; offsets are byte offsets into
/// [`RichText::plain`].
pub struct TextLayout {
    lines: Vec<Line>,
    /// Size in pixels at `zoom`.
    width: f32,
    height: f32,
    zoom: f32,
}

impl TextLayout {
    /// Lays out `text` for painting at `zoom`, wrapped at `wrap_width`
    /// canvas units (`None`: only at line and paragraph breaks). Runs
    /// without a colour use `default_color`, and its alpha applies to every
    /// run. Families that are not installed are drawn in the default font
    /// and passed to `missing_font`.
    ///
    /// Line breaks and the size come from a layout at zoom 1, so they do
    /// not change with the zoom; at another zoom the same lines are shaped
    /// again at the zoomed font size for crisp glyphs.
    pub fn layout(
        window: &mut Window,
        text: &RichText,
        wrap_width: Option<f32>,
        zoom: f32,
        default_color: Hsla,
        missing_font: &mut dyn FnMut(&str),
    ) -> TextLayout {
        let zoom = if zoom.is_finite() && zoom > 0. {
            zoom
        } else {
            1.
        };
        let wrap = wrap_width.map(|w| w.max(0.));
        let reference = Self::build(window, text, wrap, 1., default_color, missing_font, None);
        if zoom == 1. {
            return reference;
        }
        let breaks: Vec<Range<usize>> = reference.lines.iter().map(|l| l.range.clone()).collect();
        let mut zoomed = Self::build(
            window,
            text,
            wrap,
            zoom,
            default_color,
            &mut |_| {},
            Some(&breaks),
        );
        // Vertical metrics and the box come from the reference, scaled.
        zoomed.width = reference.width * zoom;
        zoomed.height = reference.height * zoom;
        if zoomed.lines.len() == reference.lines.len() {
            for (line, r) in zoomed.lines.iter_mut().zip(&reference.lines) {
                line.y = r.y * zoom;
                line.height = r.height * zoom;
                line.baseline = r.baseline * zoom;
            }
        }
        let width = zoomed.width;
        for line in &mut zoomed.lines {
            line.x = align_offset(line.align, width, line.visible_width);
        }
        zoomed
    }

    /// Lays out `text` at `zoom` with `wrap` in canvas units, breaking
    /// lines itself or at `forced` (line ranges of a reference layout).
    fn build(
        window: &mut Window,
        text: &RichText,
        wrap: Option<f32>,
        zoom: f32,
        default_color: Hsla,
        missing_font: &mut dyn FnMut(&str),
        forced: Option<&[Range<usize>]>,
    ) -> TextLayout {
        let text_system = window.text_system().clone();
        let (installed, _) = installed_fonts(&text_system);
        let mut builder = Builder {
            text_system: &text_system,
            installed,
            default_family: window.text_style().font().family,
            default_color,
            zoom,
            wrap: wrap.map(|w| w * zoom),
            forced,
            missing_font,
            lines: Vec::new(),
            y: 0.,
        };
        let mut offset = 0;
        for paragraph in &text.paragraphs {
            let mut plain = String::new();
            let mut styles = Vec::with_capacity(paragraph.runs.len());
            for run in &paragraph.runs {
                let start = plain.len();
                plain.push_str(&run.text);
                styles.push((start..plain.len(), &run.style));
            }
            for hard in hard_lines(&plain) {
                // An empty line is as tall as the text before its break.
                let style_before = styles
                    .iter()
                    .find(|(r, _)| r.start < hard.start && hard.start <= r.end)
                    .map_or(&paragraph.style, |(_, s)| *s);
                builder.hard_line(&plain, hard, &styles, style_before, offset, paragraph.align);
            }
            offset += plain.len() + 1;
        }
        let mut lines = builder.lines;
        let width = builder
            .wrap
            .unwrap_or_else(|| lines.iter().map(|l| l.visible_width).fold(0., f32::max));
        for line in &mut lines {
            line.x = align_offset(line.align, width, line.visible_width);
        }
        TextLayout {
            height: builder.y,
            lines,
            width,
            zoom,
        }
    }

    /// Width and height in canvas units: the wrap width when wrapped, else
    /// the widest line's.
    pub fn size(&self) -> (f32, f32) {
        (self.width / self.zoom, self.height / self.zoom)
    }

    pub(super) fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Stand-ins for the lines of too-small text, in canvas units from the
    /// text's top-left corner: a bar over each line's visible text, through
    /// the middle `GREEK_BAR` of its height.
    pub(super) fn greek_bars(&self) -> Vec<Rect> {
        self.lines
            .iter()
            .filter(|l| l.visible_width > 0.)
            .map(|l| {
                let bar = l.height * GREEK_BAR;
                self.rect(l.x, l.y + (l.height - bar) / 2., l.visible_width, bar)
            })
            .collect()
    }
}

/// Height of a greeked line's bar, as a fraction of the line height.
const GREEK_BAR: f32 = 0.4;

/// Caret geometry, for the text editor.
impl TextLayout {
    /// The line showing caret position `offset`: at a soft wrap, the
    /// later line.
    fn line_of(&self, offset: usize) -> Option<&Line> {
        self.lines
            .iter()
            .rev()
            .find(|l| l.range.start <= offset)
            .or(self.lines.first())
    }

    /// The caret at `offset`: a thin box as tall as its line (empty when
    /// there are no lines).
    pub fn caret_rect(&self, offset: usize) -> Rect {
        match self.line_of(offset) {
            Some(line) => self.rect(line.x_of(offset), line.y, CARET_WIDTH_PX, line.height),
            None => self.rect(0., 0., CARET_WIDTH_PX, 0.),
        }
    }

    /// Boxes covering `range`, one per line it touches; a line whose end
    /// the range continues past gets a little extra width.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let mut rects = Vec::new();
        for line in &self.lines {
            let start = range.start.max(line.range.start);
            let end = range.end.min(line.range.end);
            let continues = range.end > line.range.end && range.start <= line.range.end;
            if start > end || (start == end && !continues) {
                continue;
            }
            let x0 = line.x_of(start);
            let mut x1 = line.x_of(end);
            if continues {
                x1 += line.height * LINE_END_SELECTION;
            }
            rects.push(self.rect(x0, line.y, x1 - x0, line.height));
        }
        rects
    }

    /// The caret positions on the line showing `offset`: from its start to
    /// the last one a click on it gives (`None` when there are no lines).
    pub fn line_range(&self, offset: usize) -> Option<Range<usize>> {
        self.line_of(offset).map(|l| l.range.start..l.click_end)
    }

    /// The caret position on the line above (or, with `down`, below) the
    /// one showing `offset`, nearest `x` (canvas units from the text's left
    /// edge); `None` from the first (or last) line.
    pub fn vertical(&self, offset: usize, x: f32, down: bool) -> Option<usize> {
        let i = self
            .lines
            .iter()
            .rposition(|l| l.range.start <= offset)
            .unwrap_or(0);
        let j = if down { i + 1 } else { i.checked_sub(1)? };
        let line = self.lines.get(j)?;
        Some(self.offset_at((x, (line.y + line.height / 2.) / self.zoom)))
    }

    /// The caret position nearest `p` (canvas units from the text's
    /// top-left corner).
    pub fn offset_at(&self, p: Pt) -> usize {
        let (x, y) = (p.0 * self.zoom, p.1 * self.zoom);
        let Some(line) = self
            .lines
            .iter()
            .find(|l| y < l.y + l.height)
            .or(self.lines.last())
        else {
            return 0;
        };
        let x = x - line.x;
        let piece = line
            .pieces
            .iter()
            .find(|p| x < p.x + p.width())
            .or(line.pieces.last());
        let offset = piece.map_or(line.range.start, |p| {
            p.start + p.shaped.closest_index_for_x(px(x - p.x))
        });
        offset.clamp(line.range.start, line.click_end)
    }

    /// A box from pixel values, in canvas units.
    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        let z = self.zoom;
        Rect {
            x: x / z,
            y: y / z,
            w: w / z,
            h: h / z,
        }
    }
}

/// Builds the lines of a layout, one hard line at a time.
struct Builder<'a> {
    text_system: &'a WindowTextSystem,
    /// The installed font families.
    installed: Arc<HashSet<String>>,
    default_family: SharedString,
    default_color: Hsla,
    zoom: f32,
    /// Wrap width in pixels.
    wrap: Option<f32>,
    /// Line ranges (of the whole text) to break at instead of wrapping.
    forced: Option<&'a [Range<usize>]>,
    missing_font: &'a mut dyn FnMut(&str),
    lines: Vec<Line>,
    /// Top of the next line, in pixels.
    y: f32,
}

/// Text of one size on a hard line, shaped whole to measure it.
struct Segment {
    /// Bytes within the hard line.
    range: Range<usize>,
    size: f32,
    runs: Vec<TextRun>,
    shaped: ShapedLine,
    /// Pen position of its start from the hard line's start.
    x: f32,
}

impl Builder<'_> {
    fn family(&mut self, style: &RunStyle) -> SharedString {
        let installed = &self.installed;
        resolve_family(
            style.font.as_deref(),
            &self.default_family,
            &mut |name| installed.contains(name),
            self.missing_font,
        )
    }

    fn run(&mut self, style: &RunStyle, len: usize) -> TextRun {
        let family = self.family(style);
        text_run(style, len, family, self.default_color, self.zoom)
    }

    fn shape(&self, text: &str, size: f32, runs: &[TextRun]) -> ShapedLine {
        self.text_system.shape_line(
            SharedString::from(text.to_string()),
            px(size * self.zoom),
            runs,
            None,
        )
    }

    /// Lays out `plain[hard]`, a line between breaks of a paragraph whose
    /// runs cover `styles` and that starts at `offset` in the whole text.
    fn hard_line(
        &mut self,
        plain: &str,
        hard: Range<usize>,
        styles: &[(Range<usize>, &RunStyle)],
        style_before: &RunStyle,
        offset: usize,
        align: HAlign,
    ) {
        let text = &plain[hard.clone()];
        let base = offset + hard.start;
        if text.is_empty() {
            let run = self.run(style_before, 0);
            let size = style_before.size * self.zoom;
            let id = self.text_system.resolve_font(&run.font);
            let ascent = f32::from(self.text_system.ascent(id, px(size)));
            let descent = f32::from(self.text_system.descent(id, px(size)));
            self.push_line(
                base..base,
                base,
                Vec::new(),
                size,
                ascent,
                descent,
                0.,
                align,
            );
            return;
        }
        // The paragraph's runs clipped to this line, from its start.
        let local: Vec<(Range<usize>, &RunStyle)> = styles
            .iter()
            .filter_map(|(r, s)| {
                let (a, b) = (r.start.max(hard.start), r.end.min(hard.end));
                (a < b).then(|| (a - hard.start..b - hard.start, *s))
            })
            .collect();
        let sizes: Vec<(Range<usize>, f32)> =
            local.iter().map(|(r, s)| (r.clone(), s.size)).collect();
        let mut segments = Vec::new();
        let mut x = 0.;
        for (range, size) in size_segments(&sizes) {
            let runs: Vec<TextRun> = local
                .iter()
                .filter(|(r, _)| r.start >= range.start && r.end <= range.end)
                .map(|(r, s)| self.run(s, r.len()))
                .collect();
            let shaped = self.shape(&text[range.clone()], size, &runs);
            let width = f32::from(shaped.width());
            segments.push(Segment {
                range,
                size,
                runs,
                shaped,
                x,
            });
            x += width;
        }
        let pen = |i: usize| {
            let seg = segments
                .iter()
                .rev()
                .find(|s| s.range.start <= i)
                .unwrap_or(&segments[0]);
            seg.x + f32::from(seg.shaped.x_for_index(i - seg.range.start))
        };
        let breaks = match self.forced {
            Some(lines) => forced_breaks(lines, base, text.len()),
            None => break_lines(text, &pen, self.wrap),
        };
        let count = breaks.len();
        for (k, line) in breaks.into_iter().enumerate() {
            let mut pieces = Vec::new();
            let (mut size, mut ascent, mut descent) = (0f32, 0f32, 0f32);
            for seg in &segments {
                let (a, b) = (line.start.max(seg.range.start), line.end.min(seg.range.end));
                if a >= b {
                    continue;
                }
                let (shaped, runs) = if a == seg.range.start && b == seg.range.end {
                    (seg.shaped.clone(), seg.runs.clone())
                } else {
                    let runs = slice_runs(&seg.runs, a - seg.range.start..b - seg.range.start);
                    (self.shape(&text[a..b], seg.size, &runs), runs)
                };
                size = size.max(seg.size * self.zoom);
                ascent = ascent.max(f32::from(shaped.ascent));
                descent = descent.max(f32::from(shaped.descent));
                pieces.push(Piece {
                    start: base + a,
                    x: pen(a) - pen(line.start),
                    shaped,
                    runs,
                });
            }
            let visible = pen(visible_end(text, line.start, line.end)) - pen(line.start);
            let click_end = if k + 1 < count {
                let last = text[..line.end]
                    .chars()
                    .next_back()
                    .map_or(0, char::len_utf8);
                line.end - last
            } else {
                line.end
            };
            self.push_line(
                base + line.start..base + line.end,
                base + click_end.max(line.start),
                pieces,
                size,
                ascent,
                descent,
                visible,
                align,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_line(
        &mut self,
        range: Range<usize>,
        click_end: usize,
        pieces: Vec<Piece>,
        size: f32,
        ascent: f32,
        descent: f32,
        visible_width: f32,
        align: HAlign,
    ) {
        let height = (size * LINE_HEIGHT).max(ascent + descent);
        let baseline = (height - ascent - descent) / 2. + ascent;
        self.lines.push(Line {
            range,
            click_end,
            x: 0.,
            y: self.y,
            height,
            baseline,
            pieces,
            visible_width,
            align,
        });
        self.y += height;
    }
}

/// `runs` (covering bytes from 0) cut to `range`.
fn slice_runs(runs: &[TextRun], range: Range<usize>) -> Vec<TextRun> {
    let mut out = Vec::new();
    let mut start = 0;
    for run in runs {
        let end = start + run.len;
        let (a, b) = (start.max(range.start), end.min(range.end));
        if a < b {
            out.push(TextRun {
                len: b - a,
                ..run.clone()
            });
        }
        start = end;
    }
    out
}

/// The installed font families, and a generation that moves on whenever
/// they change.
struct InstalledFonts {
    families: Arc<HashSet<String>>,
    generation: u64,
}

impl InstalledFonts {
    fn new(names: impl IntoIterator<Item = String>) -> Self {
        InstalledFonts {
            families: Arc::new(names.into_iter().collect()),
            generation: 0,
        }
    }

    /// Replaces the families with `names`; returns whether they changed
    /// (then the generation moves on).
    fn replace(&mut self, names: impl IntoIterator<Item = String>) -> bool {
        let families: HashSet<String> = names.into_iter().collect();
        if families == *self.families {
            return false;
        }
        self.families = Arc::new(families);
        self.generation += 1;
        true
    }
}

/// Read when first needed, and again by [`refresh_installed_fonts`].
static INSTALLED: RwLock<Option<InstalledFonts>> = RwLock::new(None);

/// The installed font families and their generation.
fn installed_fonts(text_system: &TextSystem) -> (Arc<HashSet<String>>, u64) {
    if let Some(fonts) = INSTALLED.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return (fonts.families.clone(), fonts.generation);
    }
    let mut slot = INSTALLED.write().unwrap_or_else(|e| e.into_inner());
    let fonts = slot.get_or_insert_with(|| InstalledFonts::new(text_system.all_font_names()));
    (fonts.families.clone(), fonts.generation)
}

/// Takes `names` as the installed font families (the font list read them
/// as it opened), so fonts installed while the app runs are drawn, and no
/// longer reported missing, from the next frame.
pub(super) fn refresh_installed_fonts(names: &[String]) {
    let mut slot = INSTALLED.write().unwrap_or_else(|e| e.into_inner());
    match slot.as_mut() {
        Some(fonts) => {
            fonts.replace(names.iter().cloned());
        }
        None => *slot = Some(InstalledFonts::new(names.iter().cloned())),
    }
}

/// What layouts depend on beyond their text: the window's default font
/// family and the installed font set.
fn fonts_key(window: &Window) -> u64 {
    let (_, generation) = installed_fonts(window.text_system());
    let mut h = DefaultHasher::new();
    window.text_style().font().family.hash(&mut h);
    generation.hash(&mut h);
    h.finish()
}

/// Adds `name` to `missing` unless it is there.
fn note_missing(missing: &mut Vec<String>, name: &str) {
    if !missing.iter().any(|m| m == name) {
        missing.push(name.to_string());
    }
}

/// A hash of everything in `text` that affects its layout and look.
pub(super) fn text_hash(text: &RichText) -> u64 {
    fn style(s: &RunStyle, h: &mut DefaultHasher) {
        s.font.hash(h);
        s.size.to_bits().hash(h);
        (s.bold, s.italic, s.underline, s.strike).hash(h);
        s.color.hash(h);
    }
    let mut h = DefaultHasher::new();
    for p in &text.paragraphs {
        (p.align as u8).hash(&mut h);
        style(&p.style, &mut h);
        p.runs.len().hash(&mut h);
        for run in &p.runs {
            run.text.hash(&mut h);
            style(&run.style, &mut h);
        }
    }
    h.finish()
}

/// What a cached layout was computed from, apart from the zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ContentKey {
    text: u64,
    wrap: Option<u32>,
    color: [u32; 4],
    /// See [`fonts_key`].
    fonts: u64,
}

impl ContentKey {
    fn new(text: &RichText, wrap: Option<f32>, color: Hsla, fonts: u64) -> Self {
        ContentKey {
            text: text_hash(text),
            wrap: wrap.map(f32::to_bits),
            color: [color.h, color.s, color.l, color.a].map(f32::to_bits),
            fonts,
        }
    }

    /// Whether layouts for both keys have the same size.
    fn same_size(&self, other: &Self) -> bool {
        self.text == other.text && self.wrap == other.wrap && self.fonts == other.fonts
    }
}

/// A cached layout and the paths of its rotated glyphs.
pub(super) struct CacheEntry {
    content: ContentKey,
    zoom: f32,
    pub layout: TextLayout,
    /// Glyph outlines last painted rotated (see `text_paint`).
    pub rotated: Option<RotatedPaths>,
    /// Font families the text names that are not installed (drawn in the
    /// default font), each once.
    missing: Vec<String>,
    used: u64,
}

/// Text layouts by object id, reused while the text, wrap width, colour
/// and zoom stay the same. Entries not used in a frame are dropped.
#[derive(Default)]
pub struct TextLayoutCache {
    entries: HashMap<Uuid, CacheEntry>,
    frame: u64,
    /// The fonts this frame's layouts use (see [`fonts_key`]).
    fonts: u64,
}

impl TextLayoutCache {
    /// Starts a frame; entries not used by [`Self::end_frame`] are dropped.
    /// A change of the default font or of the installed fonts lays every
    /// text out again.
    pub fn begin_frame(&mut self, window: &Window) {
        self.frame = self.frame.wrapping_add(1);
        self.fonts = fonts_key(window);
    }

    /// The font families object `id`'s text names that are not installed,
    /// as found when it was last laid out (none when it has no layout).
    pub fn missing_fonts(&self, id: Uuid) -> &[String] {
        self.entries.get(&id).map_or(&[], |e| &e.missing)
    }

    /// Drops the entries not used since [`Self::begin_frame`].
    pub fn end_frame(&mut self) {
        let frame = self.frame;
        self.entries.retain(|_, e| e.used == frame);
    }

    /// Keeps `id`'s entry for this frame without laying it out.
    pub fn keep(&mut self, id: Uuid) {
        if let Some(e) = self.entries.get_mut(&id) {
            e.used = self.frame;
        }
    }

    /// The size (canvas units) of `text` laid out for object `id`, which
    /// does not depend on the zoom: from its cached layout when the text
    /// and wrap width match, else from a new layout at zoom 1.
    pub fn size(
        &mut self,
        window: &mut Window,
        id: Uuid,
        text: &RichText,
        wrap: Option<f32>,
        color: Hsla,
    ) -> (f32, f32) {
        let key = ContentKey::new(text, wrap, color, self.fonts);
        let frame = self.frame;
        if let Some(e) = self.entries.get_mut(&id) {
            if e.content.same_size(&key) {
                e.used = frame;
                return e.layout.size();
            }
        }
        self.insert(window, id, key, text, wrap, 1., color)
            .layout
            .size()
    }

    /// The layout cached for `id` when its text and wrap width match `text`
    /// and `wrap` (at any zoom and colour), kept for this frame.
    pub fn cached(&mut self, id: Uuid, text: &RichText, wrap: Option<f32>) -> Option<&TextLayout> {
        let key = ContentKey::new(text, wrap, Hsla::default(), self.fonts);
        let frame = self.frame;
        let e = self.entries.get_mut(&id)?;
        if !e.content.same_size(&key) {
            return None;
        }
        e.used = frame;
        Some(&e.layout)
    }

    /// The entry for `text` laid out for object `id` at `zoom`, laid out
    /// again on any change.
    pub fn entry(
        &mut self,
        window: &mut Window,
        id: Uuid,
        text: &RichText,
        wrap: Option<f32>,
        zoom: f32,
        color: Hsla,
    ) -> &mut CacheEntry {
        let key = ContentKey::new(text, wrap, color, self.fonts);
        let hit = self
            .entries
            .get(&id)
            .is_some_and(|e| e.content == key && e.zoom == zoom);
        if !hit {
            return self.insert(window, id, key, text, wrap, zoom, color);
        }
        let e = self.entries.get_mut(&id).expect("checked above");
        e.used = self.frame;
        e
    }

    #[allow(clippy::too_many_arguments)]
    fn insert(
        &mut self,
        window: &mut Window,
        id: Uuid,
        content: ContentKey,
        text: &RichText,
        wrap: Option<f32>,
        zoom: f32,
        color: Hsla,
    ) -> &mut CacheEntry {
        let mut missing = Vec::new();
        let layout = TextLayout::layout(window, text, wrap, zoom, color, &mut |name| {
            note_missing(&mut missing, name)
        });
        let entry = CacheEntry {
            content,
            zoom,
            layout,
            rotated: None,
            missing,
            used: self.frame,
        };
        self.entries.insert(id, entry);
        self.entries.get_mut(&id).expect("just inserted")
    }
}

/// The font family a run is drawn in: `font`, or `default` when unset or
/// not `installed` (then reported to `missing`).
pub(super) fn resolve_family(
    font: Option<&str>,
    default: &SharedString,
    installed: &mut dyn FnMut(&str) -> bool,
    missing: &mut dyn FnMut(&str),
) -> SharedString {
    match font {
        None => default.clone(),
        Some(name) if installed(name) => SharedString::from(name.to_string()),
        Some(name) => {
            missing(name);
            default.clone()
        }
    }
}

/// The gpui run for `len` bytes in `style`, drawn in `family` at `zoom`.
/// `color: None` uses `default_color`; explicit colours take its alpha, so
/// the alpha carries the object's opacity.
pub(super) fn text_run(
    style: &RunStyle,
    len: usize,
    family: SharedString,
    default_color: Hsla,
    zoom: f32,
) -> TextRun {
    let color = match style.color.as_deref().and_then(hex_to_hsla) {
        Some(c) => c.opacity(default_color.a),
        None => default_color,
    };
    let thickness = px((style.size * zoom * DECORATION_THICKNESS).max(1.));
    TextRun {
        len,
        font: Font {
            family,
            features: FontFeatures::default(),
            fallbacks: None,
            weight: if style.bold {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            },
            style: if style.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
        },
        color,
        background_color: None,
        underline: style.underline.then_some(UnderlineStyle {
            thickness,
            color: Some(color),
            wavy: false,
        }),
        strikethrough: style.strike.then_some(StrikethroughStyle {
            thickness,
            color: Some(color),
        }),
    }
}

/// Byte ranges of `text` between line breaks ([`LINE_BREAK`]).
pub(super) fn hard_lines(text: &str) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, _) in text.match_indices(LINE_BREAK) {
        lines.push(start..i);
        start = i + LINE_BREAK.len_utf8();
    }
    lines.push(start..text.len());
    lines
}

/// Splits `styles` (consecutive byte ranges with their font size) into
/// maximal ranges of one size.
pub(super) fn size_segments(styles: &[(Range<usize>, f32)]) -> Vec<(Range<usize>, f32)> {
    let mut segments: Vec<(Range<usize>, f32)> = Vec::new();
    for (range, size) in styles {
        match segments.last_mut() {
            Some((last, last_size)) if *last_size == *size && last.end == range.start => {
                last.end = range.end;
            }
            _ => segments.push((range.clone(), *size)),
        }
    }
    segments
}

/// `end`, moved back over the whitespace before it (but not before
/// `start`): where a line's visible text ends.
pub(super) fn visible_end(text: &str, start: usize, end: usize) -> usize {
    start + text[start..end].trim_end().len()
}

/// Wraps `text` at `wrap` pixels, where `x(i)` is the pen position at byte
/// `i`: greedy, breaking after whitespace, and between graphemes inside a
/// word wider than a line. Trailing whitespace hangs past the width. Each
/// line holds at least one grapheme. Unwrapped text is one line.
pub(super) fn break_lines(
    text: &str,
    x: &dyn Fn(usize) -> f32,
    wrap: Option<f32>,
) -> Vec<Range<usize>> {
    let Some(wrap) = wrap else {
        return std::iter::once(0..text.len()).collect();
    };
    // Word starts: the first non-whitespace after whitespace.
    let mut candidates = Vec::new();
    let mut prev_space = false;
    for (i, c) in text.char_indices() {
        let space = c.is_whitespace();
        if prev_space && !space {
            candidates.push(i);
        }
        prev_space = space;
    }
    let fits = |start: usize, end: usize| x(visible_end(text, start, end)) - x(start) <= wrap;
    let mut lines = Vec::new();
    let mut start = 0;
    let mut next = 0; // first candidate not yet passed
    loop {
        if fits(start, text.len()) {
            lines.push(start..text.len());
            return lines;
        }
        while next < candidates.len() && candidates[next] <= start {
            next += 1;
        }
        let mut end = None;
        while next < candidates.len() && fits(start, candidates[next]) {
            end = Some(candidates[next]);
            next += 1;
        }
        let end = end.unwrap_or_else(|| grapheme_break(text, start, x, wrap));
        lines.push(start..end);
        if end >= text.len() {
            return lines;
        }
        start = end;
    }
}

/// The last grapheme boundary after `start` within `wrap` pixels of it,
/// and at least the first one.
fn grapheme_break(text: &str, start: usize, x: &dyn Fn(usize) -> f32, wrap: f32) -> usize {
    let mut end = None;
    for (i, g) in text[start..].grapheme_indices(true) {
        let boundary = start + i + g.len();
        if end.is_some() && x(boundary) - x(start) > wrap {
            break;
        }
        end = Some(boundary);
    }
    end.unwrap_or(text.len())
}

/// The lines of a reference layout (byte ranges of the whole text) that
/// fall on the hard line of `len` bytes starting at `base`, from its start;
/// the whole hard line when none do.
pub(super) fn forced_breaks(lines: &[Range<usize>], base: usize, len: usize) -> Vec<Range<usize>> {
    let local: Vec<Range<usize>> = lines
        .iter()
        .filter(|l| l.start >= base && l.end <= base + len)
        .map(|l| l.start - base..l.end - base)
        .collect();
    if local.is_empty() {
        std::iter::once(0..len).collect()
    } else {
        local
    }
}

/// Smallest on-screen font size, in pixels, at which glyphs are drawn;
/// smaller text is drawn as bars.
pub const GREEK_BELOW_PX: f32 = 4.;

/// Whether text whose largest font size is `max_size` canvas units is too
/// small at `zoom` to draw its glyphs.
pub fn should_greek(max_size: f32, zoom: f32) -> bool {
    max_size * zoom < GREEK_BELOW_PX
}

/// The largest font size in `text`, runs and (for empty paragraphs)
/// paragraph styles.
pub fn max_font_size(text: &RichText) -> f32 {
    text.paragraphs
        .iter()
        .flat_map(|p| {
            let runs = p.runs.iter().map(|r| r.style.size);
            std::iter::once(p.style.size)
                .filter(move |_| p.runs.is_empty())
                .chain(runs)
        })
        .fold(0., f32::max)
}

/// Where a line `width` wide starts in `available` pixels.
pub(super) fn align_offset(align: HAlign, available: f32, width: f32) -> f32 {
    let free = (available - width).max(0.);
    match align {
        HAlign::Left => 0.,
        HAlign::Center => free / 2.,
        HAlign::Right => free,
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{
        align_offset, break_lines, forced_breaks, hard_lines, max_font_size, note_missing,
        resolve_family, should_greek, size_segments, text_run, visible_end, ContentKey,
        InstalledFonts,
    };
    use gpui_kit::{FontStyle, FontWeight, Hsla, SharedString};
    use mindweave_core::{HAlign, RichText, RunStyle};

    #[test]
    fn the_installed_font_set_moves_on_only_when_it_changes() {
        let mut fonts = InstalledFonts::new(["Arial".to_string(), "Menlo".to_string()]);
        let generation = fonts.generation;
        assert!(fonts.families.contains("Menlo"));
        // The same families in another order: nothing to lay out again.
        assert!(!fonts.replace(["Menlo".to_string(), "Arial".to_string()]));
        assert_eq!(fonts.generation, generation);
        // A font installed while running: layouts keyed on the old set are
        // stale, so the generation moves on.
        assert!(fonts.replace(["Menlo", "Arial", "Georgia"].map(String::from)));
        assert_ne!(fonts.generation, generation);
        assert!(fonts.families.contains("Georgia"));
    }

    #[test]
    fn layouts_are_keyed_on_the_fonts_they_were_drawn_with() {
        let text = RichText::new(RunStyle::default(), HAlign::Left);
        let a = ContentKey::new(&text, None, gpui_kit::black(), 1);
        let b = ContentKey::new(&text, None, gpui_kit::black(), 2);
        assert!(a.same_size(&a));
        assert!(!a.same_size(&b), "another default font or font set");
    }

    #[test]
    fn missing_families_are_noted_once() {
        let mut missing = Vec::new();
        note_missing(&mut missing, "Nope Sans");
        note_missing(&mut missing, "Gone Serif");
        note_missing(&mut missing, "Nope Sans");
        assert_eq!(missing, ["Nope Sans", "Gone Serif"]);
    }

    fn black() -> Hsla {
        gpui_kit::black()
    }

    #[test]
    fn families_fall_back_to_the_default_and_report_missing_ones() {
        let default = SharedString::from("Default UI");
        let mut missing = Vec::new();
        let mut installed = |name: &str| name == "Georgia";
        let mut report = |name: &str| missing.push(name.to_string());
        let f = |font: Option<&str>, i: &mut dyn FnMut(&str) -> bool, r: &mut dyn FnMut(&str)| {
            resolve_family(font, &default, i, r)
        };
        assert_eq!(f(None, &mut installed, &mut report), default);
        assert_eq!(f(Some("Georgia"), &mut installed, &mut report), "Georgia");
        assert_eq!(f(Some("Nope Sans"), &mut installed, &mut report), default);
        assert_eq!(missing, vec!["Nope Sans".to_string()]);
    }

    #[test]
    fn runs_carry_weight_style_decorations_and_colour() {
        let plain = RunStyle::default();
        let run = text_run(&plain, 5, "Georgia".into(), black().opacity(0.5), 2.);
        assert_eq!(run.len, 5);
        assert_eq!(run.font.family, "Georgia");
        assert_eq!(run.font.weight, FontWeight::NORMAL);
        assert_eq!(run.font.style, FontStyle::Normal);
        assert_eq!(run.color, black().opacity(0.5));
        assert!(run.underline.is_none() && run.strikethrough.is_none());

        let styled = RunStyle {
            bold: true,
            italic: true,
            underline: true,
            strike: true,
            color: Some("#FF0000".into()),
            size: 16.,
            ..RunStyle::default()
        };
        let run = text_run(&styled, 3, "X".into(), black().opacity(0.5), 2.);
        assert_eq!(run.font.weight, FontWeight::BOLD);
        assert_eq!(run.font.style, FontStyle::Italic);
        let red = gpui_kit::red();
        // The explicit colour takes the default colour's alpha (opacity).
        assert!((run.color.h - red.h).abs() < 1e-3 && (run.color.s - red.s).abs() < 1e-3);
        assert_eq!(run.color.a, 0.5);
        let underline = run.underline.unwrap();
        assert_eq!(underline.color, Some(run.color));
        // 16 units at zoom 2: 32px, so a 2px line.
        assert_eq!(f32::from(underline.thickness), 2.);
        assert_eq!(run.strikethrough.unwrap().color, Some(run.color));
        // Thin text still gets a visible line.
        let small = RunStyle {
            underline: true,
            size: 4.,
            ..RunStyle::default()
        };
        let run = text_run(&small, 1, "X".into(), black(), 1.);
        assert_eq!(f32::from(run.underline.unwrap().thickness), 1.);
    }

    #[test]
    fn hard_lines_split_at_line_breaks() {
        assert_eq!(hard_lines(""), vec![0..0]);
        assert_eq!(hard_lines("ab"), vec![0..2]);
        // U+2028 is three bytes.
        assert_eq!(hard_lines("ab\u{2028}c"), vec![0..2, 5..6]);
        assert_eq!(hard_lines("\u{2028}"), vec![0..0, 3..3]);
    }

    #[test]
    fn size_segments_merge_equal_sizes() {
        let styles = vec![(0..2, 14.), (2..5, 14.), (5..6, 20.), (6..9, 14.)];
        assert_eq!(
            size_segments(&styles),
            vec![(0..5, 14.), (5..6, 20.), (6..9, 14.)]
        );
        assert!(size_segments(&[]).is_empty());
    }

    #[test]
    fn visible_end_skips_trailing_whitespace() {
        assert_eq!(visible_end("ab  ", 0, 4), 2);
        assert_eq!(visible_end("ab  cd", 0, 6), 6);
        assert_eq!(visible_end("   ", 1, 3), 1);
    }

    /// Ten pixels per byte.
    fn mono(i: usize) -> f32 {
        i as f32 * 10.
    }

    #[test]
    fn unwrapped_text_is_one_line() {
        assert_eq!(break_lines("hello world", &mono, None), vec![0..11]);
        assert_eq!(break_lines("", &mono, Some(5.)), vec![0..0]);
    }

    #[test]
    fn lines_break_after_whitespace_and_spaces_hang() {
        // "hello " fits in 60px; trailing spaces hang past the width.
        assert_eq!(
            break_lines("hello world", &mono, Some(60.)),
            vec![0..6, 6..11]
        );
        assert_eq!(
            break_lines("hello   world", &mono, Some(50.)),
            vec![0..8, 8..13]
        );
        // Two words fit on one line.
        assert_eq!(break_lines("a b c d", &mono, Some(30.)), vec![0..4, 4..7]);
    }

    #[test]
    fn long_words_break_between_graphemes() {
        assert_eq!(
            break_lines("abcdefg", &mono, Some(30.)),
            vec![0..3, 3..6, 6..7]
        );
        // At least one grapheme per line, even narrower than one.
        assert_eq!(break_lines("ab", &mono, Some(1.)), vec![0..1, 1..2]);
        // "é" as e + combining acute (3 bytes) is never split.
        let text = "e\u{301}e\u{301}";
        assert_eq!(break_lines(text, &mono, Some(10.)), vec![0..3, 3..6]);
        // After a long word, the next word starts a new line.
        assert_eq!(
            break_lines("abcdef gh", &mono, Some(40.)),
            vec![0..4, 4..7, 7..9]
        );
    }

    #[test]
    fn zoomed_layouts_reuse_the_reference_breaks() {
        // Reference lines of "hello world\u{2028}again" (hard lines at 0 and 14).
        let lines = [0..6, 6..11, 14..19];
        assert_eq!(forced_breaks(&lines, 0, 11), vec![0..6, 6..11]);
        assert_eq!(forced_breaks(&lines, 14, 5), vec![0..5]);
        // Text the reference does not know about stays one line.
        assert_eq!(forced_breaks(&lines, 30, 4), vec![0..4]);
        // Breaks found at one zoom are replayed exactly at another.
        let at_1 = break_lines("hello world", &|i| i as f32 * 10., Some(60.));
        assert_eq!(forced_breaks(&at_1, 0, 11), at_1);
    }

    #[test]
    fn tiny_text_is_greeked() {
        assert!(should_greek(16., 0.2)); // 3.2px
        assert!(!should_greek(16., 0.25)); // 4px
        assert!(!should_greek(16., 1.));
        assert!(should_greek(2., 1.));
        let mut text = mindweave_core::RichText::new(RunStyle::default(), HAlign::Left);
        assert_eq!(max_font_size(&text), 16.);
        let big = RunStyle {
            size: 30.,
            ..RunStyle::default()
        };
        let small = RunStyle {
            size: 8.,
            ..RunStyle::default()
        };
        text.insert(0, "a", &big);
        text.insert(1, "\nb", &small);
        assert_eq!(max_font_size(&text), 30.);
    }

    #[test]
    fn alignment_offsets() {
        assert_eq!(align_offset(HAlign::Left, 100., 40.), 0.);
        assert_eq!(align_offset(HAlign::Center, 100., 40.), 30.);
        assert_eq!(align_offset(HAlign::Right, 100., 40.), 60.);
        // Wider than the space: no negative offset.
        assert_eq!(align_offset(HAlign::Right, 10., 40.), 0.);
    }
}
