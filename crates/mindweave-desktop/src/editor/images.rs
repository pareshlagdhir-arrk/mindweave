//! Images on the canvas: decoding pictures for painting (cached by their
//! bytes), the Image button's file dialog, and pasting pictures or
//! picture files from the clipboard.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use mindweave_core::geometry::Frame;
use mindweave_core::{placed_size, ImageData, ImageFormat, Shape, DUPLICATE_OFFSET};

use super::EditorWindow;

/// Pictures decoded for painting, by [`ImageData::key`] and raster
/// scale; `None` when the bytes do not decode. SVG pictures are
/// rasterised at the scale they are shown at (in steps of two), so icons
/// stay sharp at any size and zoom; other pictures at their own size.
#[derive(Default)]
pub struct ImageCache {
    entries: HashMap<(u64, u32), Decoded>,
    /// Each picture's own size, once known.
    natural: HashMap<u64, (f32, f32)>,
    /// Each picture's visible tone (see [`visible_luminance`]), once known.
    luminance: HashMap<u64, Option<f32>>,
    frame: u64,
}

/// A decoded picture (`None`: it does not decode) and the frame it was
/// last used in.
type Decoded = (Option<Arc<RenderImage>>, u64);

/// The longest side an SVG is rasterised to, in pixels.
const MAX_SVG_RASTER: f32 = 4096.;
/// The largest SVG raster scale.
const MAX_SVG_SCALE: u32 = 64;

/// The raster scale for an SVG whose longest side is `natural` units,
/// shown `shown` device pixels long: the next power of two at or above
/// the ratio, at least 1, kept within [`MAX_SVG_RASTER`].
pub fn svg_scale(natural: f32, shown: f32) -> u32 {
    if !(natural > 0. && shown > 0.) {
        return 1;
    }
    let want = (shown / natural).max(1.);
    let mut scale = (want.log2().ceil().exp2() as u32).clamp(1, MAX_SVG_SCALE);
    while scale > 1 && natural * scale as f32 > MAX_SVG_RASTER {
        scale /= 2;
    }
    scale
}

impl ImageCache {
    /// The picture of `data` decoded at raster `scale` (SVG only), on a
    /// miss decoding it.
    fn get_scaled(&mut self, data: &ImageData, scale: u32, cx: &App) -> Option<Arc<RenderImage>> {
        let frame = self.frame;
        let key = data.key();
        let entry = self
            .entries
            .entry((key, scale))
            .or_insert_with(|| (decode(data, scale as f32, cx), frame));
        entry.1 = frame;
        let image = entry.0.clone()?;
        self.natural.entry(key).or_insert_with(|| {
            let size = image.size(0);
            (
                size.width.0 as f32 / scale as f32,
                size.height.0 as f32 / scale as f32,
            )
        });
        Some(image)
    }

    /// How light the picture's visible pixels are, 0 to 1; `None` when it
    /// has none or does not decode.
    pub fn luminance(&mut self, data: &ImageData, cx: &App) -> Option<f32> {
        if let Some(l) = self.luminance.get(&data.key()) {
            return *l;
        }
        let image = self.get_scaled(data, 1, cx);
        let l = image.and_then(|i| i.as_bytes(0).and_then(visible_luminance));
        self.luminance.insert(data.key(), l);
        l
    }

    /// The picture's own size: its pixels, or an SVG's declared size.
    pub fn natural_size(&mut self, data: &ImageData, cx: &App) -> Option<(f32, f32)> {
        if let Some(size) = self.natural.get(&data.key()) {
            return Some(*size);
        }
        self.get_scaled(data, 1, cx)?;
        self.natural.get(&data.key()).copied()
    }

    /// The picture of `data` for showing `shown` device pixels long (its
    /// longest side).
    pub fn get(&mut self, data: &ImageData, shown: f32, cx: &App) -> Option<Arc<RenderImage>> {
        let scale = if data.format == ImageFormat::Svg {
            let (w, h) = self.natural_size(data, cx)?;
            svg_scale(w.max(h), shown)
        } else {
            1
        };
        self.get_scaled(data, scale, cx)
    }

