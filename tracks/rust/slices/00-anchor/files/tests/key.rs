use kvlog::key::{is_valid_key, MAX_KEY_LEN};

#[test]
fn accepts_normal_keys() {
    assert!(is_valid_key("user:42"));
    assert!(is_valid_key(&"k".repeat(MAX_KEY_LEN)));
}

#[test]
fn rejects_bad_keys() {
    assert!(!is_valid_key(""));
    assert!(!is_valid_key("a\nb"));
    assert!(!is_valid_key(&"k".repeat(MAX_KEY_LEN + 1)));
}

#[test]
fn limit_is_bytes_not_chars() {
    let accented = "é".repeat(200); // 200 chars, 400 bytes
    assert!(!is_valid_key(&accented));
}
