//! The Artboard section of the Properties sidebar, shown when every
//! selected shape is an artboard: its name (one artboard selected), the
//! preset giving its size, and whether it clips its members. Each change
//! is one undo step. Size, fill, stroke and corner radius are in the shape
//! panel's other sections; artboards have no rotation.

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::Sizable as _;
use gpui_kit::*;
use mindweave_core::{ArtboardPreset, DocumentSession, ParamDef, Shape, ARTBOARD_PRESETS};
use uuid::Uuid;

use super::artboard_edit::rename_artboard;
use super::properties_panel::field_row;
use super::shape_properties::{common, Value, SECTION_GAP};
use super::toolbar::preset_tooltip;
use super::{section_label, EditorWindow};

/// Width of the name field and the preset dropdown.
const CONTROL_WIDTH: f32 = 140.;
/// What the preset dropdown shows for a size no preset has.
const CUSTOM: &str = "Custom";
/// Sizes this close count as a preset's.
const SIZE_TOLERANCE: f32 = 0.01;

/// The parameters an artboard declares: its corner radius, with the range
/// of a rectangle's.
pub(super) fn artboard_params() -> Vec<ParamDef> {
    vec![ParamDef {
        key: "corner_radius".into(),
        label: "Corner radius".into(),
        min: 0.0,
        max: 1000.0,
        step: 1.0,
        default: 0.0,
    }]
}

/// The preset whose size `board` has: the one it is named after when
/// several share the size (HD screen and Slide 16:9), else the first;
/// `None` for a size of its own.
fn preset_of(board: &Shape) -> Option<&'static ArtboardPreset> {
    let size = (board.frame.w, board.frame.h);
    let fits = |p: &&ArtboardPreset| {
        (p.size.0 - size.0).abs() < SIZE_TOLERANCE && (p.size.1 - size.1).abs() < SIZE_TOLERANCE
    };
    let mut matching = ARTBOARD_PRESETS.iter().filter(fits);
    let first = matching.clone().next();
    matching
        .find(|p| board.name.as_deref() == Some(p.name))
        .or(first)
}

/// What the preset dropdown shows for `boards`: their preset's name,
/// "Custom" for a size of their own, or `None` when they differ.
fn shown_preset(boards: &[&Shape]) -> Option<&'static str> {
    match common(boards.iter().map(|b| preset_of(b).map(|p| p.name)))? {
        Value::Same(name) => Some(name.unwrap_or(CUSTOM)),
        Value::Mixed => None,
    }
}

/// Gives the artboards among `ids` the size `size` as one undo step,
/// their top-left corners staying. Other shapes' membership does not
/// change, as with any artboard resize.
fn resize_artboards(session: &mut DocumentSession, ids: &[Uuid], size: (f32, f32)) {
    session.update_shapes(ids, |s| {
        if s.is_artboard() {
            (s.frame.w, s.frame.h) = size;
        }
    });
}

/// Sets whether the artboards among `ids` clip their members, as one undo
/// step.
fn set_clip(session: &mut DocumentSession, ids: &[Uuid], clip: bool) {
    session.update_shapes(ids, |s| {
        if let Some(props) = s.artboard_props.as_mut() {
            props.clip = clip;
        }
    });
}

/// Whether the name field, showing `shows` (artboard and name), must show
/// `wanted` instead: always for another artboard, even while it is being
/// typed in, but a new name of the same artboard waits until it is not.
fn needs_show(shows: Option<&(Uuid, String)>, wanted: &(Uuid, String), focused: bool) -> bool {
    match shows {
        Some((id, _)) if *id != wanted.0 => true,
        Some(shown) => shown != wanted && !focused,
        None => true,
    }
}

/// The name field, and the artboard and name it shows.
pub struct ArtboardFields {
    pub name: Entity<InputState>,
    shows: Option<(Uuid, String)>,
}

