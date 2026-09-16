use super::human::{close, command, create, open, response, submit};
use super::*;
use machine_god_core::{ManagedAgentState, ManagedResultStatus};

fn skill_options(
    selected: NativeReferenceHostConversationOptions,
    directory: &Directory,
    clock: Arc<Clock>,
) -> NativeReferenceHostConversationOptions {
    let workspace = directory.0.join("workspace");
    let path = workspace.join("skills/selected");
    fs::create_dir_all(&path).unwrap();
    fs::write(
        path.join("SKILL.md"),
        "---\nname: selected\n---\nSELECTED_SKILL_BODY",
    )
    .unwrap();
    let root = crate::NativeSkillRoot::from_directory(
        Arc::new(fs::File::open(&workspace).unwrap()),
        "skills".into(),
        workspace.clone(),
        workspace.join("skills"),
        crate::NativeSkillSource::WorkspaceShared,
        crate::NativeSkillLinkPolicy::Reject,
    )
    .unwrap();
    options(selected, directory, clock).with_skills(Arc::new(crate::NativeSkillsService::new(
        Arc::new(crate::NativeSkillCatalog::new(vec![root]).unwrap()),
        None,
    )))
}

fn selected(fixture: &Fixture) -> crate::NativeSkillReference {
    let service = fixture.host().skills().unwrap();
    let snapshot = service
        .catalog()
        .discover(&CancellationToken::new())
        .unwrap();
    service
        .catalog()
        .reference(&snapshot.entries()[0].selection())
        .unwrap()
}

async fn state(owner: &mut NativeInteractiveSession, child: &str, state: ManagedAgentState) {
    poll_fn(|cx| {
        let progress = owner.poll_progress(cx, 20);
        assert!(
            owner.managed_error().is_none(),
            "{:?}",
            owner.managed_error()
        );
        assert!(
            owner.shutdown_error().is_none(),
            "{:?}",
            owner.shutdown_error()
        );
        let _ = owner.take_presentation();
        if owner
            .managed_agents()
            .iter()
            .any(|agent| agent.id == child && agent.state == state)
        {
            return Poll::Ready(());
        }
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    })
    .await;
}

async fn send(
    owner: &mut NativeInteractiveSession,
    child: &str,
    reference: crate::NativeSkillReference,
) {
    let pending = owner
        .request_managed_command_with_skill_references(
            command(
                serde_json::json!({"message":{"send":{"id":child,"content":"$selected $later"}}}),
            ),
            &[reference],
            CancellationToken::new(),
        )
        .unwrap();
    let result = response(owner, pending).await.unwrap();
    assert_eq!(
        result.status,
        ManagedResultStatus::MessageQueued,
        "{result:?}"
    );
}

#[test]
fn child_skill_context_uses_frozen_references_and_preserves_canonical_user_text() {
    let mut fixture = Fixture::with_options("auto", true, skill_options);
    let reference = selected(&fixture);
    let later = fixture.workspace.join("skills/later");
    fs::create_dir_all(&later).unwrap();
    fs::write(
        later.join("SKILL.md"),
        "---\nname: later\n---\nNEW_UNSELECTED_BODY",
    )
    .unwrap();
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        send(&mut owner, &child, reference).await;
        state(&mut owner, &child, ManagedAgentState::Idle).await;
        {
            let requests = fixture.transport.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            let wire = requests[0].to_string();
            assert!(wire.contains("SELECTED_SKILL_BODY"), "{wire}");
            assert!(!wire.contains("NEW_UNSELECTED_BODY"));
        }
        let history = super::navigation_history::current_history(&mut owner).await;
        assert!(history.record().messages.iter().any(|message|
            message.role == machine_god_core::Role::User && message.content.iter().any(|block|
                matches!(block, machine_god_core::ContentBlock::Text { text } if text == "$selected $later")
            )
        ));
        drop(history);
        close(owner, completion).await;
    });
}

