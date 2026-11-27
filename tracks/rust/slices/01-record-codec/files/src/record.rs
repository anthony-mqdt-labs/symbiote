//! On-disk record framing: `[key_len: u32 LE][val_len: u32 LE][key bytes][value bytes]`.

/// Bytes in the fixed header.
pub const HEADER_LEN: usize = 8;

/// One key-value pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub key: String,
    pub value: Vec<u8>,
}

/// Why a buffer could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Fewer bytes than the header or its lengths promise.
    Truncated,
    /// The key bytes are not valid UTF-8.
    InvalidKey,
}

impl Record {
    /// Serialize to the on-disk framing.
    pub fn encode(&self) -> Vec<u8> {
        todo!("header (two u32 LE lengths), then key bytes, then value bytes")
    }

    /// Decode one record from the front of `buf`, returning it and the bytes consumed.
    /// Never panics, whatever `buf` contains.
    pub fn decode(buf: &[u8]) -> Result<(Record, usize), DecodeError> {
        let _ = buf;
        todo!("bounds-checked decode")
    }
}
