use super::*;

#[test]
fn test_seedr_not_enough_space_response() -> Result<(), Box<dyn std::error::Error>> {
    let json = r#"{"result":false,"reason_phrase":"not_enough_space_added_to_wishlist"}"#;
    let res: GenericResponse = serde_json::from_str(json)?;
    assert!(res
        .reason_phrase
        .is_some_and(|r| r.contains("not_enough_space")));
    Ok(())
}

#[test]
fn test_extract_btih_hash() {
    let magnet = "magnet:?xt=urn:btih:dd04eca369c6a67f56497bb5902eaef91281f191&dn=Test";
    assert_eq!(
        extract_btih_hash(magnet).as_deref(),
        Some("dd04eca369c6a67f56497bb5902eaef91281f191")
    );
}

#[test]
fn test_folder_candidate_selection_prefers_largest_size() {
    let f1 = SeedrFolder {
        id: 1,
        name: "Test Series".into(),
        size: Some(28_000_000),
    };
    let f2 = SeedrFolder {
        id: 2,
        name: "Test Series".into(),
        size: Some(854_000_000),
    };
    let candidates = vec![&f1, &f2];
    let best = candidates.into_iter().max_by_key(|f| f.size);
    assert_eq!(best.map(|f| f.id), Some(2));
    assert_eq!(best.and_then(|f| f.size), Some(854_000_000));
}
