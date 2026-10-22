//! The canvas's dot pattern, painted as one tile of dots repeated across
//! the canvas: a picture every few hundred pixels instead of a quad per dot.

use std::sync::Arc;

use gpui_kit::*;
use image::{Frame, RgbaImage};

/// The tile's smallest side, in pixels; it holds whole cells of the
/// pattern, one dot in the middle of each.
const TILE_MIN: f32 = 256.;

/// What the tile was drawn for.
#[derive(Clone, Copy, PartialEq)]
struct Key {
    period: f32,
    dot: f32,
    bgra: [u8; 4],
    scale: f32,
}

/// The tile last drawn, kept while the pattern stays the same (panning
/// only moves it).
#[derive(Default)]
pub(super) struct DotTile(Option<(Key, Arc<RenderImage>)>);

impl DotTile {
    /// Paints `dot`-wide square dots in `color`, `period` apart, with one
    /// centred on `first`, across `bounds`. The caller masks to `bounds`.
    pub(super) fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        first: Point<Pixels>,
        period: f32,
        dot: f32,
        color: Hsla,
        window: &mut Window,
    ) {
        if !(period >= 1. && period.is_finite() && dot > 0.) {
            return;
        }
        let cells = (TILE_MIN / period).ceil().max(1.);
        let side = cells * period;
        let rgba = color.to_rgb();
        let channel = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
        let key = Key {
            period,
            dot,
            bgra: [
                channel(rgba.b),
                channel(rgba.g),
                channel(rgba.r),
                channel(rgba.a),
            ],
            scale: window.scale_factor(),
        };
        if self.0.as_ref().is_none_or(|(drawn, _)| *drawn != key) {
            if let Some((_, old)) = self.0.take() {
                window.drop_image(old).ok();
            }
            let px_side = (side * key.scale).round().max(1.) as u32;
            let bytes = raster(px_side, cells as u32, dot * key.scale, key.bgra);
            let Some(tile) = RgbaImage::from_raw(px_side, px_side, bytes) else {
                return;
            };
            let image = RenderImage::new(vec![Frame::new(tile)]);
            self.0 = Some((key, Arc::new(image)));
        }
        let Some((_, tile)) = &self.0 else {
            return;
        };
        // The tile corner a cell's half before `first`, moved back to
        // cover the top left of the bounds.
        let start = |first: Pixels, origin: Pixels| {
            (f32::from(first - origin) - period / 2.).rem_euclid(side) - side
        };
        let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let mut y = start(first.y, bounds.origin.y);
        while y < height {
            let mut x = start(first.x, bounds.origin.x);
            while x < width {
                let at = Bounds::new(
                    bounds.origin + point(px(x), px(y)),
                    size(px(side), px(side)),
                );
                window
                    .paint_image(bounds, at, Corners::default(), tile.clone(), 0, false)
                    .ok();
                x += side;
            }
            y += side;
        }
    }
}

/// A `side`-pixel square BGRA tile of `cells` × `cells` cells, each with a
/// `dot`-pixel square in `bgra` at its centre; partly covered pixels get
/// part of its alpha.
fn raster(side: u32, cells: u32, dot: f32, bgra: [u8; 4]) -> Vec<u8> {
    let mut pixels = vec![0u8; (side * side * 4) as usize];
    let cell = side as f32 / cells as f32;
    let half = dot / 2.;
    // The part of pixel `k` inside `lo..hi`, and the pixels it touches.
    let cover = |k: u32, lo: f32, hi: f32| (hi.min(k as f32 + 1.) - lo.max(k as f32)).clamp(0., 1.);
    let span = |lo: f32, hi: f32| (lo.floor().max(0.) as u32)..(hi.ceil().min(side as f32) as u32);
    for j in 0..cells {
        let cy = (j as f32 + 0.5) * cell;
        let (y0, y1) = (cy - half, cy + half);
        for i in 0..cells {
            let cx = (i as f32 + 0.5) * cell;
            let (x0, x1) = (cx - half, cx + half);
            for y in span(y0, y1) {
                let cover_y = cover(y, y0, y1);
                for x in span(x0, x1) {
                    let alpha = bgra[3] as f32 * cover(x, x0, x1) * cover_y;
                    let at = ((y * side + x) * 4) as usize;
                    pixels[at..at + 4].copy_from_slice(&[
                        bgra[0],
                        bgra[1],
                        bgra[2],
                        alpha.round() as u8,
                    ]);
                }
            }
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::raster;

    fn alpha(pixels: &[u8], side: u32, x: u32, y: u32) -> u8 {
        pixels[((y * side + x) * 4 + 3) as usize]
    }

    #[test]
    fn each_cell_has_a_dot_at_its_centre() {
        // 2 × 2 cells of 10 pixels, 2-pixel dots: pixels 4 and 5 of each.
        let side = 20;
        let pixels = raster(side, 2, 2., [1, 2, 3, 200]);
        for (x, y) in [(4, 4), (5, 5), (14, 4), (4, 15), (15, 15)] {
            assert_eq!(alpha(&pixels, side, x, y), 200, "({x}, {y})");
        }
        for (x, y) in [(0, 0), (3, 4), (6, 5), (10, 10), (19, 19)] {
            assert_eq!(alpha(&pixels, side, x, y), 0, "({x}, {y})");
        }
        assert_eq!(&pixels[(4 * side + 4) as usize * 4..][..3], &[1, 2, 3]);
        let painted = pixels.chunks(4).filter(|p| p[3] > 0).count();
        assert_eq!(painted, 4 * 4);
    }

    #[test]
    fn dots_off_the_pixel_grid_are_shaded_by_coverage() {
        // One 4-pixel cell: the 1-pixel dot spans 1.5..2.5, a quarter of
        // each of pixels (1, 1), (2, 1), (1, 2) and (2, 2).
        let pixels = raster(4, 1, 1., [0, 0, 0, 255]);
        for (x, y) in [(1, 1), (2, 1), (1, 2), (2, 2)] {
            assert_eq!(alpha(&pixels, 4, x, y), 64, "({x}, {y})");
        }
        assert_eq!(alpha(&pixels, 4, 0, 2), 0);
        assert_eq!(alpha(&pixels, 4, 3, 2), 0);
    }
}
