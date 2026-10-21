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

/// Pictures decoded for painting, by [`ImageData::key`]; `None` when the
/// bytes do not decode.
#[derive(Default)]
pub struct ImageCache {
    entries: HashMap<u64, (Option<Arc<RenderImage>>, u64)>,
    frame: u64,
}

impl ImageCache {
    /// The decoded picture of `data`, decoding it on a miss.
    pub fn get(&mut self, data: &ImageData, cx: &App) -> Option<Arc<RenderImage>> {
        let frame = self.frame;
        let entry = self
            .entries
            .entry(data.key())
            .or_insert_with(|| (decode(data, cx), frame));
        entry.1 = frame;
        entry.0.clone()
    }

    /// Starts a frame; pictures not used before [`Self::end_frame`] are
    /// released.
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// Releases the pictures not painted this frame, and their textures.
    pub fn end_frame(&mut self, window: &mut Window) {
        let frame = self.frame;
        let stale: Vec<u64> = self
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

/// SVG pictures are rasterised this many times their own size, so they
/// stay sharp when zoomed in.
const SVG_SCALE: f32 = 3.;

fn decode(data: &ImageData, cx: &App) -> Option<Arc<RenderImage>> {
    if data.format == ImageFormat::Svg {
        return cx
            .svg_renderer()
            .render_single_frame(&data.bytes, SVG_SCALE)
            .ok();
    }
    let image = gpui_kit::Image::from_bytes(gpui_format(data.format), data.bytes.to_vec());
    image.to_image_data(cx.svg_renderer()).ok()
}

/// The picture's own size: its pixels, or an SVG's declared size.
fn natural_size(data: &ImageData, image: &RenderImage) -> (f32, f32) {
    let size = image.size(0);
    let scale = if data.format == ImageFormat::Svg {
        SVG_SCALE
    } else {
        1.
    };
    (size.width.0 as f32 / scale, size.height.0 as f32 / scale)
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
                .and_then(|data| Some((self.image_cache.get(&data, cx)?, data)));
            let Some((image, data)) = decoded else {
                rejected += 1;
                continue;
            };
            let (w, h) = placed_size(natural_size(&data, &image), fit);
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
    use super::is_picture_path;
    use std::path::Path;

    #[test]
    fn picture_files_are_known_by_extension() {
        assert!(is_picture_path(Path::new("/a/photo.JPG")));
        assert!(is_picture_path(Path::new("logo.svg")));
        assert!(!is_picture_path(Path::new("notes.txt")));
        assert!(!is_picture_path(Path::new("noext")));
    }
}
