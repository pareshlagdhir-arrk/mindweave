//! Consistency checks of a document: what the app relies on.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use uuid::Uuid;

use crate::document::Document;
use crate::library::LibraryRegistry;

/// How serious a [`Problem`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The document is inconsistent; the app may drop or misplace things.
    Error,
    /// Allowed, but probably not intended (a shape from a library that is
    /// not installed is drawn as a placeholder).
    Warning,
}

/// Something wrong with a document.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Problem {
    pub severity: Severity,
    pub page: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<Uuid>,
    pub message: String,
}

/// The problems of `doc`; `libs`, when given, are the installed
/// libraries its shapes should come from.
pub fn validate(doc: &Document, libs: Option<&LibraryRegistry>) -> Vec<Problem> {
    let mut out = Vec::new();
    let mut seen: HashSet<Uuid> = HashSet::new();
    for (pi, page) in doc.pages.iter().enumerate() {
        let mut problem = |severity, object, message: String| {
            out.push(Problem {
                severity,
                page: pi,
                object,
                message,
            })
        };
        if page.layers.is_empty() {
            problem(Severity::Error, None, "The page has no layers.".into());
        }
        for id in std::iter::once(page.id).chain(page.layers.iter().map(|l| l.id)) {
            if !seen.insert(id) {
                problem(Severity::Error, None, format!("Id {id} is used twice."));
            }
        }
        let shapes: Vec<(usize, &crate::shape::Shape)> = page
            .layers
            .iter()
            .enumerate()
            .flat_map(|(li, l)| l.objects.iter().map(move |s| (li, s)))
            .collect();
        let by_id: HashMap<Uuid, (usize, &crate::shape::Shape)> =
            shapes.iter().map(|(li, s)| (s.id, (*li, *s))).collect();
        for (li, s) in &shapes {
            let id = Some(s.id);
            if !seen.insert(s.id) {
                problem(Severity::Error, id, format!("Id {} is used twice.", s.id));
            }
            let f = &s.frame;
            if ![f.x, f.y, f.w, f.h, f.rotation]
                .iter()
                .all(|v| v.is_finite())
                || f.w < 0.0
                || f.h < 0.0
            {
                problem(
                    Severity::Error,
                    id,
                    "The frame has an invalid number.".into(),
                );
            }
            if let Some(line) = &s.line {
                for (end, target) in [("start", line.start), ("end", line.end)] {
                    let Some(t) = target else { continue };
                    match by_id.get(&t) {
                        None => problem(
                            Severity::Error,
                            id,
                            format!(
                                "The line's {end} is attached to {t}, which is not on this page."
                            ),
                        ),
                        Some((_, ts)) if ts.is_line() => problem(
                            Severity::Error,
                            id,
                            format!("The line's {end} is attached to another line ({t})."),
                        ),
                        _ => {}
                    }
                }
            }
            if let Some(t) = &s.topic {
                match by_id.get(&t.map) {
                    Some((ml, m)) if m.topic.as_ref().is_some_and(|mt| mt.is_central()) => {
                        if ml != li {
                            problem(
                                Severity::Error,
                                id,
                                "The topic is on another layer than its central topic.".into(),
                            );
                        }
                    }
                    _ => problem(
                        Severity::Error,
                        id,
                        format!(
                            "The topic's map {} has no central topic on this page.",
                            t.map
                        ),
                    ),
                }
                if let Some(p) = t.parent {
                    match by_id.get(&p).and_then(|(_, ps)| ps.topic.as_ref()) {
                        Some(pt) if pt.map == t.map => {}
                        _ => problem(
                            Severity::Error,
                            id,
                            format!("The topic's parent {p} is not a topic of the same map."),
                        ),
                    }
                } else if t.map != s.id {
                    problem(
                        Severity::Error,
                        id,
                        "A topic without a parent must be its map's central topic.".into(),
                    );
                }
                if let Some((a, b)) = t.summary {
                    for m in [a, b] {
                        let ok = by_id
                            .get(&m)
                            .and_then(|(_, ms)| ms.topic.as_ref())
                            .is_some_and(|mt| mt.parent == t.parent);
                        if !ok {
                            problem(Severity::Error, id, format!("The summary covers {m}, which is not a sibling under its parent."));
                        }
                    }
                }
            }
            if let Some(b) = &s.boundary {
                if b.members.is_empty() {
                    problem(
                        Severity::Error,
                        id,
                        "The boundary encloses no topics.".into(),
                    );
                }
                for m in &b.members {
                    if by_id.get(m).is_none_or(|(_, ms)| ms.topic.is_none()) {
                        problem(
                            Severity::Error,
                            id,
                            format!(
                                "The boundary encloses {m}, which is not a topic on this page."
                            ),
                        );
                    }
                }
            }
            if let Some(a) = s.artboard {
                match by_id.get(&a) {
                    Some((al, ab)) if ab.is_artboard() && al == li => {}
                    _ => problem(Severity::Error, id, format!("The object belongs to artboard {a}, which is not an artboard on its layer.")),
                }
            }
            if let Some(libs) = libs {
                let builtin_kind = s.library == crate::shape::TEXT_LIBRARY;
                if !builtin_kind && libs.shape(&s.library, &s.shape).is_none() {
                    problem(
                        Severity::Warning,
                        id,
                        format!("Shape `{}` of library `{}` is not installed; it shows as a placeholder.", s.shape, s.library),
                    );
                }
            }
        }
        // Groups: whole, on one layer, nested the same way in every member.
        let mut groups: HashMap<Uuid, Vec<(usize, &crate::shape::Shape, usize)>> = HashMap::new();
        for (li, s) in &shapes {
            let unique: HashSet<&Uuid> = s.groups.iter().collect();
            if unique.len() != s.groups.len() {
                problem(
                    Severity::Error,
                    Some(s.id),
                    "The object lists a group twice.".into(),
                );
            }
            for (depth, g) in s.groups.iter().enumerate() {
                groups.entry(*g).or_default().push((*li, *s, depth));
            }
        }
        let mut group_ids: Vec<&Uuid> = groups.keys().collect();
        group_ids.sort();
        for g in group_ids {
            let members = &groups[g];
            let (first_layer, first, depth) = members[0];
            if members.iter().any(|(l, _, _)| *l != first_layer) {
                problem(
                    Severity::Error,
                    None,
                    format!("Group {g} has members on different layers."),
                );
            }
            if members.len() == 1 {
                problem(
                    Severity::Warning,
                    Some(first.id),
                    format!("Group {g} has only one member left; ungroup it."),
                );
            }
            let outer = &first.groups[..depth];
            if members
                .iter()
                .any(|(_, m, d)| *d != depth || m.groups[..*d] != *outer)
            {
                problem(
                    Severity::Error,
                    None,
                    format!("Group {g} is nested in different groups in different members (a dangling group id)."),
                );
            }
        }
        // Topic cycles: walking up from any topic must reach a central one.
        for (_, s) in &shapes {
            let Some(t) = &s.topic else { continue };
            let mut at = t.parent;
            let mut steps = 0;
            while let Some(p) = at {
                steps += 1;
                if steps > shapes.len() {
                    problem(
                        Severity::Error,
                        Some(s.id),
                        "The topic's parents form a cycle.".into(),
                    );
                    break;
                }
                at = by_id
                    .get(&p)
                    .and_then(|(_, ps)| ps.topic.as_ref())
                    .and_then(|pt| pt.parent);
            }
        }
    }
    if doc.pages.is_empty() {
        out.push(Problem {
            severity: Severity::Error,
            page: 0,
            object: None,
            message: "The document has no pages.".into(),
        });
    }
    out
}
