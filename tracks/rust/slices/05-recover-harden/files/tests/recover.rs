use kvlog::record::Record;
use kvlog::recover::{recover, MAX_RECORD};

fn enc(k: &str, v: &str) -> Vec<u8> {
    Record {
        key: k.to_string(),
        value: v.as_bytes().to_vec(),
    }
    .encode()
}

#[test]
fn clean_log_recovers_fully() {
    let mut buf = enc("a", "1");
    buf.extend(enc("b", "2"));
    let r = recover(&buf);
    assert_eq!(r.records.len(), 2);
    assert_eq!(r.valid_len, buf.len());
    assert!(!r.corrupt);
}

#[test]
fn torn_tail_is_truncated_not_corrupt() {
    let first = enc("a", "1");
    let mut buf = first.clone();
    buf.extend(&enc("b", "22")[..9]);
    let r = recover(&buf);
    assert_eq!(r.records.len(), 1);
    assert_eq!(r.valid_len, first.len());
    assert!(!r.corrupt);
}

#[test]
fn impossible_length_is_corruption() {
    let first = enc("a", "1");
    let mut buf = first.clone();
    buf.extend(1u32.to_le_bytes());
    buf.extend(((MAX_RECORD as u32) * 3).to_le_bytes());
    buf.extend(b"0123456789");
    let r = recover(&buf);
    assert_eq!(r.records.len(), 1);
    assert_eq!(r.valid_len, first.len());
    assert!(r.corrupt);
}

#[test]
fn bad_key_is_corruption() {
    let mut buf = enc("a", "1");
    let good = buf.len();
    buf.extend(1u32.to_le_bytes());
    buf.extend(0u32.to_le_bytes());
    buf.push(0xff);
    buf.extend(enc("c", "3"));
    let r = recover(&buf);
    assert_eq!((r.records.len(), r.valid_len, r.corrupt), (1, good, true));
}

#[test]
fn empty_input() {
    let r = recover(&[]);
    assert_eq!((r.records.len(), r.valid_len, r.corrupt), (0, 0, false));
}
