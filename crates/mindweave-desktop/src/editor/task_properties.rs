//! The Task section of the Properties sidebar, shown when every selected
//! shape is a mind map topic: makes the topics tasks and, for one task,
//! edits its dates, progress and status, and opens its rich-text
//! description in the note card. A parent task (one
//! with subtasks) shows what its subtasks roll up to. Each change is one
//! undo step.

use chrono::NaiveDate;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{v_flex, ActiveTheme as _, Sizable as _};
use gpui_kit::*;
use mindweave_core::{task_views, Task, TaskStatus, TaskView};
use uuid::Uuid;

use super::note_card::note_preview;
use super::properties_panel::field_row;
use super::shape_properties::SECTION_GAP;
use super::topic_properties::{dropdown, CONTROL_WIDTH};
use super::{section_label, EditorWindow};

/// How dates are written in the date fields and the file.
const DATE_FORMAT: &str = "%Y-%m-%d";

/// Which date field.
#[derive(Clone, Copy)]
enum Which {
    Start,
    End,
}

/// The fields of the one selected task, and the task they show.
pub struct TaskFields {
    start: Entity<DatePickerState>,
    end: Entity<DatePickerState>,
    progress: Entity<SliderState>,
    /// The topic and task shown, so a new selection or an edit made
    /// elsewhere (undo, the badge) shows its own values.
    shows: Option<(Uuid, Task)>,
}

impl TaskFields {
    pub fn new(window: &mut Window, cx: &mut Context<EditorWindow>) -> (Self, Vec<Subscription>) {
        let date = |window: &mut Window, cx: &mut Context<EditorWindow>| {
            cx.new(|cx| DatePickerState::new(window, cx).date_format(DATE_FORMAT))
        };
        let start = date(window, cx);
        let end = date(window, cx);
        let progress = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(5.)
                .default_value(0.)
        });
        let subscriptions = vec![
            cx.subscribe_in(&start, window, |this, _, event, _, cx| {
                let DatePickerEvent::Change(value) = event;
                this.set_task_date(Which::Start, value.start().map(|d| d.date()), cx);
            }),
            cx.subscribe_in(&end, window, |this, _, event, _, cx| {
                let DatePickerEvent::Change(value) = event;
                this.set_task_date(Which::End, value.start().map(|d| d.date()), cx);
            }),
            cx.subscribe_in(&progress, window, |this, _, event, _, cx| match event {
                // The percentage follows the thumb; the task changes on
                // release, as one step.
                SliderEvent::Change(_) => cx.notify(),
                SliderEvent::Release(value) => {
                    let p = value.start().round().clamp(0., 100.) as u8;
                    this.edit_tasks(|t| t.set_progress(p), cx);
                }
            }),
        ];
        let fields = TaskFields {
            start,
            end,
            progress,
            shows: None,
        };
        (fields, subscriptions)
    }
}

/// `date` (`YYYY-MM-DD`) as a calendar date.
fn parse_date(date: Option<&str>) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(date?, DATE_FORMAT).ok()
}

impl EditorWindow {
    /// The one selected topic when it is a task, with its task.
    fn selected_task(&self) -> Option<(Uuid, Task)> {
        let id = self.selected_topic()?;
        let task = self.session.find_shape(id)?.topic.as_ref()?.task.clone()?;
        Some((id, task))
    }

    /// Shows the selected task in the fields when the selection or the
    /// task changed.
    pub(super) fn sync_task_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.selected_task();
        if self.task_fields.shows == wanted {
            return;
        }
        if let Some((_, task)) = &wanted {
            let fields = &self.task_fields;
            for (picker, date) in [(&fields.start, &task.start), (&fields.end, &task.end)] {
                let date = parse_date(date.as_deref());
                picker.update(cx, |state, cx| {
                    state.set_date(Date::Single(date), window, cx)
                });
            }
            fields.progress.update(cx, |state, cx| {
                state.set_value(f32::from(task.progress), window, cx)
            });
        }
        self.task_fields.shows = wanted;
    }

    /// Sets (or with `None` clears) the selected task's start or end date,
    /// if it changed. An end before the start moves the start to it, and
    /// the other way round.
    fn set_task_date(&mut self, which: Which, date: Option<NaiveDate>, cx: &mut Context<Self>) {
        let Some((_, task)) = self.selected_task() else {
            return;
        };
        let date = date.map(|d| d.format(DATE_FORMAT).to_string());
        let now = match which {
            Which::Start => &task.start,
            Which::End => &task.end,
        };
        if *now == date {
            return;
        }
        self.edit_tasks(
            |t| match which {
                Which::Start => {
                    t.start = date.clone();
                    if t.end.is_some() && t.end < date {
                        t.end = date.clone();
                    }
                }
                Which::End => {
                    t.end = date.clone();
                    if date.is_some() && t.start > date {
                        t.start = date.clone();
                    }
                }
            },
            cx,
        );
    }
}