fn rejected_skill(available: bool) {
    let mut fixture = Fixture::with_options("auto", true, skill_options);
    let reference = selected(&fixture);
    if available {
        fs::write(
            fixture.workspace.join("skills/selected/SKILL.md"),
            "---\nname: selected\n---\nCHANGED_SKILL_BODY",
        )
        .unwrap();
    } else {
        fixture.host.as_mut().unwrap().skills = None;
    }
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        send(&mut owner, &child, reference).await;
        state(&mut owner, &child, ManagedAgentState::Failed).await;
        assert!(fixture.transport.requests.lock().unwrap().is_empty());
        // Failure belongs to this head. The manager and independent sibling
        // remain usable without dropping the failed work or silently retrying it.
        let sibling = submit(&mut owner, create()).await.child_id.unwrap();
        let sent = submit(
            &mut owner,
            command(serde_json::json!({
                "message":{"send":{"id":sibling,"content":"independent sibling"}}
            })),
        )
        .await;
        assert_eq!(sent.status, ManagedResultStatus::MessageQueued);
        state(&mut owner, &sibling, ManagedAgentState::Idle).await;
        assert_eq!(fixture.transport.requests.lock().unwrap().len(), 1);
        assert!(
            owner
                .managed_agents()
                .iter()
                .any(|agent| agent.id == child && agent.state == ManagedAgentState::Failed)
        );
        close(owner, completion).await;
    });
}

#[test]
fn changed_child_skill_fails_only_its_accepted_work() {
    rejected_skill(true);
}

#[test]
fn unavailable_child_skill_catalog_fails_only_its_accepted_work() {
    rejected_skill(false);
}

#[test]
fn child_menu_preserves_draft_binding_and_requires_new_ack_after_filter_edit() {
    use super::navigation_ui::ready;
    use crate::{NativeManagedNavigationAction as Action, NativeManagedNavigationError as Error};
    let mut fixture = Fixture::with_options("auto", true, skill_options);
    let snapshot = Arc::new(
        fixture
            .host()
            .skills()
            .unwrap()
            .catalog()
            .discover(&CancellationToken::new())
            .unwrap(),
    );
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        owner.set_managed_skills_snapshot(Some(snapshot));
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        owner.open_managed_navigation().unwrap();
        let frame = ready(&mut owner).await;
        owner.acknowledge_managed_frame(&frame).unwrap();
        owner.act_on_managed_frame(&frame, Action::Select).unwrap();
        let frame = ready(&mut owner).await;
        let editor = owner.managed_navigation().unwrap().editor;
        owner.edit_managed_draft(&editor, "before", 6).unwrap();
        owner.acknowledge_managed_frame(&frame).unwrap();
        owner.act_on_managed_frame(&frame, Action::Skills).unwrap();
        let frame = ready(&mut owner).await;
        let editor = owner.managed_navigation().unwrap().editor;
        owner.acknowledge_managed_frame(&frame).unwrap();
        owner
            .edit_managed_skill_query(&editor, "selected", 8)
            .unwrap();
        assert_eq!(
            owner.act_on_managed_frame(&frame, Action::Select),
            Err(Error::StaleFrame)
        );
        let frame = ready(&mut owner).await;
        owner.acknowledge_managed_frame(&frame).unwrap();
        owner.act_on_managed_frame(&frame, Action::Select).unwrap();
        let frame = ready(&mut owner).await;
        let draft = owner
            .managed_navigation()
            .unwrap()
            .draft
            .unwrap()
            .text
            .to_owned();
        assert_eq!(draft, "before $selected ");
        assert!(fixture.transport.requests.lock().unwrap().is_empty());
        owner.acknowledge_managed_frame(&frame).unwrap();
        owner
            .submit_managed_frame(&frame, Action::Message(draft.clone()), &draft)
            .unwrap();
        state(&mut owner, &child, ManagedAgentState::Idle).await;
        ready(&mut owner).await;
        assert!(
            owner
                .managed_navigation()
                .unwrap()
                .draft
                .unwrap()
                .text
                .is_empty()
        );
        assert!(
            fixture.transport.requests.lock().unwrap()[0]
                .to_string()
                .contains("SELECTED_SKILL_BODY")
        );
        close(owner, completion).await;
    });
}