impl ArtboardFields {
    /// The field, and the subscription renaming the artboard it shows on
    /// Enter or blur.
    pub fn new(window: &mut Window, cx: &mut Context<EditorWindow>) -> (Self, Subscription) {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        let subscription = cx.subscribe_in(&name, window, |this, input, event, window, cx| {
            if !matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                return;
            }
            // Leaving the field lets it catch up with a name changed
            // while it was being typed in.
            if matches!(event, InputEvent::Blur) {
                cx.notify();
            }
            let Some((id, shown)) = this.artboard_fields.shows.clone() else {
                return;
            };
            let value = input.read(cx).value().trim().to_string();
            if value == shown {
                return;
            }
            this.update_session(|s| rename_artboard(s, id, &value), cx);
            // The field shows the stored name: trimmed, or the old one
            // when a blank name was refused.
            let stored = this
                .session
                .find_shape(id)
                .and_then(|s| s.name.clone())
                .unwrap_or_default();
            if stored != input.read(cx).value().as_ref() {
                let text = stored.clone();
                input.update(cx, |state, cx| state.set_value(text, window, cx));
            }
            this.artboard_fields.shows = Some((id, stored));
        });
        (ArtboardFields { name, shows: None }, subscription)
    }
}

impl EditorWindow {
    /// The selected shapes when there are some and all are artboards.
    fn selected_artboards(&self) -> Vec<&Shape> {
        let shapes = self.shapes_among(&self.selection);
        if shapes.iter().all(|s| s.is_artboard()) {
            shapes
        } else {
            Vec::new()
        }
    }

    /// Shows the selected artboard's name in the name field when the
    /// selection or the name changed (see [`needs_show`]), so Enter always
    /// renames the artboard the field shows.
    pub(super) fn sync_artboard_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = match self.selected_artboards()[..] {
            [board] => Some((board.id, board.name.clone().unwrap_or_default())),
            _ => None,
        };
        let Some(wanted) = wanted else {
            self.artboard_fields.shows = None;
            return;
        };
        let input = self.artboard_fields.name.clone();
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        if !needs_show(self.artboard_fields.shows.as_ref(), &wanted, focused) {
            return;
        }
        input.update(cx, |state, cx| {
            state.set_value(wanted.1.clone(), window, cx)
        });
        self.artboard_fields.shows = Some(wanted);
    }

    /// Gives the selected artboards `preset`'s size.
    fn apply_preset(&mut self, preset: &ArtboardPreset, cx: &mut Context<Self>) {
        let ids = self.selection.clone();
        self.update_session(|s| resize_artboards(s, &ids, preset.size), cx);
    }

    /// Sets whether the selected artboards clip their members.
    fn set_artboard_clip(&mut self, clip: bool, cx: &mut Context<Self>) {
        let ids = self.selection.clone();
        self.update_session(|s| set_clip(s, &ids, clip), cx);
    }
}

/// The preset dropdown: `current` (or "Mixed"), and every preset, a
/// separator between categories.
fn preset_dropdown(
    current: Option<&'static str>,
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    let this = cx.entity().downgrade();
    Button::new("artboard-preset")
        .outline()
        .small()
        .w(px(CONTROL_WIDTH))
        .label(current.unwrap_or("Mixed"))
        .dropdown_caret(true)
        .dropdown_menu(move |mut menu, _, _| {
            for (ix, preset) in ARTBOARD_PRESETS.iter().enumerate() {
                if ix > 0 && ARTBOARD_PRESETS[ix - 1].category != preset.category {
                    menu = menu.separator();
                }
                let this = this.clone();
                menu = menu.item(
                    PopupMenuItem::new(preset_tooltip(preset))
                        .checked(current == Some(preset.name))
                        .on_click(move |_, _, cx| {
                            this.update(cx, |this, cx| this.apply_preset(preset, cx))
                                .ok();
                        }),
                );
            }
            menu
        })
}

