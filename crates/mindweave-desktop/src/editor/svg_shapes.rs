//! Import SVG as Shapes: reads SVG files off the main thread and places
//! each as one group of editable shapes, fitted to the view.

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use mindweave_core::geometry::Rect;
use mindweave_core::{
    import_svg, place_import, Shape, SvgImport, SvgImportError, DUPLICATE_OFFSET, MAX_IMPORT_BYTES,
};

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

/// The warning for `skipped` elements left out, if any.
fn skipped_notice(skipped: usize) -> Option<String> {
    match skipped {
        0 => None,
        1 => Some("1 element (a picture) could not be imported as a shape.".into()),
        n => Some(format!(
            "{n} elements (pictures) could not be imported as shapes."
        )),
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
    use super::{place_files, skipped_notice};
    use mindweave_core::geometry::Rect;
    use mindweave_core::{import_svg, DUPLICATE_OFFSET};

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
        assert!(skipped_notice(1).unwrap().starts_with("1 element "));
        assert!(skipped_notice(3).unwrap().starts_with("3 elements "));
    }
}
