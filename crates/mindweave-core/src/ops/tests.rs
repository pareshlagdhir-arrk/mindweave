//! Tests of the command layer.

use std::path::PathBuf;
use std::sync::Arc;

use uuid::Uuid;

use super::*;
use crate::library::{builtin_libraries, LibraryRegistry};
use crate::task::TaskStatus;
use crate::transform::frame_aabb;

fn libs() -> Arc<LibraryRegistry> {
    Arc::new(LibraryRegistry::new(builtin_libraries(), None).0)
}

fn new_ws(name: &str) -> (tempfile::TempDir, PathBuf, Workspace) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    let ws = Workspace::create(&path, Some("Ideas"), libs()).unwrap();
    (dir, path, ws)
}

fn reopen(path: &std::path::Path) -> Workspace {
    Workspace::open(path, libs()).unwrap()
}

fn errors(ws: &Workspace) -> Vec<Problem> {
    ws.validate()
        .into_iter()
        .filter(|p| p.severity == Severity::Error)
        .collect()
}

fn centre(ws: &Workspace, id: Uuid) -> (f32, f32) {
    frame_aabb(&ws.shape(id).unwrap().frame).center()
}

#[test]
fn create_names_the_page_and_refuses_to_replace_a_file() {
    let (_dir, path, ws) = new_ws("a.mindweave");
    assert_eq!(ws.summary().pages[0].name, "Ideas");
    assert_eq!(reopen(&path).summary().pages[0].name, "Ideas");
    let err = Workspace::create(&path, None, libs()).err().unwrap();
    assert!(err.to_string().contains("already exists"), "{err}");
}

#[test]
fn a_mind_map_is_built_saved_reloaded_and_valid() {
    let (_dir, path, mut ws) = new_ws("map.mindweave");
    let map = ws
        .add_mind_map(None, "Project", Some((0.0, 0.0)), None)
        .unwrap();
    let mains = ws
        .add_topics(
            map,
            &["Goals".into(), "Risks".into(), "People".into()],
            TopicPosition::Child,
        )
        .unwrap();
    assert_eq!(mains.len(), 3);
    let subs = ws
        .add_tree(
            mains[0],
            &[
                TopicNode {
                    text: "Ship v1".into(),
                    note: Some("- **fast**\n- *safe*".into()),
                    children: vec![TopicNode {
                        text: "Beta in May".into(),
                        ..TopicNode::default()
                    }],
                    ..TopicNode::default()
                },
                TopicNode {
                    text: "Grow".into(),
                    equation: Some("E = mc^2".into()),
                    link: Some("example.com".into()),
                    ..TopicNode::default()
                },
            ],
        )
        .unwrap();
    assert_eq!(subs.len(), 3);
    let summary = ws.add_summary(&[subs[0], subs[2]], Some("Plan")).unwrap();
    let boundary = ws.add_boundary(&[mains[1]]).unwrap();
    assert_eq!(ws.add_boundary(&[mains[1]]).unwrap(), boundary);
    let rel = ws
        .add_relationship(mains[1], mains[2], Some("affects"))
        .unwrap();
    ws.save().unwrap();

    let ws = reopen(&path);
    assert!(errors(&ws).is_empty(), "{:?}", ws.validate());
    let central = ws.get_object(map).unwrap();
    let t = central.topic.unwrap();
    assert_eq!(t.children, mains);
    assert_eq!(central.text.as_deref(), Some("Project"));
    let ship = ws.get_object(subs[0]).unwrap();
    assert_eq!(
        ship.topic.as_ref().unwrap().note.as_deref(),
        Some("fast\nsafe")
    );
    assert!(ship.topic.unwrap().summaries.is_empty());
    let goals = ws.get_object(mains[0]).unwrap().topic.unwrap();
    assert_eq!(goals.summaries, vec![summary]);
    let grow = ws.get_object(subs[2]).unwrap().topic.unwrap();
    assert_eq!(grow.equation.as_deref(), Some("E = mc^2"));
    assert_eq!(grow.link.as_deref(), Some("https://example.com"));
    let line = ws.get_object(rel).unwrap();
    assert_eq!(line.kind, ObjectKind::Relationship);
    let l = line.line.unwrap();
    assert_eq!((l.start, l.end), (Some(mains[1]), Some(mains[2])));
    assert_eq!(
        ws.get_object(boundary).unwrap().members,
        Some(vec![mains[1]])
    );

    // Laid out: main topics beside the central topic, children further
    // out than their parent, siblings not overlapping.
    let c = centre(&ws, map);
    for m in &mains {
        let p = centre(&ws, *m);
        assert!((p.0 - c.0).abs() > 60.0, "main topic {p:?} beside {c:?}");
    }
    let (g, s0) = (centre(&ws, mains[0]), centre(&ws, subs[0]));
    assert!((s0.0 - c.0).abs() > (g.0 - c.0).abs());
    let a = frame_aabb(&ws.shape(subs[0]).unwrap().frame);
    let b = frame_aabb(&ws.shape(subs[2]).unwrap().frame);
    assert!(a.y + a.h <= b.y || b.y + b.h <= a.y, "{a:?} {b:?}");
    // Topics are sized to their text.
    let wide = ws.shape(subs[1]).unwrap().frame.w;
    let short = ws.shape(subs[2]).unwrap().frame.w;
    assert!(wide > short, "{wide} > {short}");
}

