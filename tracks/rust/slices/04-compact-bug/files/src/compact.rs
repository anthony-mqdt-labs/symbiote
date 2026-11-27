//! Compaction: rewrite the log keeping only what a reader would see.

use crate::record::Record;
use std::collections::HashMap;

/// Keep only the newest record for each key, ordered by when each key was last written.
/// An empty value is a tombstone: the key was deleted and must not appear in the output.
pub fn compact(records: &[Record]) -> Vec<Record> {
    let mut latest: HashMap<&str, usize> = HashMap::new();
    for (i, r) in records.iter().enumerate() {
        latest.entry(r.key.as_str()).or_insert(i);
    }
    let mut keep: Vec<usize> = latest.into_values().collect();
    keep.sort_unstable();
    keep.into_iter()
        .map(|i| records[i].clone())
        .filter(|r| !r.value.is_empty())
        .collect()
}
