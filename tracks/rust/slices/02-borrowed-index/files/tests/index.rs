use kvlog::index::Index;

#[test]
fn get_borrows_the_stored_bytes() {
    let mut idx = Index::new();
    assert!(idx.is_empty());
    assert_eq!(idx.insert("a".into(), b"1".to_vec()), None);
    let v: Option<&[u8]> = idx.get("a");
    assert_eq!(v, Some(&b"1"[..]));
    // Two reads point at the same allocation: nothing was copied.
    assert_eq!(
        idx.get("a").unwrap().as_ptr(),
        idx.get("a").unwrap().as_ptr()
    );
    assert_eq!(idx.get("missing"), None);
}

#[test]
fn insert_and_remove_move_ownership_back() {
    let mut idx = Index::new();
    idx.insert("k".into(), b"old".to_vec());
    assert_eq!(
        idx.insert("k".into(), b"new".to_vec()),
        Some(b"old".to_vec())
    );
    assert_eq!(idx.len(), 1);
    assert_eq!(idx.remove("k"), Some(b"new".to_vec()));
    assert_eq!(idx.remove("k"), None);
    assert!(idx.is_empty());
}
