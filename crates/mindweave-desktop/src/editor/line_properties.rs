//! Dropdowns in the Properties sidebar for how outlines and lines are
//! drawn: the outline's pattern and single or double stroke for every
//! shape, and, when every selected shape is a line, its heads, route and
//! jumps. Each choice is one undo step on every selected shape.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{h_flex, Icon, Sizable as _};
use gpui_kit::*;
use mindweave_core::{ArrowHead, LineDash, LineJump, LineRoute, Shape, JUMP_SIZES};
use uuid::Uuid;

use super::properties_panel::field_row;
use super::shape_properties::{common, format_number, Value, SECTION_GAP};
use super::{section_label, EditorWindow};
use crate::libraries::lucide_icon_path;

/// Width of a dropdown button.
const DROPDOWN_WIDTH: f32 = 120.;

/// A property a dropdown edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Pattern,
    Outline,
    StartHead,
    EndHead,
    Route,
    Jump,
    JumpSize,
}

impl Field {
    /// The fields every shape has, in the Style section.
    const STYLE: [Field; 2] = [Field::Pattern, Field::Outline];
    /// The fields only lines have, in the Line section.
    const LINE: [Field; 5] = [
        Field::StartHead,
        Field::EndHead,
        Field::Route,
        Field::Jump,
        Field::JumpSize,
    ];

    fn label(self) -> &'static str {
        match self {
            Field::Pattern => "Pattern",
            Field::Outline => "Outline",
            Field::StartHead => "Start",
            Field::EndHead => "End",
            Field::Route => "Route",
            Field::Jump => "Jump",
            Field::JumpSize => "Jump size",
        }
    }

    /// The choices, as menu labels.
    fn choices(self) -> Vec<String> {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect();
        match self {
            Field::Pattern => names(&LineDash::ALL.map(LineDash::name)),
            Field::Outline => names(&["Single", "Double"]),
            Field::StartHead | Field::EndHead => names(&ArrowHead::ALL.map(ArrowHead::name)),
            Field::Route => names(&LineRoute::ALL.map(LineRoute::name)),
            Field::Jump => names(&LineJump::ALL.map(LineJump::name)),
            Field::JumpSize => {
                let mut sizes = JUMP_SIZES.to_vec();
                sizes.sort_by(f32::total_cmp);
                sizes.into_iter().map(size_label).collect()
            }
        }
    }

    /// The label of `shape`'s value; `None` for a line field on a shape
    /// that is not a line.
    fn value(self, shape: &Shape) -> Option<String> {
        let style = &shape.style;
        let line = shape.line.as_ref();
        Some(match self {
            Field::Pattern => style.dash.name().to_string(),
            Field::Outline => if style.double { "Double" } else { "Single" }.to_string(),
            Field::StartHead => line?.start_head.name().to_string(),
            Field::EndHead => line?.end_head.name().to_string(),
            Field::Route => line?.route.name().to_string(),
            Field::Jump => line?.jump.name().to_string(),
            Field::JumpSize => size_label(line?.jump_size),
        })
    }

    /// Sets `shape` to the choice labelled `choice`.
    fn apply(self, shape: &mut Shape, choice: &str) {
        fn pick<T: Copy, const N: usize>(
            all: [T; N],
            name: fn(T) -> &'static str,
            choice: &str,
        ) -> Option<T> {
            all.into_iter().find(|&v| name(v) == choice)
        }
        match self {
            Field::Pattern => {
                if let Some(d) = pick(LineDash::ALL, LineDash::name, choice) {
                    shape.style.dash = d;
                }
                return;
            }
            Field::Outline => {
                shape.style.double = choice == "Double";
                return;
            }
            _ => {}
        }
        let Some(line) = shape.line.as_mut() else {
            return;
        };
        match self {
            Field::StartHead | Field::EndHead => {
                if let Some(h) = pick(ArrowHead::ALL, ArrowHead::name, choice) {
                    if self == Field::StartHead {
                        line.start_head = h;
                    } else {
                        line.end_head = h;
                    }
                }
            }
            Field::Route => {
                if let Some(r) = pick(LineRoute::ALL, LineRoute::name, choice) {
                    line.route = r;
                }
            }
            Field::Jump => {
                if let Some(j) = pick(LineJump::ALL, LineJump::name, choice) {
                    line.jump = j;
                }
            }
            Field::JumpSize => {
                if let Some(&size) = JUMP_SIZES.iter().find(|&&s| size_label(s) == choice) {
                    line.jump_size = size;
                }
            }
            Field::Pattern | Field::Outline => {}
        }
    }
}

fn size_label(size: f32) -> String {
    format!("{} px", format_number(size))
}

/// The shapes' shared value of `field`, or `None` when they differ.
fn shown(field: Field, shapes: &[&Shape]) -> Option<String> {
    match common(shapes.iter().map(|s| field.value(s)))? {
        Value::Same(v) => v,
        Value::Mixed => None,
    }
}

/// Whether something is selected and all of it is lines.
pub(super) fn all_lines(shapes: &[&Shape]) -> bool {
    !shapes.is_empty() && shapes.iter().all(|s| s.is_line() && s.line.is_some())
}

