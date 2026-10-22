//! Equations on topics: LaTeX (with mhchem chemistry) rendered by RaTeX
//! to an SVG with its glyphs as outlines, cached by what was asked for.
//! Painting rasterises the SVG at the zoom it is shown at.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::{App, Hsla, RenderImage, Rgba, Window};
use mindweave_core::{RichText, DEFAULT_FONT_SIZE};
use ratex_layout::{layout, to_display_list, LayoutOptions};
use ratex_parser::parser::parse;
use ratex_svg::{render_to_svg, SvgOptions};
use ratex_types::color::Color;
use ratex_types::math_style::MathStyle;

/// The most characters of LaTeX accepted.
pub const MAX_EQUATION_CHARS: usize = 4_000;
/// The longest side a rendered equation may have, in canvas units.
const MAX_SIDE: f32 = 800.;

/// Pixels per point when an SVG is rasterised: RaTeX sizes its SVG in
/// `pt`, which the rasteriser reads at 96 dpi, so this undoes that to
/// give one pixel per canvas unit at scale 1.
const PX_PER_PT: f32 = 96. / 72.;

/// An equation rendered to SVG.
pub struct RenderedEquation {
    /// The SVG document.
    pub svg: Arc<[u8]>,
    /// Its size in canvas units at zoom 1.
    pub size: (f32, f32),
}

/// The SVG for `latex` in `color` at `font_size`, and its size.
fn render_svg(latex: &str, color: Hsla, font_size: f32) -> Result<(String, (f32, f32)), String> {
    let ast = parse(latex).map_err(|e| format!("{e}"))?;
    let c = Rgba::from(color);
    let options = LayoutOptions::default()
        .with_style(MathStyle::Display)
        .with_color(Color::new(c.r, c.g, c.b, c.a));
    let list = to_display_list(&layout(&ast, &options));
    let svg = render_to_svg(
        &list,
        &SvgOptions {
            font_size: font_size as f64,
            padding: 0.,
            embed_glyphs: true,
            ..SvgOptions::default()
        },
    );
    let size = svg_size(&svg).ok_or("Equation has no size")?;
    Ok((svg, size))
}

/// The `width` and `height` of the root element of `svg`, unitless.
fn svg_size(svg: &str) -> Option<(f32, f32)> {
    let root = &svg[svg.find("<svg")?..];
    let root = &root[..root.find('>')?];
    let attr = |name: &str| -> Option<f32> {
        let from = root.find(&format!(" {name}=\""))? + name.len() + 3;
        let value = &root[from..];
        // RaTeX writes the unit (`pt`) after the number; it is canvas units.
        value[..value.find('"')?]
            .trim_end_matches("pt")
            .parse()
            .ok()
    };
    Some((attr("width")?, attr("height")?))
}

/// Renders `latex` in `color` with an em of `font_size` canvas units
/// ([`DEFAULT_FONT_SIZE`] when that is not a positive number). The
/// equation is scaled down to fit [`MAX_SIDE`]. Errors are RaTeX's parse
/// message.
pub fn render_equation(
    latex: &str,
    color: Hsla,
    font_size: f32,
) -> Result<RenderedEquation, String> {
    if latex.chars().count() > MAX_EQUATION_CHARS {
        return Err("Equation is too long".to_string());
    }
    let font_size = if font_size.is_finite() && font_size > 0. {
        font_size
    } else {
        DEFAULT_FONT_SIZE
    };
    let (mut svg, mut size) = render_svg(latex, color, font_size)?;
    let longest = size.0.max(size.1);
    if longest > MAX_SIDE {
        (svg, size) = render_svg(latex, color, font_size * MAX_SIDE / longest)?;
    }
    Ok(RenderedEquation {
        svg: Arc::from(svg.into_bytes()),
        size,
    })
}

