//! Tasks: a topic can be a task, with a description, start and end dates,
//! progress and a status. A task with subtasks (child topics that are
//! tasks) is a parent task: its progress, status and dates are rolled up
//! from them, so only tasks without subtasks are ticked off by hand.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::document::Page;
use crate::text::{HAlign, RichText, RunStyle};

/// Where a task stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    #[default]
    NotStarted,
    InProgress,
    Blocked,
    Done,
}

impl TaskStatus {
    pub const ALL: [TaskStatus; 4] = [
        TaskStatus::NotStarted,
        TaskStatus::InProgress,
        TaskStatus::Blocked,
        TaskStatus::Done,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TaskStatus::NotStarted => "Not Started",
            TaskStatus::InProgress => "In Progress",
            TaskStatus::Blocked => "Blocked",
            TaskStatus::Done => "Done",
        }
    }

    /// The name used in files and by agents (`"not-started"`, …).
    pub fn key(self) -> &'static str {
        match self {
            TaskStatus::NotStarted => "not-started",
            TaskStatus::InProgress => "in-progress",
            TaskStatus::Blocked => "blocked",
            TaskStatus::Done => "done",
        }
    }

    pub fn from_key(key: &str) -> Option<TaskStatus> {
        TaskStatus::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// What makes a topic a task.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// What needs doing, as rich text (like a note).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "description_from"
    )]
    pub description: Option<RichText>,
    /// The start date, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    /// The end (due) date, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    /// Percent done, 0 to 100.
    #[serde(default)]
    pub progress: u8,
    #[serde(default)]
    pub status: TaskStatus,
}

/// A description as rich text, or as plain text (as the first builds
/// with tasks wrote it); blank is none.
fn description_from<'de, D: Deserializer<'de>>(d: D) -> Result<Option<RichText>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Plain(String),
        Rich(RichText),
    }
    Ok(match Option::<Stored>::deserialize(d)? {
        Some(Stored::Plain(s)) if !s.trim().is_empty() => {
            let mut text = RichText::new(RunStyle::default(), HAlign::Left);
            text.insert(0, &s, &RunStyle::default());
            Some(text)
        }
        Some(Stored::Rich(text)) if !text.plain().trim().is_empty() => Some(text),
        _ => None,
    })
}

impl Task {
    /// Sets the progress (clamped to 100) and keeps the status in step:
    /// 100 is done, 0 not started (unless blocked), anything between in
    /// progress (unless blocked).
    pub fn set_progress(&mut self, progress: u8) {
        self.progress = progress.min(100);
        self.status = match (self.progress, self.status) {
            (100, _) => TaskStatus::Done,
            (_, TaskStatus::Blocked) => TaskStatus::Blocked,
            (0, _) => TaskStatus::NotStarted,
            _ => TaskStatus::InProgress,
        };
    }

    /// Sets the status and keeps the progress in step: done is 100, not
    /// started 0; leaving done goes back below 100.
    pub fn set_status(&mut self, status: TaskStatus) {
        self.progress = match status {
            TaskStatus::Done => 100,
            TaskStatus::NotStarted => 0,
            _ if self.progress == 100 => 50,
            _ => self.progress,
        };
        self.status = status;
    }

    /// Ticks the task off, or back to not started when it is done.
    pub fn toggle_done(&mut self) {
        self.set_status(if self.status == TaskStatus::Done {
            TaskStatus::NotStarted
        } else {
            TaskStatus::Done
        });
    }

    /// The task with its values made valid: progress at most 100, status
    /// in step with it, bad dates and a blank description dropped.
    pub fn normalized(mut self) -> Task {
        self.start = self.start.filter(|d| is_date(d));
        self.end = self.end.filter(|d| is_date(d));
        self.description = self.description.filter(|d| !d.plain().trim().is_empty());
        let status = self.status;
        self.progress = self.progress.min(100);
        match status {
            TaskStatus::Done | TaskStatus::NotStarted => self.set_status(status),
            _ => {}
        }
        if self.progress == 100 {
            self.status = TaskStatus::Done;
        }
        self
    }
}

/// Whether `s` is a calendar date written `YYYY-MM-DD`.
pub fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |r: std::ops::Range<usize>| -> Option<u32> {
        let part = &s[r];
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (Some(y), Some(m), Some(d)) = (num(0..4), num(5..7), num(8..10)) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

impl TaskStatus {
    /// The colour of the task badge, `#RRGGBB`.
    pub fn color(self) -> &'static str {
        match self {
            TaskStatus::NotStarted => "#8C8C8C",
            TaskStatus::InProgress => "#539BF5",
            TaskStatus::Blocked => "#E5534B",
            TaskStatus::Done => "#57AB5A",
        }
    }
}