    /// Keeps the pictures of `data` for another frame without painting
    /// them, so one scrolled out of sight is not decoded again, with a
    /// stall, when it comes back.
    pub fn keep(&mut self, data: &ImageData) {
        let (key, frame) = (data.key(), self.frame);
        for ((entry_key, _), (_, used)) in &mut self.entries {
            if *entry_key == key {
                *used = frame;
            }
        }
    }

    /// Starts a frame; pictures not used (or kept) before
    /// [`Self::end_frame`] are released.
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// Releases the pictures not painted this frame, and their textures.
    pub fn end_frame(&mut self, window: &mut Window) {
        let frame = self.frame;
        let stale: Vec<(u64, u32)> = self
            .entries
            .iter()
            .filter(|(_, (_, used))| *used != frame)
            .map(|(k, _)| *k)
            .collect();
        for key in stale {
            if let Some((Some(image), _)) = self.entries.remove(&key) {
                window.drop_image(image).ok();
            }
        }
    }
}

/// Most pixels looked at to judge a picture's tone.
const TONE_SAMPLES: usize = 4096;
/// Below this contrast with what is behind it, a picture gets a plate.
const MIN_CONTRAST: f32 = 1.6;

/// The mean relative luminance (0 to 1) of the visible pixels of a BGRA
/// picture, weighted by their opacity; `None` when none is visible.
pub fn visible_luminance(bgra: &[u8]) -> Option<f32> {
    let pixels = bgra.len() / 4;
    let step = (pixels / TONE_SAMPLES).max(1);
    let (mut sum, mut weight) = (0., 0.);
    for px in bgra.chunks_exact(4).step_by(step) {
        let alpha = px[3] as f32 / 255.;
        if alpha > 0.05 {
            sum += mindweave_core::relative_luminance((px[2], px[1], px[0])) * alpha;
            weight += alpha;
        }
    }
    (weight > 0.).then(|| sum / weight)
}

/// The plate to draw behind a picture of tone `picture` shown on a
/// background of tone `background` (luminances), when the two are too
/// alike to tell apart: light on dark backgrounds, dark on light ones.
pub fn contrast_plate(picture: f32, background: f32) -> Option<Hsla> {
    let ratio = (picture.max(background) + 0.05) / (picture.min(background) + 0.05);
    if ratio >= MIN_CONTRAST {
        return None;
    }
    Some(if background < 0.4 {
        hsla(0., 0., 0.95, 1.)
    } else {
        hsla(0., 0., 0.15, 1.)
    })
}

/// The luminance of `color`.
pub fn hsla_luminance(color: Hsla) -> f32 {
    let c = Rgba::from(color);
    let channel = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    mindweave_core::relative_luminance((channel(c.r), channel(c.g), channel(c.b)))
}

/// The tone of the picture in file `path`, for the Icons pane.
pub fn file_luminance(path: &Path, cx: &App) -> Option<f32> {
    let data = ImageData::from_file_bytes(std::fs::read(path).ok()?)?;
    let image = decode(&data, 1., cx)?;
    visible_luminance(image.as_bytes(0)?)
}

/// gpui's name for `format`.
fn gpui_format(format: ImageFormat) -> gpui_kit::ImageFormat {
    match format {
        ImageFormat::Png => gpui_kit::ImageFormat::Png,
        ImageFormat::Jpeg => gpui_kit::ImageFormat::Jpeg,
        ImageFormat::Gif => gpui_kit::ImageFormat::Gif,
        ImageFormat::Webp => gpui_kit::ImageFormat::Webp,
        ImageFormat::Bmp => gpui_kit::ImageFormat::Bmp,
        ImageFormat::Tiff => gpui_kit::ImageFormat::Tiff,
        ImageFormat::Svg => gpui_kit::ImageFormat::Svg,
    }
}

fn decode(data: &ImageData, scale: f32, cx: &App) -> Option<Arc<RenderImage>> {
    if data.format == ImageFormat::Svg {
        return cx
            .svg_renderer()
            .render_single_frame(&data.bytes, scale)
            .ok();
    }
    let image = gpui_kit::Image::from_bytes(gpui_format(data.format), data.bytes.to_vec());
    image.to_image_data(cx.svg_renderer()).ok()
}

