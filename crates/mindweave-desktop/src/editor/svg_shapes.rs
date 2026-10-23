//! Import SVG as Shapes: reads SVG files off the main thread and places
//! each as one group of editable shapes, fitted to the view. Convert to
//! Editable Shapes: replaces selected SVG pictures with their shapes.

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use mindweave_core::geometry::Rect;
use mindweave_core::{
    convert_picture, import_svg, is_svg_picture, place_import, Shape, SvgImport, SvgImportError,
    DUPLICATE_OFFSET, MAX_IMPORT_BYTES,
};
use uuid::Uuid;

use super::EditorWindow;

/// Reads the SVG file at `path`, refusing one too large before reading it.
fn read_svg(path: &std::path::Path) -> Result<SvgImport, SvgImportError> {
    let len = std::fs::metadata(path).map_or(0, |m| m.len());
    if len > MAX_IMPORT_BYTES as u64 {
        return Err(SvgImportError::TooLarge);
    }
    let bytes = std::fs::read(path).map_err(|e| SvgImportError::Invalid(e.to_string()))?;
    import_svg(&bytes)
}

/// The shapes of `imports` placed in `view` (see [`place_import`]), each
/// file a group of its own, stepped down-right like placed pictures.
fn place_files(imports: Vec<SvgImport>, view: &Rect) -> Vec<Shape> {
    let mut out = Vec::new();
    for (i, import) in imports.into_iter().enumerate() {
        let step = DUPLICATE_OFFSET * i as f32;
        let mut shapes = place_import(import.shapes, view);
        for s in &mut shapes {
            s.frame.x += step;
            s.frame.y += step;
        }
        out.extend(shapes);
    }
    out
}

/// The warning for `skipped` elements left out (pictures, text no font
/// could show), if any.
fn skipped_notice(skipped: usize) -> Option<String> {
    match skipped {
        0 => None,
        1 => Some("1 element could not be imported.".into()),
        n => Some(format!("{n} elements could not be imported.")),
    }
}

/// What converting SVG pictures gives.
#[derive(Default)]
struct Conversion {
    /// Each picture converted, as it was, with the shapes replacing it.
    converted: Vec<(Shape, Vec<Shape>)>,
    /// Why each picture that could not be converted was refused.
    failures: Vec<String>,
    /// Elements left out of the pictures converted.
    skipped: usize,
}

/// The editable shapes of the SVG pictures among `shapes` (see
/// [`convert_picture`]); other shapes are passed over.
fn convert_pictures(shapes: Vec<Shape>) -> Conversion {
    let mut out = Conversion::default();
    for picture in shapes {
        match convert_picture(&picture) {
            None => {}
            Some(Ok(import)) if import.shapes.is_empty() => out
                .failures
                .push("That SVG has no shapes or text to import.".into()),
            Some(Ok(import)) => {
                out.skipped += import.skipped;
                out.converted.push((picture, import.shapes));
            }
            Some(Err(e)) => out.failures.push(e.to_string()),
        }
    }
    out
}

/// The one notice for the pictures that could not be converted, why
/// each was refused in `failures`, if any.
fn failure_notice(failures: &[String]) -> Option<String> {
    match failures {
        [] => None,
        [one] => Some(format!("The picture could not be converted. {one}")),
        many => Some(format!("{} pictures could not be converted.", many.len())),
    }
}

