//! Implicit SRS projected from events.jsonl (ARCHITECTURE.md §2 "Implicit SRS").
//! The latest event per concept wins; nothing here writes.

use crate::model::Evidence;
use crate::util::{iso, parse_iso};
use std::collections::HashMap;

pub const LADDER: [u32; 5] = [1, 3, 7, 14, 30];
const DAY_MS: i64 = 86_400_000;

pub fn is_failure(grade: &str) -> bool {
    matches!(grade, "shaky" | "wrong")
}

/// Latest graded event per concept.
pub fn latest_by_concept(events: &[Evidence]) -> HashMap<String, Evidence> {
    let mut out: HashMap<String, Evidence> = HashMap::new();
    for e in events {
        if let (Some(c), Some(_)) = (&e.concept, &e.grade) {
            out.insert(c.clone(), e.clone());
        }
    }
    out
}

/// Success climbs one rung above the previous interval; shaky/wrong resets to 1 day.
pub fn next_interval(prev: Option<&Evidence>, grade: &str) -> u32 {
    if is_failure(grade) {
        return 1;
    }
    match prev {
        // After a reset (or first sighting) the ladder restarts at 1 day.
        Some(p) if !p.grade.as_deref().is_some_and(is_failure) => {
            let d = p.interval_days.unwrap_or(0);
            *LADDER.iter().find(|&&r| r > d).unwrap_or(&LADDER[LADDER.len() - 1])
        }
        _ => LADDER[0],
    }
}

/// Build a graded evidence line with its next due date.
pub fn grade_event(events: &[Evidence], now_ms: i64, event: &str, slice_id: Option<&str>, concept: &str, grade: &str, misconception: Option<String>) -> Evidence {
    let latest = latest_by_concept(events);
    let days = next_interval(latest.get(concept), grade);
    Evidence {
        ts: iso(now_ms),
        event: event.into(),
        slice_id: slice_id.map(str::to_string),
        concept: Some(concept.into()),
        grade: Some(grade.into()),
        misconception_shape: misconception,
        interval_days: Some(days),
        due_at: Some(iso(now_ms + days as i64 * DAY_MS)),
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Metrics {
    pub due_items: usize,
    pub open_misconceptions: usize,
    /// Due or open concepts, oldest due_at first.
    pub queue: Vec<String>,
}

pub fn metrics(events: &[Evidence], now_ms: i64) -> Metrics {
    let latest = latest_by_concept(events);
    let mut queue: Vec<(i64, String)> = vec![];
    let (mut due, mut open) = (0, 0);
    for (c, e) in &latest {
        let due_at = e.due_at.as_deref().and_then(parse_iso).unwrap_or(i64::MAX);
        let is_due = due_at <= now_ms;
        let is_open = e.grade.as_deref().is_some_and(is_failure);
        due += usize::from(is_due);
        open += usize::from(is_open);
        if is_due || is_open {
            queue.push((due_at, c.clone()));
        }
    }
    queue.sort();
    Metrics { due_items: due, open_misconceptions: open, queue: queue.into_iter().map(|(_, c)| c).collect() }
}

/// Exit criteria: stop new material while the review debt is high.
pub fn must_review(m: &Metrics) -> bool {
    m.due_items >= 3 || m.open_misconceptions >= 2
}

/// Cold Return: keep the 2 oldest due concepts now, spread the rest over days 3..=7.
pub fn stagger(events: &[Evidence], now_ms: i64) -> Vec<Evidence> {
    let latest = latest_by_concept(events);
    let m = metrics(events, now_ms);
    let due: Vec<&String> = m.queue.iter().filter(|c| latest[*c].due_at.as_deref().and_then(parse_iso).is_some_and(|t| t <= now_ms)).collect();
    due.iter()
        .skip(2)
        .enumerate()
        .map(|(i, c)| {
            let prev = &latest[*c];
            let day = 3 + (i as i64 % 5);
            Evidence { ts: iso(now_ms), event: "srs_stagger".into(), due_at: Some(iso(now_ms + day * DAY_MS)), ..prev.clone() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(events: &mut Vec<Evidence>, now: i64, concept: &str, grade: &str) {
        let e = grade_event(events, now, "drill_pass", None, concept, grade, None);
        events.push(e);
    }

    #[test]
    fn ladder_climbs_and_resets() {
        let mut evs = vec![];
        let t = 1_795_725_855_000;
        for want in [1, 3, 7, 14, 30, 30] {
            ev(&mut evs, t, "ctx", "solid");
            assert_eq!(evs.last().unwrap().interval_days, Some(want));
        }
        ev(&mut evs, t, "ctx", "shaky");
        assert_eq!(evs.last().unwrap().interval_days, Some(1));
        ev(&mut evs, t, "ctx", "solid");
        assert_eq!(evs.last().unwrap().interval_days, Some(1), "first success after a reset restarts the ladder");
    }

    #[test]
    fn metrics_and_exit_criteria() {
        let t0 = 1_795_725_855_000;
        let mut evs = vec![];
        ev(&mut evs, t0, "a", "solid");
        ev(&mut evs, t0, "b", "wrong");
        ev(&mut evs, t0, "c", "shaky");
        let now = t0 + DAY_MS / 2;
        let m = metrics(&evs, now);
        assert_eq!((m.due_items, m.open_misconceptions), (0, 2));
        assert!(must_review(&m));
        ev(&mut evs, now, "b", "solid"); // clears the misconception
        let m = metrics(&evs, now);
        assert_eq!(m.open_misconceptions, 1);
        assert!(!must_review(&m));
        let later = t0 + 2 * DAY_MS;
        let m = metrics(&evs, later);
        assert_eq!(m.due_items, 3);
        assert_eq!(m.queue[0], "a", "oldest due first");
    }

    #[test]
    fn stagger_keeps_two() {
        let t0 = 1_795_725_855_000;
        let mut evs = vec![];
        for c in ["a", "b", "c", "d", "e"] {
            ev(&mut evs, t0, c, "solid");
        }
        let now = t0 + 10 * DAY_MS;
        let moved = stagger(&evs, now);
        assert_eq!(moved.len(), 3);
        evs.extend(moved);
        assert_eq!(metrics(&evs, now).due_items, 2);
    }
}