/// A row with `field`'s dropdown for the shapes `ids`.
fn dropdown_row(
    field: Field,
    shapes: &[&Shape],
    ids: &[Uuid],
    cx: &mut Context<EditorWindow>,
) -> AnyElement {
    let current = shown(field, shapes);
    let this = cx.entity().downgrade();
    let ids = ids.to_vec();
    let label = current.clone().unwrap_or_else(|| "Mixed".into());
    let button = Button::new(("outline-field", field as usize))
        .outline()
        .small()
        .w(px(DROPDOWN_WIDTH))
        .label(label)
        .dropdown_caret(true)
        .dropdown_menu(move |mut menu, _, _| {
            for choice in field.choices() {
                let ids = ids.clone();
                let this = this.clone();
                let checked = current.as_deref() == Some(choice.as_str());
                menu = menu.item(
                    PopupMenuItem::new(choice.clone())
                        .checked(checked)
                        .on_click(move |_, _, cx| {
                            let ids = ids.clone();
                            let choice = choice.clone();
                            this.update(cx, |this, cx| {
                                this.update_session(
                                    |s| s.update_shapes(&ids, |shape| field.apply(shape, &choice)),
                                    cx,
                                );
                            })
                            .ok();
                        }),
                );
            }
            menu
        });
    field_row(field.label(), None, button).into_any_element()
}

/// The Style section's Pattern and Outline rows for the selection.
pub(super) fn style_rows(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> Vec<AnyElement> {
    let shapes = editor.shapes_among(&editor.selection);
    let ids: Vec<Uuid> = shapes.iter().map(|s| s.id).collect();
    Field::STYLE
        .into_iter()
        .map(|field| dropdown_row(field, &shapes, &ids, cx))
        .collect()
}

/// The Line section, when every selected shape is a line.
pub(super) fn render(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> Option<AnyElement> {
    let shapes = editor.shapes_among(&editor.selection);
    if !all_lines(&shapes) {
        return None;
    }
    let ids: Vec<Uuid> = shapes.iter().map(|s| s.id).collect();
    let rows: Vec<AnyElement> = Field::LINE
        .into_iter()
        .map(|field| dropdown_row(field, &shapes, &ids, cx))
        .collect();
    let swap = Button::new("line-swap-heads")
        .ghost()
        .xsmall()
        .icon(Icon::empty().path(lucide_icon_path("arrow-left-right", cx)))
        .tooltip("Swap Start and End Heads")
        .on_click(cx.listener(move |this, _, _, cx| {
            let ids = ids.clone();
            this.update_session(
                |s| {
                    s.update_shapes(&ids, |shape| {
                        if let Some(p) = shape.line.as_mut() {
                            std::mem::swap(&mut p.start_head, &mut p.end_head);
                        }
                    })
                },
                cx,
            );
        }));
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                h_flex()
                    .justify_between()
                    .mt(px(SECTION_GAP))
                    .child(section_label("Line", cx))
                    .child(swap),
            )
            .children(rows)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::{shown, Field};
    use mindweave_core::geometry::Frame;
    use mindweave_core::{builtin_libraries, ArrowHead, LineDash, LineJump, Shape};

    fn shape(id: &str) -> Shape {
        let libs = builtin_libraries();
        Shape::new(libs[0].shape(id).unwrap(), "basic-shapes", Frame::default())
    }

    #[test]
    fn every_choice_applies_and_shows_back() {
        for field in Field::STYLE.into_iter().chain(Field::LINE) {
            for choice in field.choices() {
                let mut s = shape("line");
                field.apply(&mut s, &choice);
                assert_eq!(
                    field.value(&s).as_deref(),
                    Some(choice.as_str()),
                    "{field:?}"
                );
            }
        }
        let mut s = shape("line");
        Field::EndHead.apply(&mut s, "Triangle");
        Field::Jump.apply(&mut s, "Arc");
        let line = s.line.unwrap();
        assert_eq!(
            (line.end_head, line.start_head),
            (ArrowHead::Triangle, ArrowHead::None)
        );
        assert_eq!(line.jump, LineJump::Arc);
    }

    #[test]
    fn other_shapes_take_the_outline_style_only() {
        let mut r = shape("rectangle");
        Field::Pattern.apply(&mut r, "Dashed");
        Field::Outline.apply(&mut r, "Double");
        Field::Jump.apply(&mut r, "Arc");
        assert_eq!((r.style.dash, r.style.double), (LineDash::Dashed, true));
        assert!(r.line.is_none());
        assert_eq!(Field::Jump.value(&r), None);
    }

    #[test]
    fn differing_shapes_show_mixed() {
        let a = shape("line");
        let mut b = shape("line");
        Field::EndHead.apply(&mut b, "Arrow");
        assert_eq!(shown(Field::EndHead, &[&a, &b]), None);
        assert_eq!(shown(Field::StartHead, &[&a, &b]).as_deref(), Some("None"));
        let r = shape("rectangle");
        assert_eq!(shown(Field::Pattern, &[&a, &r]).as_deref(), Some("Solid"));
    }
}
