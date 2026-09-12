//! Successful configured Ask/Resume acceptance, not manual runtime publication.
use super::*;
use super::{configured_support as support, configured_wire as wire};
use serde_json::json;

#[test]
fn configured_required_ask_and_new_host_resume_search_select_call_and_page_archive() {
    let directory = ScopedTestDirectory::new("mcp-configured-required");
    let ask = support::Fixture::new(&directory, true, support::script("ask", "first-profile"));
    let subscription = ask.activate(true, "first-profile").unwrap();
    ask.turn(
        SessionSelection::CreateGenerated,
        wire::call(&ask.listener, 4, "first-profile"),
    );
    let id = ask.provider.session_ids()[0].clone();
    assert_eq!(ask.provider.session_ids(), vec![id.clone(); 4]);
    let record = ask.record(id.clone());
    support::assert_search(&record, "ask");
    let handle = support::archive(&record, "ask");
    support::assert_projection(&ask.provider, 1, "first-profile");
    assert_eq!(record.next_turn_sequence, 2);
    let incarnation = record.incarnation_id;
    ask.finish(subscription);

    // A distinct CLI host reloads the selected profile and negotiates a new peer.
    // The saved session supplies history, never the old publication or socket.
    let mut responses = vec![support::call(
        "resume-page",
        "read_tool_result",
        &json!({
            "handle":handle,"start_byte":69_000,"byte_count":16384
        }),
    )];
    responses.extend(support::script("resume", "second-profile"));
    let resume = support::Fixture::new(&directory, true, responses);
    let subscription = resume.activate(true, "second-profile").unwrap();
    resume.turn(
        SessionSelection::Resume(id.clone()),
        wire::call(&resume.listener, 4, "second-profile"),
    );
    assert_eq!(resume.provider.session_ids(), vec![id.clone(); 5]);
    let record = resume.record(id);
    assert_eq!(record.incarnation_id, incarnation);
    assert_eq!(record.next_turn_sequence, 3);
    support::assert_search(&record, "resume");
    assert_ne!(support::archive(&record, "resume"), handle);
    let page = support::persisted(&record, "resume-page");
    assert!(!page.is_error, "{page:?}");
    assert!(
        serde_json::to_string(&page.content)
            .unwrap()
            .contains("configured-archive-end")
    );
    support::assert_projection(&resume.provider, 2, "second-profile");
    resume.finish(subscription);
}

#[test]
fn configured_optional_first_model_demand_discovers_once_and_owned_shutdown_closes_listener() {
    let directory = ScopedTestDirectory::new("mcp-configured-optional");
    let mut responses = support::script("first", "optional-profile");
    responses.extend(support::script("second", "optional-profile"));
    let fixture = support::Fixture::new(&directory, false, responses);
    assert!(fixture.activate(false, "optional-profile").is_none());
    let server = async {
        let socket = wire::startup(&fixture.listener, "optional-profile").await;
        assert_eq!(
            fixture.provider.request_bodies().len(),
            1,
            "only the first model search may trigger deferred startup"
        );
        wire::call(&fixture.listener, 4, "optional-profile").await;
        socket
    };
    let subscription = fixture.turn(SessionSelection::CreateGenerated, server);
    let id = fixture.provider.session_ids()[0].clone();
    let first = fixture.record(id.clone());
    support::assert_search(&first, "first");
    support::archive(&first, "first");

    // No discover/list/listen script remains: repeated model search must use the
    // admitted positive-TTL optional publication and continue its ID allocator.
    fixture.turn(
        SessionSelection::Resume(id.clone()),
        wire::call(&fixture.listener, 5, "optional-profile"),
    );
    assert_eq!(fixture.provider.session_ids(), vec![id.clone(); 8]);
    let record = fixture.record(id);
    support::assert_search(&record, "second");
    support::archive(&record, "second");
    assert_eq!(record.next_turn_sequence, 3);
    fixture.finish(subscription);
}
