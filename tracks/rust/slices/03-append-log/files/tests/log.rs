use kvlog::log::{replay, LogWriter};
use kvlog::record::Record;
use std::io::ErrorKind;

fn rec(k: &str, v: &[u8]) -> Record {
    Record {
        key: k.to_string(),
        value: v.to_vec(),
    }
}

#[test]
fn append_returns_offsets_and_replay_roundtrips() {
    let mut log = LogWriter::new(Vec::new());
    let a = rec("a", b"1");
    let b = rec("bb", b"22");
    assert_eq!(log.append(&a).unwrap(), 0);
    assert_eq!(log.append(&b).unwrap(), a.encode().len() as u64);
    let bytes = log.into_inner();
    assert_eq!(replay(&bytes[..]).unwrap(), vec![a, b]);
}

#[test]
fn empty_log_replays_to_nothing() {
    assert_eq!(replay(&b""[..]).unwrap(), vec![]);
}

#[test]
fn torn_tail_is_unexpected_eof() {
    let mut bytes = rec("a", b"1").encode();
    bytes.extend(&rec("b", b"2").encode()[..5]);
    assert_eq!(
        replay(&bytes[..]).unwrap_err().kind(),
        ErrorKind::UnexpectedEof
    );
}
