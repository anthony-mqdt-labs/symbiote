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
        let _ = (&self.map, key);
        todo!("borrow from self.map")
    }

    /// Store `value`, returning the value it replaced.
    pub fn insert(&mut self, key: String, value: Vec<u8>) -> Option<Vec<u8>> {
        let _ = (key, value);
        todo!()
    }

    /// Remove `key`, handing its value back to the caller.
    pub fn remove(&mut self, key: &str) -> Option<Vec<u8>> {
        let _ = key;
        todo!()
    }

    pub fn len(&self) -> usize {
        todo!()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
