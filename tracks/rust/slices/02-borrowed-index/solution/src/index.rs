//! In-memory index: key → latest value.

use std::collections::HashMap;

/// The latest value for each key.
#[derive(Debug, Default)]
pub struct Index {
    map: HashMap<String, Vec<u8>>,
}

impl Index {
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the value for `key` without copying it.
    pub fn get(&self, key: &str) -> Option<&[u8]> {
        self.map.get(key).map(Vec::as_slice)
    }

    /// Store `value`, returning the value it replaced.
    pub fn insert(&mut self, key: String, value: Vec<u8>) -> Option<Vec<u8>> {
        self.map.insert(key, value)
    }

    /// Remove `key`, handing its value back to the caller.
    pub fn remove(&mut self, key: &str) -> Option<Vec<u8>> {
        self.map.remove(key)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