/// Whether `path` names a picture file the import dialog offers.
fn is_picture_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ImageFormat::EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The pictures among the clipboard's contents: picture files copied in
/// Finder, or a copied picture. `None` when it holds neither, so paste
/// carries on with text.
pub fn clipboard_pictures(item: &ClipboardItem) -> Option<Vec<Vec<u8>>> {
    for entry in item.entries() {
        match entry {
            ClipboardEntry::ExternalPaths(paths) => {
                let files: Vec<Vec<u8>> = paths
                    .paths()
                    .iter()
                    .filter(|p| is_picture_path(p))
                    .filter_map(|p| std::fs::read(p).ok())
                    .collect();
                if !files.is_empty() {
                    return Some(files);
                }
            }
            ClipboardEntry::Image(image) => return Some(vec![image.bytes.clone()]),
            ClipboardEntry::String(_) => {}
        }
    }
    None
}

/// Whether clipboard text `text` is an SVG picture's source, as
/// Chromium-based browsers copy an inline SVG (and as icon sites copy
/// their icons).
pub fn is_svg_markup(text: &str) -> bool {
    let t = text.trim();
    let head = &t[..t.len().min(1024)];
    let starts_svg = head.starts_with("<svg")
        || ((head.starts_with("<?xml")
            || head.starts_with("<!DOCTYPE svg")
            || head.starts_with("<!--"))
            && head.contains("<svg"));
    starts_svg && t.ends_with('>') && t.contains("</svg>")
        || starts_svg && t.ends_with("/>") && !t.contains("</svg>")
}

/// Whether a picture on the clipboard wins over its text `text`: when there
/// is no text, or the text is only a web address, a path or a file name
/// (one line without spaces), as browsers and Preview add to a copied
/// picture. Longer text (a spreadsheet's or a document's, which add a
/// picture of themselves) is pasted as text.
pub fn picture_wins_over(text: Option<&str>) -> bool {
    match text.map(str::trim) {
        None | Some("") => true,
        Some(t) => !t.contains(char::is_whitespace),
    }
}

/// The picture data on the system clipboard, read directly: gpui reports
/// text before image data, so a picture copied with text would otherwise
/// paste as its text. PNG first, then the other formats Mindweave opens.
#[cfg(target_os = "macos")]
pub fn native_clipboard_picture() -> Option<Vec<u8>> {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;
    let pasteboard = NSPasteboard::generalPasteboard();
    [
        "public.png",
        "public.jpeg",
        "com.compuserve.gif",
        "org.webmproject.webp",
        "public.svg-image",
        "public.tiff",
        "com.microsoft.bmp",
    ]
    .into_iter()
    .find_map(|uti| {
        let data = pasteboard.dataForType(&NSString::from_str(uti))?;
        let bytes = data.to_vec();
        (!bytes.is_empty()).then_some(bytes)
    })
}

/// No direct clipboard reading elsewhere yet: gpui's entries only.
#[cfg(not(target_os = "macos"))]
pub fn native_clipboard_picture() -> Option<Vec<u8>> {
    None
}

impl EditorWindow {
    /// The Image button: asks for picture files and places them.
    pub(super) fn import_images(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = rfd::AsyncFileDialog::new()
            .set_title("Import Image")
            .add_filter("Images", &ImageFormat::EXTENSIONS)
            .pick_files();
        cx.spawn_in(window, async move |this, cx| {
            let Some(files) = picked.await else {
                return;
            };
            let bytes: Vec<Vec<u8>> = files
                .iter()
                .filter_map(|f| std::fs::read(f.path()).ok())
                .collect();
            this.update_in(cx, |this, window, cx| {
                this.place_pictures(bytes, window, cx)
            })
            .ok();
        })
        .detach();
    }