#[test]
fn topics_go_before_and_after_in_the_order_given() {
    let (_dir, _path, mut ws) = new_ws("order.mindweave");
    let map = ws.add_mind_map(None, "C", None, Some("logic")).unwrap();
    let mid = ws
        .add_topics(map, &["M".into()], TopicPosition::Child)
        .unwrap()[0];
    let after = ws
        .add_topics(mid, &["A1".into(), "A2".into()], TopicPosition::After)
        .unwrap();
    let before = ws
        .add_topics(mid, &["B1".into(), "B2".into()], TopicPosition::Before)
        .unwrap();
    let kids = ws.get_object(map).unwrap().topic.unwrap().children;
    assert_eq!(kids, vec![before[0], before[1], mid, after[0], after[1]]);
    let err = ws
        .add_topics(Uuid::new_v4(), &["x".into()], TopicPosition::Child)
        .unwrap_err();
    assert!(err.to_string().contains("list_objects"), "{err}");
}

#[test]
fn diagrams_connect_group_style_and_delete() {
    let (_dir, path, mut ws) = new_ws("diagram.mindweave");
    let a = ws
        .add_shape(
            None,
            "basic-shapes",
            "rectangle",
            (0.0, 0.0),
            None,
            Some("Start"),
            None,
        )
        .unwrap()[0];
    let b = ws
        .add_shape(
            None,
            "basic-shapes",
            "ellipse",
            (300.0, 0.0),
            Some((100.0, 60.0)),
            Some("End"),
            Some(&StyleInput {
                fill: Some("#FFEEAA".into()),
                ..StyleInput::default()
            }),
        )
        .unwrap()[0];
    let note = ws
        .add_text(None, (0.0, 200.0), "A note", None, None)
        .unwrap();
    let line = ws
        .connect(
            a,
            b,
            &ConnectOptions {
                end_head: Some("triangle".into()),
                text: Some("next".into()),
                ..ConnectOptions::default()
            },
        )
        .unwrap();
    let info = ws.get_object(line).unwrap();
    let l = info.line.clone().unwrap();
    assert_eq!((l.start, l.end), (Some(a), Some(b)));
    assert_eq!(l.end_head, crate::line::ArrowHead::Triangle);
    // The ends meet the outlines, not the centres.
    assert!((l.from.0 - 120.0).abs() < 1.0, "{:?}", l.from);
    assert!((l.to.0 - 300.0).abs() < 1.0, "{:?}", l.to);
    assert_eq!(info.text.as_deref(), Some("next"));

    let group = ws.group(&[a, b]).unwrap();
    assert_eq!(ws.get_object(a).unwrap().groups, vec![group]);
    ws.set_style(
        &[a],
        &StyleInput {
            stroke: Some("#FF0000".into()),
            dash: Some("dashed".into()),
            corner_radius: Some(12.0),
            ..StyleInput::default()
        },
    )
    .unwrap();
    let ai = ws.get_object(a).unwrap();
    assert_eq!(ai.style.stroke, "#FF0000");
    assert_eq!(ai.params.unwrap()["corner_radius"], 12.0);
    let bad = ws
        .set_style(
            &[a],
            &StyleInput {
                fill: Some("reddish".into()),
                ..StyleInput::default()
            },
        )
        .unwrap_err();
    assert!(bad.to_string().contains("#RRGGBB"), "{bad}");

    ws.set_text(
        note,
        &TextInput {
            text: Some("A longer note".into()),
            bold: Some(true),
            ..TextInput::default()
        },
    )
    .unwrap();
    assert!(ws.get_object(note).unwrap().text_style.unwrap().bold);

    // Moving a shape moves the attached end along.
    ws.move_objects(&[b], 0.0, 100.0).unwrap();
    let l = ws.get_object(line).unwrap().line.unwrap();
    assert!(l.to.1 > 50.0, "{:?}", l.to);

    let removed = ws.delete(&[b]).unwrap();
    assert_eq!(removed, vec![b]);
    let l = ws.get_object(line).unwrap().line.unwrap();
    assert_eq!((l.start, l.end), (Some(a), None));
    ws.save().unwrap();
    assert!(errors(&reopen(&path)).is_empty());
}