/// The filled part of a task badge round `(cx, cy)` of radius `r` for
/// `progress` percent: a wedge clockwise from the top, as a polygon
/// (empty at 0, the whole disc at 100).
pub fn task_pie((cx, cy): (f32, f32), r: f32, progress: u8) -> Vec<(f32, f32)> {
    let f = f32::from(progress.min(100)) / 100.0;
    if f <= 0.0 {
        return Vec::new();
    }
    let steps = ((f * 48.0).ceil() as usize).max(2);
    let mut points = Vec::with_capacity(steps + 2);
    if f < 1.0 {
        points.push((cx, cy));
    }
    for i in 0..=steps {
        let a = std::f32::consts::TAU * f * i as f32 / steps as f32;
        points.push((cx + r * a.sin(), cy - r * a.cos()));
    }
    points
}

/// The tick drawn on a done task's badge in the box `(x, y, side)`, as
/// a polyline.
pub fn task_tick((x, y): (f32, f32), side: f32) -> [(f32, f32); 3] {
    [
        (x + side * 0.28, y + side * 0.52),
        (x + side * 0.44, y + side * 0.68),
        (x + side * 0.73, y + side * 0.36),
    ]
}

/// A task as shown: its own values, or for a parent task the values
/// rolled up from its subtasks.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskView {
    pub progress: u8,
    pub status: TaskStatus,
    pub start: Option<String>,
    pub end: Option<String>,
    /// How many subtasks it has (0 for a plain task).
    pub subtasks: usize,
}

impl TaskView {
    pub fn is_parent(&self) -> bool {
        self.subtasks > 0
    }
}

/// The task view of every task topic on `page`, by id. A parent task's
/// progress is the average of its subtasks'; it is done when they all
/// are, blocked when one is, in progress when one has started; its dates
/// run from its subtasks' earliest start to their latest end (its own
/// where they have none).
pub fn task_views(page: &Page) -> HashMap<Uuid, TaskView> {
    let mut tasks: HashMap<Uuid, &Task> = HashMap::new();
    let mut kids: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for s in page.layers.iter().flat_map(|l| &l.objects) {
        let Some(t) = &s.topic else { continue };
        let Some(task) = &t.task else { continue };
        tasks.insert(s.id, task);
        if let (Some(parent), None) = (t.parent, t.summary) {
            kids.entry(parent).or_default().push(s.id);
        }
    }
    let mut out = HashMap::new();
    for &id in tasks.keys() {
        view_of(id, &tasks, &kids, &mut out, 0);
    }
    out
}

/// Deepest subtask nesting rolled up; deeper tasks show their own values.
const MAX_DEPTH: usize = 256;