    /// Places each of `files` (picture file bytes) at its own size, fitted
    /// to the view, centred and stepped down-right, and selects them as
    /// one undo step. Files that are not pictures, too large or that do
    /// not decode are reported.
    pub(super) fn place_pictures(
        &mut self,
        files: Vec<Vec<u8>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.visible_canvas();
        let centre = (view.x + view.w / 2., view.y + view.h / 2.);
        let fit = (view.w * 0.8, view.h * 0.8);
        let mut shapes = Vec::new();
        let mut rejected = 0;
        for bytes in files {
            let decoded = ImageData::from_file_bytes(bytes)
                .and_then(|data| Some((self.image_cache.natural_size(&data, cx)?, data)));
            let Some((natural, data)) = decoded else {
                rejected += 1;
                continue;
            };
            let (w, h) = placed_size(natural, fit);
            let step = DUPLICATE_OFFSET * shapes.len() as f32;
            let frame = Frame {
                x: centre.0 - w / 2. + step,
                y: centre.1 - h / 2. + step,
                w,
                h,
                ..Frame::default()
            };
            shapes.push(Shape::new_image(data, frame));
        }
        if rejected > 0 {
            let message = if rejected == 1 {
                "That file is not a picture Mindweave can open, or it is larger than 20 MB."
                    .to_string()
            } else {
                format!(
                    "{rejected} files are not pictures Mindweave can open, or are larger than 20 MB."
                )
            };
            window.push_notification(Notification::warning(message), cx);
        }
        if !shapes.is_empty() {
            self.add_and_select(shapes, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_picture_path, is_svg_markup, picture_wins_over, svg_scale};

    use std::path::Path;

    #[test]
    fn pictures_like_their_background_get_a_contrasting_plate() {
        use super::{contrast_plate, visible_luminance};
        // BGRA: two black pixels, one transparent white one.
        let black = [0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 0];
        assert_eq!(visible_luminance(&black), Some(0.));
        assert_eq!(visible_luminance(&[255, 255, 255, 0]), None);
        // Black on a dark canvas: a light plate. Black on white: none.
        let plate = contrast_plate(0., 0.02).unwrap();
        assert!(plate.l > 0.9);
        assert_eq!(contrast_plate(0., 1.), None);
        // White on white: a dark plate.
        assert!(contrast_plate(1., 0.95).unwrap().l < 0.2);
    }

    #[test]
    fn svgs_are_rasterised_at_the_scale_they_are_shown() {
        assert_eq!(svg_scale(24., 24.), 1);
        assert_eq!(svg_scale(24., 10.), 1);
        assert_eq!(svg_scale(24., 48.), 2);
        assert_eq!(svg_scale(24., 49.), 4);
        assert_eq!(svg_scale(48., 3000.), 64);
        // Never past the largest raster.
        assert_eq!(svg_scale(1000., 100_000.), 4);
        assert_eq!(svg_scale(0., 10.), 1);
    }

    #[test]
    fn picture_files_are_known_by_extension() {
        assert!(is_picture_path(Path::new("/a/photo.JPG")));
        assert!(is_picture_path(Path::new("logo.svg")));
        assert!(!is_picture_path(Path::new("notes.txt")));
        assert!(!is_picture_path(Path::new("noext")));
    }

    #[test]
    fn a_picture_wins_over_an_address_but_not_over_text() {
        assert!(picture_wins_over(None));
        assert!(picture_wins_over(Some("  ")));
        assert!(picture_wins_over(Some("https://example.com/logo.png")));
        assert!(picture_wins_over(Some("/Users/me/Pictures/cat.jpg")));
        assert!(picture_wins_over(Some("cat.jpg")));
        // Copied from a spreadsheet or a document, with a picture of it.
        assert!(!picture_wins_over(Some("Q3 revenue")));
        assert!(!picture_wins_over(Some("a\tb\nc\td")));
    }

    #[test]
    fn svg_source_text_is_recognised() {
        let svg = r#"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
 <path d="M3 5h18"/>
</svg>"#;
        assert!(is_svg_markup(svg));
        assert!(is_svg_markup(&format!("  \n{svg}\n")));
        assert!(is_svg_markup(&format!("<?xml version=\"1.0\"?>\n{svg}")));
        assert!(is_svg_markup(
            r#"<svg xmlns="http://www.w3.org/2000/svg"/>"#
        ));
        assert!(!is_svg_markup("Use an <svg> element for icons."));
        assert!(!is_svg_markup("<div><svg></svg></div>"));
        assert!(!is_svg_markup("hello"));
    }
}
