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
    let mut out = Recovered {
        records: Vec::new(),
        valid_len: 0,
        corrupt: false,
    };
    while let Some(hdr) = buf.get(out.valid_len..out.valid_len + HEADER_LEN) {
        let key_len = u32::from_le_bytes(hdr[0..4].try_into().unwrap()) as usize;
        let val_len = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
        if key_len + val_len > MAX_RECORD {
            out.corrupt = true;
            break;
        }
        match Record::decode(&buf[out.valid_len..]) {
            Ok((rec, used)) => {
                out.records.push(rec);
                out.valid_len += used;
            }
            Err(DecodeError::Truncated) => break,
            Err(DecodeError::InvalidKey) => {
                out.corrupt = true;
                break;
            }
        }
    }
    out
}