/// The Artboard section, when every selected shape is an artboard.
pub(super) fn render(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> Option<AnyElement> {
    let boards = editor.selected_artboards();
    if boards.is_empty() {
        return None;
    }
    let name_row = (boards.len() == 1).then(|| {
        field_row(
            "Name",
            None,
            div()
                .w(px(CONTROL_WIDTH))
                .child(Input::new(&editor.artboard_fields.name).small()),
        )
    });
    let preset_row = field_row(
        "Preset",
        Some("Gives the artboard a ready-made size; its top-left corner stays"),
        preset_dropdown(shown_preset(&boards), cx),
    );
    let clips = boards
        .iter()
        .all(|b| b.artboard_props.as_ref().is_none_or(|p| p.clip));
    let this = cx.entity().downgrade();
    let clip_row = field_row(
        "Clip content",
        Some("Shows the artboard's shapes only inside its frame"),
        Switch::new("artboard-clip")
            .checked(clips)
            .small()
            .on_click(move |on, _, cx| {
                this.update(cx, |this, cx| this.set_artboard_clip(*on, cx))
                    .ok();
            }),
    );
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(section_label("Artboard", cx).mt(px(SECTION_GAP)))
            .children(name_row)
            .child(preset_row)
            .child(clip_row)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{needs_show, preset_of, resize_artboards, set_clip, shown_preset};
    use mindweave_core::geometry::Frame;
    use mindweave_core::{DocumentSession, Shape};
    use uuid::Uuid;

    fn board(w: f32, h: f32, name: &str) -> Shape {
        let frame = Frame {
            x: 10.,
            y: 20.,
            w,
            h,
            ..Frame::default()
        };
        Shape::new_artboard(frame, name)
    }

    #[test]
    fn an_artboard_shows_the_preset_of_its_size() {
        assert_eq!(
            preset_of(&board(393., 852., "x")).unwrap().name,
            "iPhone 16"
        );
        // Two presets share 1920 × 1080: the one it is named after wins.
        let slide = board(1920., 1080., "Slide 16:9");
        assert_eq!(preset_of(&slide).unwrap().name, "Slide 16:9");
        let other = board(1920., 1080., "Home");
        assert_eq!(preset_of(&other).unwrap().name, "HD screen");
        assert!(preset_of(&board(400., 400., "Artboard")).is_none());

        let (a, b) = (board(393., 852., "a"), board(400., 400., "b"));
        assert_eq!(shown_preset(&[&a]), Some("iPhone 16"));
        assert_eq!(shown_preset(&[&b, &b]), Some("Custom"));
        assert_eq!(shown_preset(&[&a, &b]), None);
    }

    #[test]
    fn a_preset_resizes_artboards_only_keeping_their_corner_in_one_step() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = DocumentSession::create(dir.path().join("a.mindweave")).unwrap();
        let a = board(400., 400., "a");
        let mut text = Shape::new_text_box((0., 0.));
        text.frame.w = 50.;
        let (aid, tid) = (a.id, text.id);
        s.add_shapes(vec![a, text]);
        resize_artboards(&mut s, &[aid, tid], (393., 852.));
        let f = s.find_shape(aid).unwrap().frame;
        assert_eq!((f.x, f.y, f.w, f.h), (10., 20., 393., 852.));
        assert_eq!(s.find_shape(tid).unwrap().frame.w, 50.);
        assert!(s.undo());
        assert_eq!(s.find_shape(aid).unwrap().frame.w, 400.);
    }

    #[test]
    fn clip_content_is_one_step() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = DocumentSession::create(dir.path().join("a.mindweave")).unwrap();
        let a = board(400., 400., "a");
        let id = a.id;
        s.add_shapes(vec![a]);
        let clips = |s: &DocumentSession| {
            s.find_shape(id)
                .unwrap()
                .artboard_props
                .clone()
                .unwrap()
                .clip
        };
        set_clip(&mut s, &[id], false);
        assert!(!clips(&s));
        assert!(s.undo());
        assert!(clips(&s));
    }

    #[test]
    fn the_name_field_follows_the_selected_artboard_even_while_focused() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let shown = (a, "A".to_string());
        for focused in [false, true] {
            // Nothing shown yet, or another artboard selected: show it.
            assert!(needs_show(None, &shown, focused));
            assert!(needs_show(Some(&shown), &(b, "B".into()), focused));
            assert!(!needs_show(Some(&shown), &shown, focused));
        }
        // The same artboard renamed elsewhere: shown once not typing.
        assert!(needs_show(Some(&shown), &(a, "A2".into()), false));
        assert!(!needs_show(Some(&shown), &(a, "A2".into()), true));
    }
}
