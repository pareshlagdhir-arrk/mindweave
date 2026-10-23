//! Rendering for the server: topic equations with RaTeX (as the app
//! draws them) and SVG pictures rasterised to PNG with resvg.

use std::sync::{Arc, OnceLock};

use mindweave_core::export::EquationSvg;
use ratex_layout::{layout, to_display_list, LayoutOptions};
use ratex_parser::parser::parse;
use ratex_svg::{render_to_svg, SvgOptions};
use ratex_types::color::Color;
use ratex_types::math_style::MathStyle;
use resvg::{tiny_skia, usvg};

/// The most characters of LaTeX rendered (as in the app).
const MAX_EQUATION_CHARS: usize = 1_500;
/// The longest side a rendered equation may have, in canvas units.
const MAX_EQUATION_SIDE: f32 = 800.0;
/// The most pixels a PNG may have along either side.
pub const MAX_PNG_SIDE: u32 = 8192;
/// The most pixels along either side of a PNG returned inline.
pub const MAX_INLINE_SIDE: u32 = 2048;
/// The most pixels a PNG may have in all.
const MAX_PNG_PIXELS: u64 = 32_000_000;

/// `latex` rendered in `color` (`#RRGGBB`) at `font_size`, scaled down
/// to fit the app's largest equation; `None` when it does not parse.
pub fn equation_svg(latex: &str, color: &str, font_size: f32) -> Option<EquationSvg> {
    if latex.chars().count() > MAX_EQUATION_CHARS {
        return None;
    }
    let font_size = if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        mindweave_core::DEFAULT_FONT_SIZE
    };
    let (r, g, b) = mindweave_core::parse_hex_color(color).unwrap_or((0, 0, 0));
    let mut out = render(latex, (r, g, b), font_size)?;
    let longest = out.size.0.max(out.size.1);
    if longest > MAX_EQUATION_SIDE {
        out = render(latex, (r, g, b), font_size * MAX_EQUATION_SIDE / longest)?;
    }
    Some(out)
}

/// One RaTeX render of `latex`.
fn render(latex: &str, (r, g, b): (u8, u8, u8), font_size: f32) -> Option<EquationSvg> {
    let ast = parse(latex).ok()?;
    let c = |v: u8| f32::from(v) / 255.0;
    let options = LayoutOptions::default()
        .with_style(MathStyle::Display)
        .with_color(Color::new(c(r), c(g), c(b), 1.0));
    let list = to_display_list(&layout(&ast, &options));
    let svg = render_to_svg(
        &list,
        &SvgOptions {
            font_size: f64::from(font_size),
            padding: 0.0,
            embed_glyphs: true,
            ..SvgOptions::default()
        },
    );
    let size = svg_size(&svg)?;
    Some(EquationSvg { svg, size })
}

/// The `width` and `height` of the root element of `svg`, unitless.
fn svg_size(svg: &str) -> Option<(f32, f32)> {
    let root = &svg[svg.find("<svg")?..];
    let root = &root[..root.find('>')?];
    let attr = |name: &str| -> Option<f32> {
        let from = root.find(&format!(" {name}=\""))? + name.len() + 3;
        let value = &root[from..];
        value[..value.find('"')?]
            .trim_end_matches("pt")
            .parse()
            .ok()
    };
    let size = (attr("width")?, attr("height")?);
    (size.0 > 0.0 && size.1 > 0.0).then_some(size)
}

/// The size of `latex` rendered at `font_size`, for fitting topics.
pub fn equation_size(latex: &str, font_size: f32) -> Option<(f32, f32)> {
    equation_svg(latex, "#000000", font_size).map(|e| e.size)
}

/// The system fonts, loaded once.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// A rasterised picture.
pub struct Png {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// `svg` rasterised at `scale` pixels per unit, scaled down if needed to
/// stay within `max_side` (at most [`MAX_PNG_SIDE`]) and the pixel budget.
pub fn rasterise(svg: &str, scale: f32, max_side: u32) -> Result<Png, String> {
    let options = usvg::Options {
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    let tree =
        usvg::Tree::from_str(svg, &options).map_err(|e| format!("The SVG did not parse: {e}"))?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    let mut scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let side = max_side.clamp(1, MAX_PNG_SIDE) as f32;
    scale = scale.min(side / w.max(1.0)).min(side / h.max(1.0));
    let pixels = f64::from(w * scale) * f64::from(h * scale);
    if pixels > MAX_PNG_PIXELS as f64 {
        scale *= (MAX_PNG_PIXELS as f64 / pixels).sqrt() as f32;
    }
    let (pw, ph) = (
        ((w * scale).ceil() as u32).max(1),
        ((h * scale).ceil() as u32).max(1),
    );
    let mut pixmap =
        tiny_skia::Pixmap::new(pw, ph).ok_or_else(|| "The picture is too large.".to_string())?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let bytes = pixmap
        .encode_png()
        .map_err(|e| format!("The PNG could not be encoded: {e}"))?;
    Ok(Png {
        bytes,
        width: pw,
        height: ph,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equations_render_with_a_size() {
        let eq = equation_svg("E = mc^2", "#112233", 16.0).unwrap();
        assert!(eq.svg.contains("<svg"));
        assert!(eq.size.0 > eq.size.1 && eq.size.1 > 5.0, "{:?}", eq.size);
        assert!(equation_svg("\\frac{", "#000000", 16.0).is_none());
    }

    #[test]
    fn svgs_rasterise_at_a_scale() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"50\"><rect width=\"100\" height=\"50\" fill=\"#FF0000\"/></svg>";
        let png = rasterise(svg, 2.0, MAX_PNG_SIDE).unwrap();
        assert_eq!((png.width, png.height), (200, 100));
        assert!(png.bytes.starts_with(b"\x89PNG"));
        assert!(rasterise("<nope", 1.0, MAX_PNG_SIDE).is_err());
        let small = rasterise(svg, 100.0, 300).unwrap();
        assert_eq!((small.width, small.height), (300, 150));
    }
}