/// A read-only value in a row.
fn value_text(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .w(px(CONTROL_WIDTH))
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// The Task section, when every selected shape is a topic: the Task
/// switch and, for one selected task, its fields.
pub(super) fn render(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> Option<AnyElement> {
    let shapes = editor.shapes_among(&editor.selection);
    if shapes.is_empty() || shapes.iter().any(|s| s.topic.is_none()) {
        return None;
    }
    let is_task = editor.selected_topics_are_tasks();
    let this = cx.entity().downgrade();
    let switch_row = field_row(
        "Task",
        Some("Track the topic as a task. Subtopics added to a task are tasks too; a task with subtasks rolls up their progress."),
        Switch::new("task-on")
            .checked(is_task)
            .small()
            .on_click(move |_, _, cx| {
                this.update(cx, |this, cx| this.toggle_task(cx)).ok();
            }),
    );
    let section = v_flex()
        .gap(px(4.))
        .child(section_label("Task", cx).mt(px(SECTION_GAP)))
        .child(switch_row);
    let Some((id, task)) = editor.selected_task() else {
        return Some(section.into_any_element());
    };
    let view: TaskView = task_views(editor.session().page())
        .remove(&id)
        .unwrap_or(TaskView {
            progress: task.progress,
            status: task.status,
            start: task.start.clone(),
            end: task.end.clone(),
            subtasks: 0,
        });
    let fields = &editor.task_fields;
    let preview = task
        .description
        .as_ref()
        .map(note_preview)
        .filter(|p| !p.is_empty());
    let has_description = preview.is_some();
    let description = v_flex()
        .gap(px(2.))
        .child(field_row(
            "Description",
            Some("Rich text: bold, italic, lists and links, edited in a card beside the topic"),
            Button::new("task-description")
                .ghost()
                .xsmall()
                .label(if has_description { "Edit…" } else { "Add…" })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_description_card(id, window, cx)
                })),
        ))
        .children(preview.map(|p| {
            div()
                .text_size(px(12.))
                .text_color(cx.theme().muted_foreground)
                .child(p)
        }));
    let section = section.child(description);
    let section = if view.is_parent() {
        let subtasks = match view.subtasks {
            1 => "1 subtask".to_string(),
            n => format!("{n} subtasks"),
        };
        let dates = |d: &Option<String>| d.clone().unwrap_or_else(|| "–".into());
        section
            .child(field_row(
                "Progress",
                Some("A parent task's progress is the average of its subtasks'"),
                value_text(format!("{}% of {subtasks}", view.progress), cx),
            ))
            .child(field_row(
                "Status",
                None,
                value_text(view.status.name(), cx),
            ))
            .child(field_row("Start", None, value_text(dates(&view.start), cx)))
            .child(field_row("End", None, value_text(dates(&view.end), cx)))
    } else {
        let shown = fields.progress.read(cx).value().start().round() as u8;
        section
            .child(field_row(
                "Start",
                None,
                div()
                    .w(px(CONTROL_WIDTH))
                    .child(DatePicker::new(&fields.start).small().cleanable(true)),
            ))
            .child(field_row(
                "End",
                None,
                div()
                    .w(px(CONTROL_WIDTH))
                    .child(DatePicker::new(&fields.end).small().cleanable(true)),
            ))
            .child(field_row(
                format!("Progress {shown}%"),
                Some("Done at 100%; click the task's badge on the canvas to tick it off"),
                div()
                    .w(px(CONTROL_WIDTH))
                    .child(Slider::new(&fields.progress)),
            ))
            .child(field_row(
                "Status",
                None,
                dropdown(
                    "task-status",
                    Some(task.status.name().to_string()),
                    TaskStatus::ALL.map(|s| s.name().to_string()).to_vec(),
                    |this, choice, cx| {
                        if let Some(s) = TaskStatus::ALL.into_iter().find(|s| s.name() == choice) {
                            this.edit_tasks(|t| t.set_status(s), cx);
                        }
                    },
                    cx,
                ),
            ))
    };
    Some(section.into_any_element())
}
