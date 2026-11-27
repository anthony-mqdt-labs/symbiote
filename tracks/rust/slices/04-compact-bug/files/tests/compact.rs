use kvlog::compact::compact;
use kvlog::record::Record;

fn rec(k: &str, v: &str) -> Record {
    Record {
        key: k.to_string(),
        value: v.as_bytes().to_vec(),
    }
}

#[test]
fn newest_value_wins_in_last_write_order() {
    let log = [rec("a", "1"), rec("b", "2"), rec("a", "3")];
    assert_eq!(compact(&log), vec![rec("b", "2"), rec("a", "3")]);
}

#[test]
fn tombstone_removes_the_key() {
    let log = [rec("a", "1"), rec("b", "2"), rec("a", "")];
    assert_eq!(compact(&log), vec![rec("b", "2")]);
}

#[test]
fn write_after_tombstone_revives_the_key() {
    let log = [rec("a", "1"), rec("a", ""), rec("a", "9")];
    assert_eq!(compact(&log), vec![rec("a", "9")]);
}
