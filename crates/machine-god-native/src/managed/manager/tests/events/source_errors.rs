use super::*;

fn unavailable_older_page(sections: &[&str], continuation: bool) {
    for missing in [false, true] {
        let mut fixture = Fixture::new(vec![]);
        receipt(fixture.command(serde_json::json!({
            "create": {"name": "worker", "mode": "persistent"}
        })));
        fixture.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
        let pages = fixture.journal_pages();
        assert_eq!(pages.len(), 1, "one actual creation page");
        let original = std::fs::read(&pages[0]).unwrap();
        receipt(fixture.command(serde_json::json!({
            "configure": {"id": "child-1", "name": "renamed"}
        })));
        fixture.drive(|f| f.manager.active.is_none() && f.manager.replay.done);

        let mut query = serde_json::json!({
            "id": "child-1", "sections": sections, "limit": 100
        });
        if continuation {
            let first = fixture.command(serde_json::json!({
                "inspect": {"id": "child-1", "sections": sections, "limit": 1}
            }));
            assert!(first.ok, "{first:?}");
            query["cursor"] = first.cursor.expect("older records remain").into();
        }
        fixture.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
        if missing {
            std::fs::remove_file(&pages[0]).unwrap();
        } else {
            std::fs::write(&pages[0], b"corrupt fixture-owned older page").unwrap();
        }
        // The current head/tail remain valid: this failure belongs to history
        // traversal, not child lookup. Restore evidence before assertions/drop.
        assert!(block_on(fixture.journal.inspect("child-1".into())).is_ok());
        let result = fixture.command(serde_json::json!({"inspect": query}));
        std::fs::write(&pages[0], original).unwrap();
        assert!(
            result.ok,
            "other selected sources remain usable: {result:?}"
        );
        result.validate().unwrap();
        let Some(ManagedRequested::Inspection(page)) = result.requested else {
            panic!("inspection required");
        };
        let wire = serde_json::to_value(&page).unwrap();
        assert_eq!(wire["events_error"], "unavailable", "{wire}");
        assert!(page.events.is_empty());
        assert_eq!(page.history_error.is_some(), sections.contains(&"messages"));
        assert_eq!(
            page.tool_activity_error.is_some(),
            sections.contains(&"tool_activity")
        );
        assert!(page.next_cursor.is_none());
        assert!(!page.restart_required);
        assert_eq!(events(&mut fixture).len(), 2, "restored source is readable");
        assert!(fixture.factory.provider.requests().is_empty());
    }
}

#[test]
fn events_only_inspection_reports_unavailable_older_pages() {
    unavailable_older_page(&["events"], false);
}

#[test]
fn event_continuation_reports_unavailable_older_pages() {
    unavailable_older_page(&["events"], true);
}

#[test]
fn mixed_history_inspection_reports_each_selected_source_failure() {
    unavailable_older_page(&["events", "messages", "tool_activity"], false);
}
