use machine_god_core::SessionId;
use machine_god_native::{MAX_NATIVE_SESSION_CATALOG_CURSOR_BYTES, NativeSessionCatalogCursor};

#[test]
fn portable_cursor_round_trips_known_unknown_and_native_id_domain() {
    for time in [
        None,
        Some(i64::MIN),
        Some(-1),
        Some(0),
        Some(1),
        Some(i64::MAX),
    ] {
        for id in ["ordinary-id", "native:colon:id", "..", "_"] {
            let cursor = NativeSessionCatalogCursor::new(time, SessionId::new(id).unwrap());
            assert_eq!(cursor.updated_at_ms(), time);
            assert_eq!(cursor.id().as_str(), id);
            assert_eq!(
                NativeSessionCatalogCursor::parse(&cursor.to_string()).unwrap(),
                cursor
            );
            assert_eq!(format!("{cursor:?}"), "NativeSessionCatalogCursor { .. }");
        }
    }
    assert_eq!(
        NativeSessionCatalogCursor::new(Some(20), SessionId::new("session-a").unwrap()).to_string(),
        "v1:20:session-a"
    );
    assert_eq!(
        NativeSessionCatalogCursor::new(None, SessionId::new("old").unwrap()).to_string(),
        "v1:unknown:old"
    );
}

#[test]
fn parser_rejects_noncanonical_timestamps_and_unbounded_or_invalid_ids_redacted() {
    for raw in [
        "",
        "v2:20:session-a",
        "v1:020:session-a",
        "v1:+20:session-a",
        "v1:-0:session-a",
        "v1: 20:session-a",
        "v1:20.0:session-a",
        "v1:9223372036854775808:id",
        "v1:-9223372036854775809:id",
        "v1:Unknown:id",
        "v1::id",
        "v1:0:",
        "v1:0:../unsafe",
        "v1:0:private\nvalue",
    ] {
        let error = NativeSessionCatalogCursor::parse(raw).unwrap_err();
        assert_eq!(
            error.to_string(),
            "native session catalog cursor is invalid"
        );
        assert_eq!(format!("{error:?}"), "NativeSessionCatalogCursorError");
    }
    assert!(
        NativeSessionCatalogCursor::parse(&"x".repeat(MAX_NATIVE_SESSION_CATALOG_CURSOR_BYTES + 1))
            .is_err()
    );
    assert!(NativeSessionCatalogCursor::parse(&format!("v1:0:{}", "a".repeat(129))).is_err());
    let max_id = "a".repeat(128);
    let encoded = format!("v1:{}:{max_id}", i64::MIN);
    assert_eq!(
        NativeSessionCatalogCursor::parse(&encoded)
            .unwrap()
            .to_string(),
        encoded
    );
}