fn view_of(
    id: Uuid,
    tasks: &HashMap<Uuid, &Task>,
    kids: &HashMap<Uuid, Vec<Uuid>>,
    out: &mut HashMap<Uuid, TaskView>,
    depth: usize,
) -> Option<TaskView> {
    if let Some(v) = out.get(&id) {
        return Some(v.clone());
    }
    let task = tasks.get(&id)?;
    let own = TaskView {
        progress: task.progress.min(100),
        status: task.status,
        start: task.start.clone(),
        end: task.end.clone(),
        subtasks: 0,
    };
    let subs: Vec<TaskView> = match kids.get(&id) {
        Some(children) if depth < MAX_DEPTH => children
            .iter()
            .filter_map(|c| view_of(*c, tasks, kids, out, depth + 1))
            .collect(),
        _ => Vec::new(),
    };
    let view = if subs.is_empty() {
        own
    } else {
        let total: u32 = subs.iter().map(|v| u32::from(v.progress)).sum();
        let progress = (total as f32 / subs.len() as f32).round() as u8;
        let status = if subs.iter().all(|v| v.status == TaskStatus::Done) {
            TaskStatus::Done
        } else if subs.iter().any(|v| v.status == TaskStatus::Blocked) {
            TaskStatus::Blocked
        } else if progress > 0 || subs.iter().any(|v| v.status != TaskStatus::NotStarted) {
            TaskStatus::InProgress
        } else {
            TaskStatus::NotStarted
        };
        let start = subs
            .iter()
            .filter_map(|v| v.start.clone())
            .min()
            .or(own.start);
        let end = subs.iter().filter_map(|v| v.end.clone()).max().or(own.end);
        TaskView {
            progress,
            status,
            start,
            end,
            subtasks: subs.len(),
        }
    };
    out.insert(id, view.clone());
    Some(view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Page;
    use crate::geometry::Frame;
    use crate::mindmap::TopicProps;
    use crate::shape::Shape;

    fn topic(props: TopicProps, task: Option<Task>) -> Shape {
        let mut s = Shape::new_topic(Frame::default());
        let mut props = props;
        props.task = task;
        s.topic = Some(props);
        s
    }

    fn leaf(progress: u8, status: TaskStatus) -> Task {
        Task {
            progress,
            status,
            ..Task::default()
        }
    }

    #[test]
    fn pie_grows_clockwise_from_the_top() {
        assert!(task_pie((0., 0.), 10., 0).is_empty());
        let quarter = task_pie((0., 0.), 10., 25);
        assert_eq!(quarter[0], (0., 0.));
        assert!((quarter[1].1 + 10.).abs() < 1e-4);
        let last = quarter.last().unwrap();
        assert!((last.0 - 10.).abs() < 1e-3 && last.1.abs() < 1e-3);
        // Whole: no centre point.
        assert_ne!(task_pie((0., 0.), 10., 100)[0], (0., 0.));
    }

    #[test]
    fn plain_and_rich_descriptions_load() {
        let t: Task = serde_json::from_str(r#"{"description":"Ship it"}"#).unwrap();
        assert_eq!(t.description.as_ref().unwrap().plain(), "Ship it");
        let back: Task = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back, t);
        let blank: Task = serde_json::from_str(r#"{"description":"  "}"#).unwrap();
        assert_eq!(blank.description, None);
    }

    #[test]
    fn dates_are_checked() {
        assert!(is_date("2026-02-28"));
        assert!(is_date("2024-02-29"));
        assert!(!is_date("2026-02-29"));
        assert!(!is_date("2026-13-01"));
        assert!(!is_date("2026-1-01"));
        assert!(!is_date("+026-01-01"));
        assert!(!is_date("2026-04-31"));
    }

    #[test]
    fn progress_and_status_stay_in_step() {
        let mut t = Task::default();
        t.set_progress(40);
        assert_eq!(t.status, TaskStatus::InProgress);
        t.set_progress(250);
        assert_eq!((t.progress, t.status), (100, TaskStatus::Done));
        t.set_status(TaskStatus::InProgress);
        assert_eq!(t.progress, 50);
        t.set_status(TaskStatus::Blocked);
        t.set_progress(0);
        assert_eq!(t.status, TaskStatus::Blocked);
        t.toggle_done();
        assert_eq!((t.progress, t.status), (100, TaskStatus::Done));
        t.toggle_done();
        assert_eq!((t.progress, t.status), (0, TaskStatus::NotStarted));
    }

    #[test]
    fn normalizing_fixes_bad_values() {
        let t = Task {
            start: Some("2026-02-30".into()),
            end: Some("2026-03-01".into()),
            progress: 200,
            status: TaskStatus::NotStarted,
            ..Task::default()
        }
        .normalized();
        assert_eq!(t.start, None);
        assert_eq!(t.end.as_deref(), Some("2026-03-01"));
        assert_eq!((t.progress, t.status), (0, TaskStatus::NotStarted));
        let t = leaf(100, TaskStatus::InProgress).normalized();
        assert_eq!(t.status, TaskStatus::Done);
    }

    #[test]
    fn parents_roll_up_their_subtasks() {
        let mut central = topic(TopicProps::central(Uuid::nil()), Some(Task::default()));
        let c = central.id;
        central.topic.as_mut().unwrap().map = c;
        let a = topic(TopicProps::child(c, c, 0.), Some(Task::default()));
        let mut a1_task = leaf(100, TaskStatus::Done);
        a1_task.start = Some("2026-01-05".into());
        let a1 = topic(TopicProps::child(c, a.id, 0.), Some(a1_task));
        let mut a2_task = leaf(50, TaskStatus::InProgress);
        a2_task.end = Some("2026-02-01".into());
        let a2 = topic(TopicProps::child(c, a.id, 1.), Some(a2_task));
        let b = topic(
            TopicProps::child(c, c, 1.),
            Some(leaf(0, TaskStatus::Blocked)),
        );
        // Not a task: left out of the roll-up.
        let plain = topic(TopicProps::child(c, c, 2.), None);
        let ids = (c, a.id, a1.id, b.id, plain.id);
        let mut page = Page::new("P");
        page.layers[0].objects = vec![central, a, a1, a2, b, plain];
        let views = task_views(&page);
        let (c, a, a1, b, plain) = ids;
        assert_eq!(views[&a1].subtasks, 0);
        let va = &views[&a];
        assert_eq!(
            (va.progress, va.status, va.subtasks),
            (75, TaskStatus::InProgress, 2)
        );
        assert_eq!(va.start.as_deref(), Some("2026-01-05"));
        assert_eq!(va.end.as_deref(), Some("2026-02-01"));
        let vc = &views[&c];
        // (75 + 0) / 2, rounded; a blocked subtask blocks the parent.
        assert_eq!((vc.progress, vc.status), (38, TaskStatus::Blocked));
        assert_eq!(views[&b].status, TaskStatus::Blocked);
        assert!(!views.contains_key(&plain));
    }

    #[test]
    fn all_done_subtasks_make_a_done_parent() {
        let mut c = topic(TopicProps::central(Uuid::nil()), Some(Task::default()));
        let id = c.id;
        c.topic.as_mut().unwrap().map = id;
        let x = topic(
            TopicProps::child(id, id, 0.),
            Some(leaf(100, TaskStatus::Done)),
        );
        let mut page = Page::new("P");
        page.layers[0].objects = vec![c, x];
        let v = &task_views(&page)[&id];
        assert_eq!((v.progress, v.status), (100, TaskStatus::Done));
    }
}