#[test]
fn templates_place_grouped_parts_and_unknown_items_are_refused() {
    let (_dir, _path, mut ws) = new_ws("t.mindweave");
    let err = ws
        .add_shape(
            None,
            "basic-shapes",
            "octagon",
            (0.0, 0.0),
            None,
            None,
            None,
        )
        .unwrap_err();
    assert!(err.to_string().contains("list_libraries"), "{err}");
    let err = ws
        .add_shape(None, "nope", "x", (0.0, 0.0), None, None, None)
        .unwrap_err();
    assert!(err.to_string().contains("basic-shapes"), "{err}");
    let libs = ws.list_libraries();
    assert!(libs[0].items.iter().any(|i| i.id == "rectangle"));
}

#[test]
fn artboards_are_placed_side_by_side_and_take_in_content() {
    let (_dir, _path, mut ws) = new_ws("boards.mindweave");
    let first = ws
        .add_artboard(None, &ArtboardSize::Preset("iphone 16".into()), None, None)
        .unwrap();
    let second = ws
        .add_artboard(
            None,
            &ArtboardSize::Size {
                width: 500.0,
                height: 300.0,
            },
            Some("Wide"),
            None,
        )
        .unwrap();
    let f1 = ws.shape(first).unwrap().frame;
    let f2 = ws.shape(second).unwrap().frame;
    assert_eq!((f1.x, f1.y, f1.w, f1.h), (0.0, 0.0, 393.0, 852.0));
    assert_eq!((f2.x, f2.y), (393.0 + crate::ARTBOARD_GAP, 0.0));
    assert_eq!(ws.get_object(second).unwrap().name.as_deref(), Some("Wide"));
    let inside = ws
        .add_shape(
            None,
            "basic-shapes",
            "rectangle",
            (20.0, 20.0),
            None,
            None,
            None,
        )
        .unwrap()[0];
    let outside = ws
        .add_text(None, (-500.0, 0.0), "Outside", None, None)
        .unwrap();
    assert_eq!(ws.get_object(inside).unwrap().artboard, Some(first));
    assert_eq!(ws.get_object(outside).unwrap().artboard, None);
    let map = ws
        .add_mind_map(None, "In board", Some((700.0, 100.0)), None)
        .unwrap();
    let kid = ws
        .add_topics(map, &["k".into()], TopicPosition::Child)
        .unwrap()[0];
    assert_eq!(ws.get_object(kid).unwrap().artboard, Some(second));
    // Moving it out of the artboard ends the membership.
    ws.move_objects(&[inside], -1000.0, 0.0).unwrap();
    assert_eq!(ws.get_object(inside).unwrap().artboard, None);
    let members = ws
        .list_objects(
            None,
            &ObjectFilter {
                artboard: Some(second),
                ..ObjectFilter::default()
            },
            false,
        )
        .unwrap();
    assert_eq!(members.len(), 2);
    let err = ws
        .add_artboard(None, &ArtboardSize::Preset("Poster".into()), None, None)
        .unwrap_err();
    assert!(err.to_string().contains("A4"), "{err}");
    assert!(errors(&ws).is_empty());
}

