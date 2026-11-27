//! Append-only log over any `Write` sink; replay from any `Read` source.

use crate::record::{DecodeError, Record};
use std::io::{self, Read, Write};

/// Appends encoded records to a sink (a File in production, a `Vec<u8>` in tests).
pub struct LogWriter<W: Write> {
    sink: W,
    offset: u64,
}

impl<W: Write> LogWriter<W> {
    pub fn new(sink: W) -> Self {
        Self { sink, offset: 0 }
    }

    /// Append one record, returning the byte offset where it starts.
    pub fn append(&mut self, rec: &Record) -> io::Result<u64> {
        let _ = (rec, self.offset);
        todo!("write the encoded record, advance the offset")
    }

    pub fn into_inner(self) -> W {
        self.sink
    }
}

/// Read every record from `r`. Clean end of input is Ok; a torn final record is
/// `ErrorKind::UnexpectedEof`; a non-UTF-8 key is `ErrorKind::InvalidData`.
pub fn replay<R: Read>(r: R) -> io::Result<Vec<Record>> {
    let _ = (r, DecodeError::Truncated);
    todo!()
}