/// Rendered SVG `svg` rasterised at `scale` pixels per canvas unit;
/// `None` when it does not rasterise.
pub fn rasterise(svg: &[u8], scale: f32, cx: &App) -> Option<Arc<RenderImage>> {
    cx.svg_renderer()
        .render_single_frame(svg, scale / PX_PER_PT)
        .ok()
}

/// The em of an equation under `label`: the topic text's largest font
/// size, [`DEFAULT_FONT_SIZE`] when it has none.
pub fn equation_font_size(label: Option<&RichText>) -> f32 {
    let largest = label.map_or(0., crate::editor::max_font_size);
    if largest > 0. {
        largest
    } else {
        DEFAULT_FONT_SIZE
    }
}

/// What an equation is rendered from: its LaTeX, colour and size.
type Key = (String, u32, u32);

/// A render, its rasters by scale (`None`: the SVG did not rasterise)
/// and the frame it was last used in.
struct Entry {
    render: Result<RenderedEquation, String>,
    rasters: HashMap<u32, Option<Arc<RenderImage>>>,
    used: u64,
}

/// Rendered equations, by LaTeX, colour and font size, with their rasters
/// for painting.
#[derive(Default)]
pub struct EquationCache {
    entries: HashMap<Key, Entry>,
    frame: u64,
}

/// The cache key for `latex` in `color` at `font_size`.
fn key(latex: &str, color: Hsla, font_size: f32) -> Key {
    let c = Rgba::from(color);
    let channel = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    let rgba = channel(c.r) << 24 | channel(c.g) << 16 | channel(c.b) << 8 | channel(c.a);
    (latex.to_string(), rgba, font_size.to_bits())
}

impl EquationCache {
    /// The entry for `latex` in `color` at `font_size`, rendered on a miss
    /// and marked used this frame.
    fn entry(&mut self, latex: &str, color: Hsla, font_size: f32) -> &mut Entry {
        let frame = self.frame;
        let entry = self
            .entries
            .entry(key(latex, color, font_size))
            .or_insert_with(|| Entry {
                render: render_equation(latex, color, font_size),
                rasters: HashMap::new(),
                used: frame,
            });
        entry.used = frame;
        entry
    }

    /// The render of `latex`, on a miss rendering it.
    pub fn get(
        &mut self,
        latex: &str,
        color: Hsla,
        font_size: f32,
    ) -> &Result<RenderedEquation, String> {
        &self.entry(latex, color, font_size).render
    }

    /// The render of `latex` rasterised at `scale` (chosen as for SVG
    /// pictures, so it stays sharp when zoomed), on a miss rasterising it;
    /// `None` when it does not render.
    pub fn image(
        &mut self,
        latex: &str,
        color: Hsla,
        font_size: f32,
        scale: u32,
        cx: &App,
    ) -> Option<Arc<RenderImage>> {
        let entry = self.entry(latex, color, font_size);
        let svg = entry.render.as_ref().ok()?.svg.clone();
        entry
            .rasters
            .entry(scale)
            .or_insert_with(|| rasterise(&svg, scale as f32, cx))
            .clone()
    }

    /// Keeps the render of `latex` for another frame without painting it.
    pub fn keep(&mut self, latex: &str, color: Hsla, font_size: f32) {
        let frame = self.frame;
        if let Some(entry) = self.entries.get_mut(&key(latex, color, font_size)) {
            entry.used = frame;
        }
    }

    /// Starts a frame; renders not used (or kept) before
    /// [`Self::end_frame`] are released.
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// Releases the renders not used this frame, and their textures.
    pub fn end_frame(&mut self, window: &mut Window) {
        for image in self.release_unused() {
            window.drop_image(image).ok();
        }
    }