#[test]
fn list_objects_filters_by_kind_text_and_map() {
    let (_dir, _path, mut ws) = new_ws("list.mindweave");
    let map = ws.add_mind_map(None, "Alpha", None, None).unwrap();
    ws.add_topics(
        map,
        &["Beta".into(), "gamma ray".into()],
        TopicPosition::Child,
    )
    .unwrap();
    ws.add_text(None, (0.0, 500.0), "Gamma note", None, None)
        .unwrap();
    let all = ws
        .list_objects(None, &ObjectFilter::default(), false)
        .unwrap();
    assert_eq!(all.len(), 4);
    let topics = ws
        .list_objects(
            None,
            &ObjectFilter {
                kind: Some(ObjectKind::Topic),
                ..ObjectFilter::default()
            },
            false,
        )
        .unwrap();
    assert_eq!(topics.len(), 3);
    let gamma = ws
        .list_objects(
            None,
            &ObjectFilter {
                text_contains: Some("GAMMA".into()),
                ..ObjectFilter::default()
            },
            false,
        )
        .unwrap();
    assert_eq!(gamma.len(), 2);
    let of_map = ws
        .list_objects(
            None,
            &ObjectFilter {
                mind_map: Some(map),
                ..ObjectFilter::default()
            },
            false,
        )
        .unwrap();
    assert_eq!(of_map.len(), 3);
    assert!(ws
        .list_objects(Some(3), &ObjectFilter::default(), false)
        .is_err());
}

#[test]
fn editing_a_file_keeps_unrelated_objects_unchanged() {
    let (_dir, path, mut ws) = new_ws("keep.mindweave");
    let map = ws.add_mind_map(None, "Keep", None, None).unwrap();
    ws.add_topics(map, &["one".into(), "two".into()], TopicPosition::Child)
        .unwrap();
    ws.add_shape(
        None,
        "basic-shapes",
        "star",
        (500.0, 500.0),
        None,
        Some("*"),
        None,
    )
    .unwrap();
    ws.add_page(Some("Second")).unwrap();
    ws.save().unwrap();
    let before = crate::format::load(&path).unwrap();

    let mut ws = reopen(&path);
    let added = ws
        .add_text(Some(0), (900.0, 900.0), "New", None, None)
        .unwrap();
    ws.save().unwrap();
    let mut after = crate::format::load(&path).unwrap();
    after.pages[0].layers[0].objects.retain(|s| s.id != added);
    assert_eq!(before, after);
}

#[test]
fn pages_and_layers_are_added_and_targeted() {
    let (_dir, path, mut ws) = new_ws("pages.mindweave");
    let p = ws.add_page(Some("Detail")).unwrap();
    assert_eq!(p, 1);
    let l = ws.add_layer(Some(1), Some("Notes")).unwrap();
    assert_eq!(l, 0);
    let id = ws
        .add_text(Some(1), (0.0, 0.0), "on page 2", None, None)
        .unwrap();
    let info = ws.get_object(id).unwrap();
    assert_eq!((info.page, info.layer), (1, 0));
    ws.set_active(0, Some(0)).unwrap();
    assert!(ws.set_active(0, Some(5)).is_err());
    ws.save().unwrap();
    let s = reopen(&path).summary();
    assert_eq!(s.pages.len(), 2);
    assert_eq!(s.pages[1].layers[0].name, "Notes");
    assert_eq!(s.pages[1].counts.get("text"), Some(&1));
    assert_eq!(s.active_page, 0);
}

#[test]
fn validate_reports_dangling_references() {
    let (_dir, _path, mut ws) = new_ws("bad.mindweave");
    let a = ws
        .add_shape(
            None,
            "basic-shapes",
            "rectangle",
            (0.0, 0.0),
            None,
            None,
            None,
        )
        .unwrap()[0];
    let mut doc = ws.doc().clone();
    let shape = &mut doc.pages[0].layers[0].objects[0];
    assert_eq!(shape.id, a);
    shape.artboard = Some(Uuid::new_v4());
    shape.library = "missing-lib".into();
    let problems = validate(&doc, Some(ws.libraries()));
    assert!(problems
        .iter()
        .any(|p| p.severity == Severity::Error && p.object == Some(a)));
    assert!(problems.iter().any(|p| p.severity == Severity::Warning));
}

