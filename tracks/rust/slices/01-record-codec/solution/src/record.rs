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
        let mut out = Vec::with_capacity(HEADER_LEN + self.key.len() + self.value.len());
        out.extend_from_slice(&(self.key.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.value.len() as u32).to_le_bytes());
        out.extend_from_slice(self.key.as_bytes());
        out.extend_from_slice(&self.value);
        out
    }

    /// Decode one record from the front of `buf`, returning it and the bytes consumed.
    /// Never panics, whatever `buf` contains.
    pub fn decode(buf: &[u8]) -> Result<(Record, usize), DecodeError> {
        let hdr = buf.get(..HEADER_LEN).ok_or(DecodeError::Truncated)?;
        let key_len = u32::from_le_bytes(hdr[0..4].try_into().unwrap()) as usize;
        let val_len = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
        let key_end = HEADER_LEN
            .checked_add(key_len)
            .ok_or(DecodeError::Truncated)?;
        let end = key_end.checked_add(val_len).ok_or(DecodeError::Truncated)?;
        let key = buf.get(HEADER_LEN..key_end).ok_or(DecodeError::Truncated)?;
        let value = buf.get(key_end..end).ok_or(DecodeError::Truncated)?;
        let key = std::str::from_utf8(key).map_err(|_| DecodeError::InvalidKey)?;
        Ok((
            Record {
                key: key.to_string(),
                value: value.to_vec(),
            },
            end,
        ))
    }
}
