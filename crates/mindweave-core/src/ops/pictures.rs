//! Pictures: images placed on a page, and the picture (an icon or an
//! image) a topic shows above its text. The caller reads the picture and
//! measures it; these commands place it.

use uuid::Uuid;

use super::diagrams::check_size_arg;
use super::{check_number, OpError, OpResult, Workspace};
use crate::geometry::Frame;
use crate::image::{placed_size, ImageData};
use crate::mindmap::{clamp_topic_picture, fit_topic_picture};
use crate::shape::Shape;

/// A picture to place: its data and its own size (pixels, or an SVG's
/// size in its units).
pub struct Picture {
    pub data: ImageData,
    pub size: (f32, f32),
}

impl Workspace {
    /// Places `picture` on page `page` (the active one when `None`) with
    /// its top-left corner at `at`. Given one of `w` and `h`, the other
    /// keeps the picture's proportions; given neither, its longest side is
    /// scaled to `longest` when given, else it keeps its own size up to
    /// 600 units.
    /// Returns its id; inside an artboard, it joins it.
    pub fn add_image(
        &mut self,
        page: Option<usize>,
        picture: Picture,
        at: (f32, f32),
        (w, h): (Option<f32>, Option<f32>),
        longest: Option<f32>,
    ) -> OpResult<Uuid> {
        let page = self.page_index(page)?;
        let (x, y) = (check_number("x", at.0)?, check_number("y", at.1)?);
        let (pw, ph) = picture.size;
        let (pw, ph) = if pw > 0.0 && ph > 0.0 {
            (pw, ph)
        } else {
            (100.0, 100.0)
        };
        let (w, h) = match (w, h) {
            (Some(w), Some(h)) => (check_size_arg(w)?, check_size_arg(h)?),
            (Some(w), None) => {
                let w = check_size_arg(w)?;
                (w, w * ph / pw)
            }
            (None, Some(h)) => {
                let h = check_size_arg(h)?;
                (h * pw / ph, h)
            }
            (None, None) => match longest {
                // Scaled up or down, as the Icons pane places icons.
                Some(l) => {
                    let k = check_size_arg(l)? / pw.max(ph);
                    (pw * k, ph * k)
                }
                None => placed_size((pw, ph), (f32::MAX, f32::MAX)),
            },
        };
        let shape = Shape::new_image(
            picture.data,
            Frame {
                x,
                y,
                w: w.max(1.0),
                h: h.max(1.0),
                ..Frame::default()
            },
        );
        let id = shape.id;
        self.on_layer(page, None, |s| super::add_with_membership(s, vec![shape]));
        Ok(id)
    }

    /// Shows `picture` in topic `id` above its text, its longest side
    /// `longest` (its own size at most, clamped to 12 to 800 units), or
    /// with `None` removes the topic's picture. The topic grows to fit.
    pub fn set_topic_picture(
        &mut self,
        id: Uuid,
        picture: Option<Picture>,
        longest: f32,
    ) -> OpResult<()> {
        let (page, layer) = self.topic_at(id, "set_topic_picture")?;
        if !longest.is_finite() || longest <= 0.0 {
            return Err(OpError::invalid(
                "The picture size must be a positive number.",
            ));
        }
        let shown = picture
            .as_ref()
            .map(|p| clamp_topic_picture(fit_topic_picture(p.size, longest)));
        let data = picture.map(|p| p.data);
        self.on_layer(page, Some(layer), |s| {
            s.update_shapes(&[id], |shape| {
                if let Some(t) = shape.topic.as_mut() {
                    t.picture = data.clone();
                    t.picture_size = shown;
                }
            })
        });
        self.fit(page, &[id]);
        Ok(())
    }
}
