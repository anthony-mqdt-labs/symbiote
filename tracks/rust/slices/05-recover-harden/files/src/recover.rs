//! Crash recovery for a log that may end mid-append or contain garbage.

use crate::record::{DecodeError, Record, HEADER_LEN};

/// No legitimate record declares more than this many key + value bytes.
pub const MAX_RECORD: usize = 1 << 20;

/// What survived.
#[derive(Debug, PartialEq, Eq)]
pub struct Recovered {
    pub records: Vec<Record>,
    /// Length of the longest valid prefix; truncate the file here.
    pub valid_len: usize,
    /// True when bytes that could never have been written were found (not just a torn tail).
    pub corrupt: bool,
}

/// Keep every complete record, stop at the first torn or corrupt one.
pub fn recover(buf: &[u8]) -> Recovered {
    let _ = (buf, HEADER_LEN, MAX_RECORD, DecodeError::Truncated);
    todo!()
}