impl EditorWindow {
    /// Import SVG as Shapes: asks for SVG files and places their shapes.
    pub(super) fn import_svg_shapes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = rfd::AsyncFileDialog::new()
            .set_title("Import SVG as Shapes")
            .add_filter("SVG", &["svg"])
            .pick_files();
        cx.spawn_in(window, async move |this, cx| {
            let Some(files) = picked.await else {
                return;
            };
            let paths: Vec<_> = files.iter().map(|f| f.path().to_path_buf()).collect();
            let results = cx
                .background_spawn(async move { paths.iter().map(|p| read_svg(p)).collect() })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.place_svg_imports(results, window, cx)
            })
            .ok();
        })
        .detach();
    }

    /// Whether the selection holds an SVG picture to convert.
    pub(super) fn svg_picture_selected(&self) -> bool {
        self.shapes_among(&self.selection)
            .into_iter()
            .any(is_svg_picture)
    }

    /// Convert to Editable Shapes: converts the selected SVG pictures off
    /// the main thread, then replaces each with its shapes.
    pub(super) fn convert_svg_pictures(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_edit_action(window, cx);
        let pictures: Vec<Shape> = self
            .shapes_among(&self.selection)
            .into_iter()
            .filter(|s| is_svg_picture(s))
            .cloned()
            .collect();
        if pictures.is_empty() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let conversion = cx
                .background_spawn(async move { convert_pictures(pictures) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.replace_svg_pictures(conversion, window, cx)
            })
            .ok();
        })
        .detach();
    }

    /// Replaces each picture of `conversion` still as it was with its
    /// shapes, all as one undo step, and selects them; reports the
    /// pictures refused and the elements left out.
    fn replace_svg_pictures(
        &mut self,
        conversion: Conversion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(note) = failure_notice(&conversion.failures) {
            window.push_notification(Notification::error(note), cx);
        }
        if let Some(note) = skipped_notice(conversion.skipped) {
            window.push_notification(Notification::warning(note), cx);
        }
        // A picture edited meanwhile keeps its edit.
        let replacements: Vec<(Uuid, Vec<Shape>)> = conversion
            .converted
            .into_iter()
            .filter(|(p, _)| self.session.find_shape(p.id) == Some(p))
            .map(|(p, parts)| (p.id, parts))
            .collect();
        if replacements.is_empty() {
            return;
        }
        let selection: Vec<Uuid> = replacements
            .iter()
            .flat_map(|(_, parts)| parts.iter().map(|s| s.id))
            .collect();
        self.update_session(|s| s.replace_shapes(replacements), cx);
        self.selection = selection;
        cx.notify();
    }

    /// Places the files read successfully among `results` as one undo
    /// step and selects them; reports the files refused and the elements
    /// left out.
    fn place_svg_imports(
        &mut self,
        results: Vec<Result<SvgImport, SvgImportError>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut imports = Vec::new();
        let mut skipped = 0;
        for result in results {
            match result {
                Ok(import) if import.shapes.is_empty() => {
                    skipped += import.skipped;
                    window.push_notification(
                        Notification::warning("That SVG has no shapes or text to import."),
                        cx,
                    );
                }
                Ok(import) => {
                    skipped += import.skipped;
                    imports.push(import);
                }
                Err(e) => window.push_notification(Notification::error(e.to_string()), cx),
            }
        }
        if let Some(note) = skipped_notice(skipped) {
            window.push_notification(Notification::warning(note), cx);
        }
        let shapes = place_files(imports, &self.visible_canvas());
        if !shapes.is_empty() {
            self.add_and_select(shapes, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{convert_pictures, failure_notice, place_files, skipped_notice};
    use mindweave_core::geometry::{Frame, Rect};
    use mindweave_core::{import_svg, ImageData, ImageFormat, Shape, DUPLICATE_OFFSET};
    use std::sync::Arc;

    fn picture(bytes: &[u8], format: ImageFormat) -> Shape {
        let data = ImageData {
            format,
            id: mindweave_core::image_id(bytes),
            bytes: Arc::new(bytes.to_vec()),
        };
        let frame = Frame {
            w: 20.,
            h: 20.,
            ..Frame::default()
        };
        Shape::new_image(data, frame)
    }

    #[test]
    fn svg_pictures_convert_and_others_are_left_or_reported() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <rect width="5" height="5"/><image width="1" height="1" href="a.png"/></svg>"#;
        let good = picture(svg, ImageFormat::Svg);
        let png = picture(b"\x89PNG\r\n\x1a\n", ImageFormat::Png);
        let broken = picture(b"<svg <<", ImageFormat::Svg);
        let empty = picture(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#,
            ImageFormat::Svg,
        );
        let out = convert_pictures(vec![good.clone(), png, broken, empty]);
        assert_eq!(out.converted.len(), 1);
        let (original, parts) = &out.converted[0];
        assert_eq!(original, &good);
        assert_eq!(parts.len(), 1);
        assert!((parts[0].frame.w - 10.).abs() < 0.01);
        assert_eq!(out.skipped, 1);
        assert_eq!(out.failures.len(), 2);
    }

    #[test]
    fn failures_make_one_notice() {
        assert_eq!(failure_notice(&[]), None);
        let one = vec!["The file is not an SVG Mindweave can read: x".to_string()];
        assert_eq!(
            failure_notice(&one).as_deref(),
            Some(
                "The picture could not be converted. The file is not an SVG Mindweave can read: x"
            )
        );
        let two = vec![one[0].clone(), one[0].clone()];
        assert_eq!(
            failure_notice(&two).as_deref(),
            Some("2 pictures could not be converted.")
        );
    }

    #[test]
    fn each_file_is_its_own_group_stepped_down_right() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <rect width="10" height="10"/><rect x="10" width="10" height="10"/></svg>"#;
        let view = Rect {
            x: 0.,
            y: 0.,
            w: 400.,
            h: 400.,
        };
        let a = import_svg(svg).unwrap();
        let b = import_svg(svg).unwrap();
        let shapes = place_files(vec![a, b], &view);
        assert_eq!(shapes.len(), 4);
        assert_eq!(shapes[0].groups, shapes[1].groups);
        assert_ne!(shapes[0].groups, shapes[2].groups);
        // Centred on the view: 20 × 10 round (200, 200).
        assert_eq!((shapes[0].frame.x, shapes[0].frame.y), (190., 195.));
        assert_eq!(
            (shapes[2].frame.x, shapes[2].frame.y),
            (190. + DUPLICATE_OFFSET, 195. + DUPLICATE_OFFSET)
        );
    }

    #[test]
    fn left_out_elements_are_counted_in_the_notice() {
        assert_eq!(skipped_notice(0), None);
        assert_eq!(
            skipped_notice(1).as_deref(),
            Some("1 element could not be imported.")
        );
        assert_eq!(
            skipped_notice(3).as_deref(),
            Some("3 elements could not be imported.")
        );
    }
}
