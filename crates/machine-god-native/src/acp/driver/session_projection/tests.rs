use super::*;
use crate::{
    FileSessionStore, NativeSessionCatalog, NativeSessionCatalogQuery, NativeSessionMetadata,
    NativeSessionOrigin,
};
use machine_god_core::{
    Message, Role, SessionId, SessionIncarnationId, SessionRecord, SessionStore,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[test]
fn utc_known_epoch_leap_century_and_proleptic_dates() {
    for (time, expected) in [
        (0, "1970-01-01T00:00:00Z"),
        (999, "1970-01-01T00:00:00Z"),
        (-1, "1969-12-31T23:59:59Z"),
        (-1001, "1969-12-31T23:59:58Z"),
        (1_700_000_000_123, "2023-11-14T22:13:20Z"),
        (-62_167_219_200_000, "0000-01-01T00:00:00Z"),
        (-62_135_596_800_000, "0001-01-01T00:00:00Z"),
        (-12_219_292_800_000, "1582-10-15T00:00:00Z"),
        (-2_203_977_600_000, "1900-02-28T00:00:00Z"),
        (-2_203_891_200_000, "1900-03-01T00:00:00Z"),
        (951_782_400_000, "2000-02-29T00:00:00Z"),
        (4_107_542_400_000, "2100-03-01T00:00:00Z"),
        (13_574_563_200_000, "2400-02-29T00:00:00Z"),
        (253_402_300_799_999, "9999-12-31T23:59:59Z"),
    ] {
        assert_eq!(format_utc(time).as_deref(), Some(expected));
    }
}

#[test]
fn utc_rejects_out_of_range_and_traverses_a_complete_gregorian_cycle() {
    for time in [i64::MIN, -62_167_219_200_001, 253_402_300_800_000, i64::MAX] {
        assert!(format_utc(time).is_none());
    }
    let mut time = 946_684_800_000_i64; // 2000-01-01
    for year in 2000..2400 {
        let lengths = [
            31,
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        for (index, length) in lengths.into_iter().enumerate() {
            for day in 1..=length {
                assert_eq!(
                    format_utc(time).unwrap(),
                    format!("{year:04}-{:02}-{day:02}T00:00:00Z", index + 1)
                );
                time += 86_400_000;
            }
        }
    }
    assert_eq!(format_utc(time).as_deref(), Some("2400-01-01T00:00:00Z"));
}

#[test]
fn model_projection_preserves_order_current_and_native_mode_names() {
    for mode in [
        PermissionMode::Ask,
        PermissionMode::Auto,
        PermissionMode::Yolo,
    ] {
        let value = config_value(mode, "current", ["other", "current"].into_iter()).unwrap();
        assert_eq!(value["configOptions"][0]["currentValue"], mode.as_str());
        assert_eq!(
            value["configOptions"][1]["options"],
            json!([
                {"value":"other","name":"other"},{"value":"current","name":"current"}
            ])
        );
        let appended = config_value(mode, "current", ["other"].into_iter()).unwrap();
        assert_eq!(
            appended["configOptions"][1]["options"],
            value["configOptions"][1]["options"]
        );
        let empty = config_value(mode, "current", std::iter::empty()).unwrap();
        assert_eq!(
            empty["configOptions"][1]["options"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn all_model_output_is_preflighted_before_projection_or_extra_current_allocation() {
    assert!(
        config_value(
            PermissionMode::Ask,
            "current",
            std::iter::repeat_n("id", MAX_MODEL_OPTIONS)
        )
        .is_ok()
    );
    assert!(matches!(
        config_value(
            PermissionMode::Ask,
            "current",
            std::iter::repeat_n("id", MAX_MODEL_OPTIONS + 1)
        ),
        Err(AcpSessionError::Limit)
    ));
    let escaped = "\0".repeat(MAX_PROJECTION_BYTES / 6);
    assert!(matches!(
        config_value(PermissionMode::Ask, &escaped, std::iter::empty()),
        Err(AcpSessionError::Limit)
    ));
    assert!(matches!(
        config_value(
            PermissionMode::Ask,
            "current",
            [escaped.as_str()].into_iter()
        ),
        Err(AcpSessionError::Limit)
    ));
    let mut budget = Budget(8);
    budget.string("\0").unwrap(); // quotes plus the six-byte JSON escape
    assert!(budget.charge(1).is_err());
    assert!(check_catalog_count(100).is_ok());
    assert!(matches!(
        check_catalog_count(101),
        Err(AcpSessionError::Limit)
    ));
}

struct CatalogFixture {
    directory: PathBuf,
    store: Arc<FileSessionStore>,
}
impl CatalogFixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "mg-acp-projection-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let store = Arc::new(FileSessionStore::open(&directory).unwrap());
        Self { directory, store }
    }
    fn save(&self, id: &str, metadata: &NativeSessionMetadata) {
        self.save_value(id, metadata.to_value());
    }
    fn save_value(&self, id: &str, metadata: Value) {
        let mut record = SessionRecord::empty(
            SessionId::new(id).unwrap(),
            SessionIncarnationId::new(format!("life-{id}")).unwrap(),
        );
        record
            .messages
            .push(Message::text(Role::User, "preview is not a title"));
        record
            .metadata
            .insert(crate::NATIVE_SESSION_METADATA_KEY.to_owned(), metadata);
        futures_executor::block_on(self.store.save(record, None)).unwrap();
    }
    fn page(&self, query: NativeSessionCatalogQuery) -> NativeSessionCatalogPage {
        futures_executor::block_on(NativeSessionCatalog::new(self.store.clone()).list(query))
            .unwrap()
    }
}
impl Drop for CatalogFixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}
fn metadata(time: i64) -> NativeSessionMetadata {
    NativeSessionMetadata::new(Path::new("/workspace"), time, NativeSessionOrigin::Acp).unwrap()
}

#[test]
fn actual_catalog_projects_only_known_native_facts_and_utc_time() {
    let fixture = CatalogFixture::new();
    let mut titled = metadata(1_700_000_000_123);
    titled.rename("Real title", 1_700_000_000_123).unwrap();
    fixture.save("titled", &titled);
    fixture.save("pre-epoch", &metadata(-1));
    fixture.save("unrepresentable", &metadata(i64::MAX));
    let mut unknown = metadata(0).to_value();
    unknown["created_at_ms"] = Value::Null;
    unknown["updated_at_ms"] = Value::Null;
    fixture.save_value("unknown-time", unknown);
    let page = fixture.page(NativeSessionCatalogQuery::new(100).unwrap());
    let projected = catalog_response(&page).unwrap();
    let rows = projected["sessions"].as_array().unwrap();
    let row = |id: &str| rows.iter().find(|row| row["sessionId"] == id).unwrap();
    assert_eq!(row("titled")["updatedAt"], "2023-11-14T22:13:20Z");
    assert_eq!(row("titled")["title"], "Real title");
    assert_eq!(row("pre-epoch")["updatedAt"], "1969-12-31T23:59:59Z");
    assert!(row("pre-epoch").get("title").is_none());
    assert!(row("unknown-time").get("updatedAt").is_none());
    assert!(row("unrepresentable").get("updatedAt").is_none());
    assert!(rows.iter().all(|row| row["cwd"] == "/workspace"));
    assert_eq!(projected["_meta"]["machineGod"]["omittedUpdatedAt"], 1);
    assert!(projected.get("nextCursor").is_none());
}

#[test]
fn filtered_rows_keep_the_native_cursor_and_report_workspace_omissions() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let fixture = CatalogFixture::new();
    fixture.save("first", &metadata(30));
    let mut unknown = NativeSessionMetadata::default().to_value();
    unknown["updated_at_ms"] = json!(20);
    fixture.save_value("missing-workspace", unknown);
    fixture.save("last", &metadata(10));
    let page = fixture.page(NativeSessionCatalogQuery::new(2).unwrap());
    let projected = catalog_response(&page).unwrap();
    assert_eq!(projected["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(projected["_meta"]["machineGod"]["omittedWorkspace"], 1);
    assert_eq!(projected["_meta"]["machineGod"]["resultsTruncated"], true);
    assert_eq!(
        projected["_meta"]["machineGod"]["scanComplete"],
        page.scan_complete()
    );
    let cursor = page.next_cursor().unwrap();
    assert_eq!(projected["nextCursor"], cursor.to_string());
    let next = fixture.page(
        NativeSessionCatalogQuery::new(2)
            .unwrap()
            .with_continuation(cursor),
    );
    assert_eq!(
        catalog_response(&next).unwrap()["sessions"][0]["sessionId"],
        "last"
    );
    for (id, path) in [
        (
            "non-utf8",
            PathBuf::from(OsString::from_vec(vec![b'/', 0xff])),
        ),
        ("control", PathBuf::from("/bad\nworkspace")),
    ] {
        fixture.save(
            id,
            &NativeSessionMetadata::new(&path, 40, NativeSessionOrigin::Acp).unwrap(),
        );
    }
    let projected =
        catalog_response(&fixture.page(NativeSessionCatalogQuery::new(100).unwrap())).unwrap();
    assert_eq!(projected["_meta"]["machineGod"]["omittedWorkspace"], 3);
    assert_eq!(projected["sessions"].as_array().unwrap().len(), 2);
}

#[test]
fn full_native_page_and_skipped_invalid_evidence_remain_bounded_and_honest() {
    let fixture = CatalogFixture::new();
    for index in 0..100 {
        fixture.save(&format!("row-{index}"), &metadata(index));
    }
    let mut invalid = metadata(200).to_value();
    invalid["unknown"] = Value::Bool(true);
    fixture.save_value("invalid", invalid);
    let page = fixture.page(
        NativeSessionCatalogQuery::new(100)
            .unwrap()
            .with_invalid_records(crate::NativeSessionCatalogInvalidRecords::SkipAndReport),
    );
    let projected = catalog_response(&page).unwrap();
    assert_eq!(projected["sessions"].as_array().unwrap().len(), 100);
    assert_eq!(
        projected["_meta"]["machineGod"]["skippedInvalid"],
        page.skipped_invalid()
    );
    assert_eq!(page.skipped_invalid(), 1);
    assert!(serde_json::to_vec(&projected).unwrap().len() < MAX_PROJECTION_BYTES);
}

#[test]
fn actual_selected_session_uses_its_current_mode_model_and_identity_without_new_factory_effects() {
    use crate::acp::{
        selection::{NativeAcpSelectionOutcome, NativeAcpSelectionOwner, tests::fixture::Factory},
        session::NativeAcpSessionSelection,
    };
    use std::task::Poll;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let factory = Arc::new(Factory::new());
            let mut owner = NativeAcpSelectionOwner::new(factory.clone());
            owner
                .request(
                    NativeAcpSessionSelection::New,
                    factory.workspace.clone(),
                    crate::mcp::ephemeral::NativeMcpEphemeralConfiguration::decode(None).unwrap(),
                    1,
                )
                .unwrap();
            let selected = futures_util::future::poll_fn(|cx| {
                let _ = owner.poll_progress(cx, 1);
                owner.take_outcome().map_or(Poll::Pending, Poll::Ready)
            })
            .await;
            assert!(matches!(
                selected,
                NativeAcpSelectionOutcome::Selected { .. }
            ));
            let session = owner.current().unwrap();
            session.set_mode(&session.id(), "auto").unwrap();
            let transport = Arc::new(CatalogTransport(AtomicU64::new(0)));
            let catalog = Arc::new(
                crate::AiGatewayModelCatalogProvider::new(
                    crate::AiGatewayModelCatalogAccessMode::PublicOnly,
                    transport.clone(),
                )
                .list_model_details(machine_god_core::CancellationToken::new())
                .await
                .unwrap(),
            );
            session
                .runtime()
                .set_model_catalog(catalog.clone())
                .unwrap();
            let projected = selection_response(session).unwrap();
            let config = config_response(session).unwrap();
            assert_eq!(projected["sessionId"], session.id().as_str());
            assert_eq!(projected["configOptions"], config["configOptions"]);
            assert_eq!(projected["modes"]["currentModeId"], "auto");
            assert_eq!(projected["configOptions"][0]["currentValue"], "auto");
            assert_eq!(
                projected["configOptions"][1]["currentValue"],
                session.runtime().model_preferences().model()
            );
            assert_eq!(factory.preparations.load(Ordering::Relaxed), 1);
            assert_eq!(transport.0.load(Ordering::Relaxed), 1);
            let options = projected["configOptions"][1]["options"].as_array().unwrap();
            for (index, entry) in catalog.entries().iter().enumerate() {
                assert_eq!(options[index]["value"], entry.model().id());
                assert_eq!(options[index]["name"], entry.model().id());
            }
            assert_eq!(options.len(), catalog.entries().len() + 1);
            owner.request_shutdown();
            let closed = futures_util::future::poll_fn(|cx| {
                let _ = owner.poll_progress(cx, 1);
                owner.take_outcome().map_or(Poll::Pending, Poll::Ready)
            })
            .await;
            assert!(matches!(closed, NativeAcpSelectionOutcome::Closed { .. }));
        });
}

struct CatalogTransport(AtomicU64);
impl crate::AiGatewayModelCatalogTransport for CatalogTransport {
    fn wait_until(&self, deadline: std::time::Instant) -> machine_god_core::BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
    fn get(
        &self,
        _: crate::AiGatewayModelCatalogRequestAccess,
        _: std::time::Instant,
        _: machine_god_core::CancellationToken,
    ) -> machine_god_core::BoxFuture<
        '_,
        Result<
            crate::AiGatewayModelCatalogTransportResponse,
            crate::AiGatewayModelCatalogTransportError,
        >,
    > {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(crate::AiGatewayModelCatalogTransportResponse::new(200, br#"{"data":[{"id":"z/last","type":"language"},{"id":"a/first","type":"language"}]}"#.to_vec()))
        })
    }
}
