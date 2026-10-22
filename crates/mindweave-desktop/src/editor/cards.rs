//! Placement of the cards that open beside a topic (the equation and
//! note cards).

use mindweave_core::geometry::{Frame, Rect};
use mindweave_core::{DocumentSession, Viewport};
use uuid::Uuid;

/// The least space kept between a card and the canvas's edges.
pub(super) const MARGIN: f32 = 8.;

/// Where a card `card` (width, height) big opens beside `topic`, both in
/// screen coordinates relative to the canvas, which is `window` big: right
/// of the topic, `gap` away, when it fits; else left of it; else clamped
/// inside the canvas. Its top is level with the topic's, kept inside the
/// canvas.
pub(super) fn card_origin(
    topic: Rect,
    card: (f32, f32),
    window: (f32, f32),
    gap: f32,
) -> (f32, f32) {
    let max_x = window.0 - card.0 - MARGIN;
    let right = topic.x + topic.w + gap;
    let left = topic.x - gap - card.0;
    let x = if right <= max_x {
        right
    } else if left >= MARGIN {
        left
    } else {
        right.min(max_x).max(MARGIN)
    };
    let y = topic.y.min(window.1 - card.1 - MARGIN).max(MARGIN);
    (x, y)
}

/// The screen rectangle (pixels from the canvas's top left) of a topic
/// with `frame`, seen through `viewport`.
pub(super) fn topic_on_screen(frame: &Frame, viewport: &Viewport) -> Rect {
    let (x, y) = viewport.canvas_to_screen((frame.x, frame.y));
    Rect {
        x,
        y,
        w: frame.w * viewport.zoom,
        h: frame.h * viewport.zoom,
    }
}

/// Whether topic `topic`, which a card is open on, is still on the active
/// page's active layer; the card closes once it is not.
pub(super) fn card_topic_present(session: &DocumentSession, topic: Uuid) -> bool {
    session.find_shape(topic).is_some_and(|s| s.topic.is_some())
}

#[cfg(test)]
mod tests {
    use super::{card_origin, card_topic_present};
    use mindweave_core::geometry::Rect;
    use mindweave_core::DocumentSession;

    fn topic(x: f32, y: f32) -> Rect {
        Rect {
            x,
            y,
            w: 100.,
            h: 40.,
        }
    }

    #[test]
    fn a_card_opens_right_of_its_topic_when_it_fits() {
        assert_eq!(
            card_origin(topic(50., 60.), (200., 150.), (800., 600.), 12.),
            (162., 60.)
        );
    }

    #[test]
    fn a_card_flips_left_near_the_right_edge() {
        assert_eq!(
            card_origin(topic(600., 60.), (200., 150.), (800., 600.), 12.),
            (388., 60.)
        );
    }

    #[test]
    fn a_card_is_clamped_inside_a_small_window() {
        // Neither side fits: it keeps an 8 px margin from the right and
        // the bottom edges.
        assert_eq!(
            card_origin(topic(100., 250.), (200., 150.), (300., 300.), 12.),
            (92., 142.)
        );
        // A window smaller than the card: the top left margin wins.
        assert_eq!(
            card_origin(topic(10., 10.), (200., 150.), (100., 100.), 12.),
            (8., 8.)
        );
        // Above the canvas, the topic's top is clamped down.
        assert_eq!(
            card_origin(topic(50., -80.), (200., 150.), (800., 600.), 12.),
            (162., 8.)
        );
    }

    #[test]
    fn card_closes_when_its_topic_goes() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = DocumentSession::create(dir.path().join("e.mindweave")).unwrap();
        let id = s.add_central_topic((0., 0.));
        assert!(card_topic_present(&s, id));
        s.delete_with_branches(&[id]);
        assert!(!card_topic_present(&s, id));
        assert!(s.undo());
        assert!(card_topic_present(&s, id));
        // Another page does not show it either.
        s.add_page();
        assert!(!card_topic_present(&s, id));
    }
}
