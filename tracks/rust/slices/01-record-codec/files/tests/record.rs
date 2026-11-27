use kvlog::record::{DecodeError, Record, HEADER_LEN};

fn rec(k: &str, v: &[u8]) -> Record {
    Record {
        key: k.to_string(),
        value: v.to_vec(),
    }
}

#[test]
fn roundtrip() {
    let r = rec("user:1", b"alice");
    let bytes = r.encode();
    assert_eq!(bytes.len(), HEADER_LEN + 6 + 5);
    assert_eq!(&bytes[..4], &6u32.to_le_bytes());
    assert_eq!(Record::decode(&bytes), Ok((r, bytes.len())));
}

#[test]
fn decodes_back_to_back_records() {
    let mut buf = rec("a", b"1").encode();
    buf.extend(rec("bb", b"").encode());
    let (first, used) = Record::decode(&buf).unwrap();
    assert_eq!(first, rec("a", b"1"));
    let (second, used2) = Record::decode(&buf[used..]).unwrap();
    assert_eq!(second, rec("bb", b""));
    assert_eq!(used + used2, buf.len());
}

#[test]
fn truncated_input_is_an_error_not_a_panic() {
    let full = rec("key", b"value").encode();
    for cut in 0..full.len() {
        assert_eq!(
            Record::decode(&full[..cut]),
            Err(DecodeError::Truncated),
            "cut at {cut}"
        );
    }
}

#[test]
fn untrusted_lengths_cannot_panic() {
    let mut buf = u32::MAX.to_le_bytes().to_vec();
    buf.extend(u32::MAX.to_le_bytes());
    buf.extend(b"tail");
    assert_eq!(Record::decode(&buf), Err(DecodeError::Truncated));
}

#[test]
fn non_utf8_key_is_rejected() {
    let mut buf = 2u32.to_le_bytes().to_vec();
    buf.extend(0u32.to_le_bytes());
    buf.extend([0xff, 0xfe]);
    assert_eq!(Record::decode(&buf), Err(DecodeError::InvalidKey));
}
