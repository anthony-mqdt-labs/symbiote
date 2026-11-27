//! Key validation.

/// Longest key the log accepts, in bytes.
pub const MAX_KEY_LEN: usize = 256;

/// A key is valid when it is non-empty, at most `MAX_KEY_LEN` bytes, and contains no `\n`.
pub fn is_valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= MAX_KEY_LEN && !key.contains('\n')
}