    /// Removes the renders not used this frame; returns their rasters.
    fn release_unused(&mut self) -> Vec<Arc<RenderImage>> {
        let frame = self.frame;
        let mut released = Vec::new();
        self.entries.retain(|_, entry| {
            let keep = entry.used == frame;
            if !keep {
                released.extend(entry.rasters.drain().filter_map(|(_, image)| image));
            }
            keep
        });
        released
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::hsla;
    use mindweave_core::{HAlign, RunStyle};

    fn black() -> Hsla {
        hsla(0., 0., 0., 1.)
    }

    #[test]
    fn renders_maths_and_chemistry() {
        for latex in [r"\frac{a}{b}", r"\ce{2H2 + O2 -> 2H2O}"] {
            let eq = render_equation(latex, black(), 28.).unwrap();
            assert!(eq.size.0 > 0. && eq.size.1 > 0., "{latex}");
            assert!(!String::from_utf8_lossy(&eq.svg).contains("<text"));
        }
    }

    #[test]
    fn a_bad_font_size_falls_back_to_the_default() {
        let size = |f: f32| render_equation("x", black(), f).unwrap().size;
        assert_eq!(size(f32::NAN), size(DEFAULT_FONT_SIZE));
        assert_eq!(size(0.), size(DEFAULT_FONT_SIZE));
    }

    #[test]
    fn bad_latex_is_an_error() {
        assert!(render_equation(r"\frac{a", black(), 28.).is_err());
    }

    #[test]
    fn size_follows_font_size() {
        let small = render_equation(r"\frac{a}{b}+c", black(), 14.).unwrap();
        let big = render_equation(r"\frac{a}{b}+c", black(), 28.).unwrap();
        let ratio = big.size.0 / small.size.0;
        assert!((1.8..2.2).contains(&ratio), "{ratio}");
    }

    #[test]
    fn huge_input_is_refused_or_clamped() {
        let long = "x".repeat(MAX_EQUATION_CHARS + 1);
        assert_eq!(
            render_equation(&long, black(), 28.).err().as_deref(),
            Some("Equation is too long")
        );
        let tall = render_equation(&"x\\\\".repeat(300), black(), 28.).unwrap();
        assert!(tall.size.0.max(tall.size.1) <= MAX_SIDE + 1.);
    }

    #[test]
    fn colour_is_part_of_the_cache_key() {
        let mut cache = EquationCache::default();
        let red = hsla(0., 1., 0.5, 1.);
        let a = cache.get("x^2", black(), 28.).as_ref().unwrap().svg.clone();
        let b = cache.get("x^2", red, 28.).as_ref().unwrap().svg.clone();
        let again = cache.get("x^2", black(), 28.).as_ref().unwrap().svg.clone();
        assert_ne!(a, b);
        assert!(Arc::ptr_eq(&a, &again));
    }

    #[test]
    fn unused_renders_are_released_at_frame_end() {
        let mut cache = EquationCache::default();
        cache.begin_frame();
        cache.get("a", black(), 28.);
        cache.get("b", black(), 28.);
        cache.release_unused();
        assert_eq!(cache.entries.len(), 2);
        cache.begin_frame();
        cache.get("a", black(), 28.);
        cache.release_unused();
        assert_eq!(cache.entries.len(), 1);
        cache.begin_frame();
        cache.release_unused();
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn kept_renders_survive_a_frame() {
        let mut cache = EquationCache::default();
        cache.begin_frame();
        cache.get("a", black(), 28.);
        cache.release_unused();
        cache.begin_frame();
        cache.keep("a", black(), 28.);
        cache.release_unused();
        assert_eq!(cache.entries.len(), 1);
    }

    #[test]
    fn font_size_is_the_largest_run_or_the_default() {
        assert_eq!(equation_font_size(None), DEFAULT_FONT_SIZE);
        let style = RunStyle {
            size: 40.,
            ..RunStyle::default()
        };
        let text = RichText::new(style.clone(), HAlign::default());
        assert_eq!(equation_font_size(Some(&text)), 40.);
        let mut text = RichText::new(RunStyle::default(), HAlign::default());
        text.paragraphs[0].runs.push(mindweave_core::Run {
            text: "ab".into(),
            style,
        });
        assert_eq!(equation_font_size(Some(&text)), 40.);
    }
}
