use super::human::{close, command, create, open, submit};
use super::navigation_ui::ready;
use super::*;
use crate::{NativeManagedNavigationAction as Action, NativeManagedNavigationError as Error};

async fn act(owner: &mut NativeInteractiveSession, action: Action) {
    let frame = ready(owner).await;
    owner.acknowledge_managed_frame(&frame).unwrap();
    owner.act_on_managed_frame(&frame, action).unwrap();
    ready(owner).await;
}

async fn select(owner: &mut NativeInteractiveSession, id: &str) {
    if owner.managed_navigation().is_none() {
        owner.open_managed_navigation().unwrap();
    }
    ready(owner).await;
    let view = owner.managed_navigation().unwrap();
    let target = view.rows.iter().position(|row| row.id == id).unwrap();
    let current = view.selected.unwrap();
    for _ in current..target {
        act(owner, Action::Next).await;
    }
    for _ in target..current {
        act(owner, Action::Previous).await;
    }
    act(owner, Action::Select).await;
}

#[test]
fn message_acceptance_clears_submitted_text_without_erasing_a_newer_draft() {
    let mut fixture = Fixture::with_options("auto", true, options);
    fixture
        .transport
        .responses
        .lock()
        .unwrap()
        .extend([answer(), answer()]);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let first = submit(&mut owner, create()).await.child_id.unwrap();
        let second = submit(&mut owner, create()).await.child_id.unwrap();
        for (child, newer) in [(first, false), (second, true)] {
            select(&mut owner, &child).await;
            let view = owner.managed_navigation().unwrap();
            let frame = view.frame;
            owner.edit_managed_draft(&view.editor, "send", 4).unwrap();
            owner.acknowledge_managed_frame(&frame).unwrap();
            owner
                .act_on_managed_frame(&frame, Action::Message("send".into()))
                .unwrap();
            let view = owner.managed_navigation().unwrap();
            assert_eq!(view.draft.unwrap().text, "send");
            assert!(view.busy);
            if newer {
                owner.edit_managed_draft(&view.editor, "newer", 2).unwrap();
            }
            ready(&mut owner).await;
            let view = owner.managed_navigation().unwrap();
            assert!(view.result.unwrap().ok);
            assert_eq!(view.draft.unwrap().text, if newer { "newer" } else { "" });
            act(&mut owner, Action::Back).await;
        }
        close(owner, completion).await;
    });
}

#[test]
fn independent_unicode_drafts_survive_siblings_close_and_fresh_editor_epochs() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let first = submit(&mut owner, create()).await.child_id.unwrap();
        let second = submit(&mut owner, create()).await.child_id.unwrap();
        select(&mut owner, &first).await;
        let old = owner.managed_navigation().unwrap().editor;
        owner.edit_managed_draft(&old, "α\nfirst", 2).unwrap();
        act(&mut owner, Action::Back).await;
        select(&mut owner, &second).await;
        let view = owner.managed_navigation().unwrap();
        assert!(view.draft.unwrap().text.is_empty());
        owner.edit_managed_draft(&view.editor, "second", 3).unwrap();
        owner.close_managed_navigation();
        select(&mut owner, &first).await;
        let view = owner.managed_navigation().unwrap();
        assert_eq!(view.draft.unwrap().text, "α\nfirst");
        assert_eq!(view.draft.unwrap().cursor, 2);
        assert_ne!(old, view.editor);
        let current = view.editor;
        assert_eq!(
            owner.edit_managed_draft(&old, "stale", 0),
            Err(Error::StaleFrame)
        );
        assert_eq!(
            owner.edit_managed_draft(&current, "α", 1),
            Err(Error::InvalidAction)
        );
        assert_eq!(
            owner.managed_navigation().unwrap().draft.unwrap().text,
            "α\nfirst"
        );
        act(&mut owner, Action::Back).await;
        select(&mut owner, &second).await;
        let draft = owner.managed_navigation().unwrap().draft.unwrap();
        assert_eq!((draft.text, draft.cursor), ("second", 3));
        let view = owner.managed_navigation().unwrap();
        let frame = view.frame;
        owner.edit_managed_draft(&view.editor, "/back", 5).unwrap();
        owner.acknowledge_managed_frame(&frame).unwrap();
        owner
            .submit_managed_frame(&frame, Action::Back, "/back")
            .unwrap();
        select(&mut owner, &second).await;
        assert!(
            owner
                .managed_navigation()
                .unwrap()
                .draft
                .unwrap()
                .text
                .is_empty()
        );
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn rejected_message_preserves_draft_in_its_new_editor() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        select(&mut owner, &child).await;
        let editor = owner.managed_navigation().unwrap().editor;
        owner.edit_managed_draft(&editor, "keep", 2).unwrap();
        assert!(
            submit(
                &mut owner,
                command(serde_json::json!({
                    "configure":{"id":child,"name":"changed"}
                }))
            )
            .await
            .ok
        );
        act(&mut owner, Action::Message("keep".into())).await;
        let view = owner.managed_navigation().unwrap();
        assert!(!view.result.unwrap().ok);
        assert_ne!(view.editor, editor);
        assert_eq!(
            (view.draft.unwrap().text, view.draft.unwrap().cursor),
            ("keep", 2)
        );
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}