#[test]
fn validate_checks_groups() {
    let (_dir, _path, mut ws) = new_ws("groups.mindweave");
    let a = ws
        .add_shape(
            None,
            "basic-shapes",
            "rectangle",
            (0.0, 0.0),
            None,
            None,
            None,
        )
        .unwrap()[0];
    let b = ws
        .add_shape(
            None,
            "basic-shapes",
            "ellipse",
            (200.0, 0.0),
            None,
            None,
            None,
        )
        .unwrap()[0];
    let c = ws
        .add_shape(None, "basic-shapes", "star", (400.0, 0.0), None, None, None)
        .unwrap()[0];
    let g = ws.group(&[a, b]).unwrap();
    assert!(ws.validate().is_empty(), "{:?}", ws.validate());
    let about = |doc: &crate::document::Document, sev: Severity, words: &str| {
        validate(doc, None)
            .iter()
            .any(|p| p.severity == sev && p.message.contains(words))
    };
    // One member left.
    let mut doc = ws.doc().clone();
    doc.pages[0].layers[0].objects[1].groups.clear();
    assert!(about(&doc, Severity::Warning, "only one member"));
    // Members on two layers.
    let mut doc = ws.doc().clone();
    let moved = doc.pages[0].layers[0].objects.remove(1);
    let mut top = crate::document::Layer::new("Top");
    top.objects.push(moved);
    doc.pages[0].layers.insert(0, top);
    assert!(about(&doc, Severity::Error, "different layers"));
    // A group nested differently in its members.
    let mut doc = ws.doc().clone();
    let objects = &mut doc.pages[0].layers[0].objects;
    objects[0].groups = vec![Uuid::new_v4(), g];
    assert_eq!(objects[2].id, c);
    assert!(about(&doc, Severity::Error, "dangling group id"));
}

#[test]
fn topics_cannot_be_resized_by_hand() {
    let (_dir, _path, mut ws) = new_ws("topic-size.mindweave");
    let map = ws.add_mind_map(None, "C", None, None).unwrap();
    let err = ws
        .set_frame(map, (0.0, 0.0), Some((300.0, 100.0)))
        .unwrap_err();
    assert!(err.to_string().contains("size themselves"), "{err}");
    ws.set_frame(map, (50.0, 50.0), None).unwrap();
    assert_eq!(ws.shape(map).unwrap().frame.x, 50.0);
}

#[test]
fn tasks_roll_up_and_bad_values_are_refused() {
    let (_dir, path, mut ws) = new_ws("t.mindweave");
    let map = ws.add_mind_map(None, "Launch", None, None).unwrap();
    let task = |progress: Option<u8>, status: Option<&str>| {
        Some(TaskEdit {
            progress,
            status: status.map(Into::into),
            ..TaskEdit::default()
        })
    };
    let ids = ws
        .add_tree(
            map,
            &[TopicNode {
                text: "Build".into(),
                task: Some(TaskEdit {
                    description: Some("The first release".into()),
                    ..TaskEdit::default()
                }),
                children: vec![
                    TopicNode {
                        text: "API".into(),
                        task: Some(TaskEdit {
                            start: Some("2026-11-02".into()),
                            end: Some("2026-11-20".into()),
                            progress: Some(100),
                            ..TaskEdit::default()
                        }),
                        ..TopicNode::default()
                    },
                    TopicNode {
                        text: "UI".into(),
                        task: task(None, Some("in-progress")),
                        ..TopicNode::default()
                    },
                ],
                ..TopicNode::default()
            }],
        )
        .unwrap();
    ws.save().unwrap();
    let ws2 = reopen(&path);
    let build = ws2.get_object(ids[0]).unwrap().topic.unwrap().task.unwrap();
    assert_eq!(build.description, "The first release");
    // (100 + 0) / 2; dates from the subtasks.
    assert_eq!((build.progress, build.status), (50, TaskStatus::InProgress));
    assert_eq!(build.subtasks, 2);
    assert_eq!(build.start.as_deref(), Some("2026-11-02"));
    let api = ws2.get_object(ids[1]).unwrap().topic.unwrap().task.unwrap();
    assert_eq!(api.status, TaskStatus::Done);

    let edit = |t: Option<TaskEdit>| TopicEdit {
        task: t,
        ..TopicEdit::default()
    };
    for bad in [
        TaskEdit {
            start: Some("2026-02-30".into()),
            ..TaskEdit::default()
        },
        TaskEdit {
            start: Some("2026-12-01".into()),
            end: Some("2026-11-01".into()),
            ..TaskEdit::default()
        },
        TaskEdit {
            progress: Some(101),
            ..TaskEdit::default()
        },
        TaskEdit {
            status: Some("later".into()),
            ..TaskEdit::default()
        },
    ] {
        assert!(ws.set_topic(ids[2], &edit(Some(bad))).is_err());
    }
    ws.set_topic(
        ids[2],
        &edit(Some(TaskEdit {
            remove: true,
            ..TaskEdit::default()
        })),
    )
    .unwrap();
    let ui = ws.get_object(ids[2]).unwrap().topic.unwrap();
    assert!(ui.task.is_none());
    let build = ws.get_object(ids[0]).unwrap().topic.unwrap().task.unwrap();
    assert_eq!((build.progress, build.subtasks), (100, 1));
}
