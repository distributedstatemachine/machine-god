use super::super::support;
use super::*;
use machine_god_core::{Message, Role, SessionIncarnationId, SessionRecord, SessionStore};
use machine_god_native::{NATIVE_SESSION_METADATA_KEY, NativeSessionMetadata, NativeSessionOrigin};
use std::{future::poll_fn, time::Duration};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn save(
    fixture: &support::Fixture,
    id: &str,
    title: &str,
    time: i64,
    workspace: &std::path::Path,
) {
    let mut record = SessionRecord::empty(
        SessionId::new(id).unwrap(),
        SessionIncarnationId::new(format!("incarnation-{id}")).unwrap(),
    );
    let mut metadata =
        NativeSessionMetadata::new(workspace, time, NativeSessionOrigin::Cli).unwrap();
    metadata.rename(title, time).unwrap();
    record
        .metadata
        .insert(NATIVE_SESSION_METADATA_KEY.into(), metadata.to_value());
    record
        .messages
        .push(Message::text(Role::User, "a searchable preview"));
    fixture
        .host
        .session_lifecycle()
        .session_store()
        .save(record, None)
        .await
        .unwrap();
}

async fn ready(picker: &mut Picker) {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            picker.poll(cx);
            if picker.pending.is_none() && picker.queued.is_none() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }),
    )
    .await
    .unwrap();
}

#[test]
fn page_sizes_and_ascii_search_are_bounded_and_do_not_search_ids() {
    assert_eq!(
        [page_size(1), page_size(24), page_size(u16::MAX)],
        [10, 17, 100]
    );
    assert!(contains_ascii(b"aBcdef", b"BCD"));
    assert!(!contains_ascii("É".as_bytes(), "é".as_bytes()));
    assert!(!contains_ascii(b"a", b"longer"));
}

#[test]
fn exact_acknowledged_selection_search_scope_and_cache() {
    let runtime = runtime();
    let fixture = support::Fixture::new();
    runtime.block_on(async {
        save(&fixture, "current", "Current", 30, &fixture.workspace).await;
        save(
            &fixture,
            "id-not-searchable",
            "Mixed Case",
            20,
            &fixture.workspace,
        )
        .await;
        save(
            &fixture,
            "elsewhere",
            "Elsewhere",
            40,
            std::path::Path::new("/other/workspace"),
        )
        .await;
        let mut picker = Picker::new(
            fixture.host.session_catalog_reader().unwrap(),
            Some(SessionId::new("current").unwrap()),
            24,
        );
        picker.open(NativeSessionCatalogScope::CurrentWorkspace);
        ready(&mut picker).await;
        assert_eq!(picker.view.as_ref().unwrap().matches.len(), 1);
        let (generation, revision) = picker.identity().unwrap();
        assert!(matches!(
            picker.select(generation, revision),
            Selection::None
        ));
        picker.acknowledge(generation, revision);
        picker.query(" MIXED ").unwrap();
        assert!(matches!(
            picker.select(generation, revision),
            Selection::None
        ));
        assert_eq!(picker.view.as_ref().unwrap().matches.len(), 1);
        picker.query("id-not-searchable").unwrap();
        assert!(picker.view.as_ref().unwrap().matches.is_empty());
        picker.query("searchable preview").unwrap();
        assert_eq!(picker.view.as_ref().unwrap().matches.len(), 1);
        picker.toggle_scope();
        ready(&mut picker).await;
        assert_eq!(picker.view.as_ref().unwrap().matches.len(), 2);
        let (generation, revision) = picker.identity().unwrap();
        picker.acknowledge(generation, revision);
        let Selection::Session(target) = picker.select(generation, revision) else {
            panic!("selected")
        };
        assert_eq!(target.id().as_str(), "elsewhere");
        picker.selection_failed("busy");
        picker.close();
        picker.open(NativeSessionCatalogScope::CurrentWorkspace);
        assert_eq!(
            picker.view.as_ref().unwrap().page.rows.len(),
            1,
            "cached before refresh"
        );
        ready(&mut picker).await;
        let request = Request {
            generation: picker.generation,
            scope: NativeSessionCatalogScope::CurrentWorkspace,
            limit: 10,
            cursor: None,
        };
        picker.accept(&request, Err(NativeSessionCatalogReadError::Busy));
        assert_eq!(picker.view.as_ref().unwrap().page.rows.len(), 1);
        assert!(picker.view.as_ref().unwrap().failure.is_some());
        drop(picker);
    });
    fixture.finish();
}

#[test]
fn pagination_uses_unfiltered_cursor_and_selects_new_matching_rows() {
    let runtime = runtime();
    let fixture = support::Fixture::new();
    runtime.block_on(async {
        for index in 0..12 {
            save(
                &fixture,
                &format!("row-{index:02}"),
                &format!("Title {index:02}"),
                index,
                &fixture.workspace,
            )
            .await;
        }
        let mut picker = Picker::new(fixture.host.session_catalog_reader().unwrap(), None, 5);
        picker.open(NativeSessionCatalogScope::CurrentWorkspace);
        ready(&mut picker).await;
        picker.query("Title 00").unwrap();
        assert!(picker.view.as_ref().unwrap().matches.is_empty());
        assert!(picker.view.as_ref().unwrap().page.cursor.is_some());
        let (generation, revision) = picker.identity().unwrap();
        picker.acknowledge(generation, revision);
        assert!(matches!(
            picker.select(generation, revision),
            Selection::LoadMore
        ));
        ready(&mut picker).await;
        let view = picker.view.as_ref().unwrap();
        assert_eq!(view.matches.len(), 1);
        assert_eq!(
            view.page.rows[view.matches[view.selected]]
                .observed
                .id()
                .as_str(),
            "row-00"
        );
        assert!(view.page.cursor.is_none());
        drop(picker);
    });
    fixture.finish();
}

#[test]
fn menu_labels_cannot_inject_controls_or_wrap_at_small_sizes() {
    let rendered = super::super::composer_view::label("x\u{1b}[2J\u{202e}y", 80, 512);
    assert!(!rendered.contains(&27));
    assert!(!String::from_utf8(rendered).unwrap().contains('\u{202e}'));
    assert!(super::super::composer_view::label("abc", 1, 512).is_empty());
    assert!(super::super::composer_view::label(&"x".repeat(1000), 1000, 512).len() <= 512);
}
