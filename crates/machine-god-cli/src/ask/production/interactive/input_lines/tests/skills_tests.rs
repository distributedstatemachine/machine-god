use super::*;
use machine_god_native::{NativeSkillPicker, NativeSkillPickerMode};

fn binding(picker: &NativeSkillPicker, query: bool) -> InputBinding {
    InputBinding::Skills {
        epoch: picker.draft_identity().clone(),
        frame: None,
        query,
    }
}

async fn next(
    input: &mut InputLines,
    binding: &InputBinding,
    mode: Option<NativeSkillPickerMode>,
) -> (ComposerEvent, InputBinding) {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            input.poll_event(
                cx,
                binding.clone(),
                ComposerContext {
                    skills: mode,
                    ..ComposerContext::default()
                },
            )
        }),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap()
}

#[test]
fn skills_query_preserves_entire_original_draft_and_utf8_cursor() {
    let (mut input, mut writer) = raw_source();
    let draft = "before 🦀 after";
    input
        .composer
        .as_mut()
        .unwrap()
        .replace(0..0, draft, 7)
        .unwrap();
    let picker = NativeSkillPicker::new(draft.into(), 7).unwrap();
    let query = "é".repeat(512);
    input.open_skills_query(&query).unwrap();
    assert_eq!(input.original_draft(), Some((draft, 7)));
    assert_eq!(input.raw_draft(), Some((query.as_str(), 1024)));
    runtime().block_on(async {
        writer.write_all(b"\t").unwrap();
        assert!(matches!(
            next(
                &mut input,
                &binding(&picker, true),
                Some(NativeSkillPickerMode::Menu)
            )
            .await
            .0,
            ComposerEvent::SkillSelected
        ));
        assert_eq!(input.raw_draft().unwrap().0, query);
    });
    input.close_skills_query();
    assert_eq!(input.raw_draft(), Some((draft, 7)));
    finish(input);
}

#[test]
fn skills_query_close_does_not_suppress_a_fresh_newline_from_the_old_slash_cr() {
    let (mut input, _writer) = raw_source();
    let composer = input.composer.as_mut().unwrap();
    composer.replace(0..0, "/skills", 7).unwrap();
    assert!(matches!(
        composer.feed(b"\r", ComposerContext::default()).1,
        Some(ComposerEvent::Submit(_))
    ));
    input.open_skills_query("").unwrap();
    input.close_skills_query();
    assert!(
        matches!(input.composer.as_mut().unwrap().feed(b"\n", ComposerContext::default()).1, Some(ComposerEvent::Submit(text)) if text.is_empty())
    );
    finish(input);
}

#[test]
fn skills_query_close_discards_old_query_chunk_before_restored_draft() {
    let (mut input, mut writer) = raw_source();
    input
        .composer
        .as_mut()
        .unwrap()
        .replace(0..0, "draft", 3)
        .unwrap();
    let picker = NativeSkillPicker::new("draft".into(), 3).unwrap();
    input.open_skills_query("").unwrap();
    runtime().block_on(async {
        writer.write_all(b"query\r").unwrap();
        assert!(matches!(
            next(
                &mut input,
                &binding(&picker, true),
                Some(NativeSkillPickerMode::Menu)
            )
            .await
            .0,
            ComposerEvent::Changed
        ));
        input.close_skills_query();
        assert!(matches!(
            next(&mut input, &binding(&picker, false), None).await.0,
            ComposerEvent::StaleInput
        ));
        assert_eq!(input.raw_draft(), Some(("draft", 3)));
        writer.write_all(b"X").unwrap();
        assert!(matches!(
            next(&mut input, &binding(&picker, false), None).await.0,
            ComposerEvent::Changed
        ));
        assert_eq!(input.raw_draft(), Some(("draXft", 4)));
    });
    finish(input);
}

#[test]
fn skills_fresh_identical_draft_cannot_consume_previous_chunk_submit() {
    let (mut input, mut writer) = raw_source();
    let mut picker = NativeSkillPicker::new(String::new(), 0).unwrap();
    let old = binding(&picker, false);
    runtime().block_on(async {
        writer.write_all(b"same\r").unwrap();
        assert!(matches!(
            next(&mut input, &old, None).await.0,
            ComposerEvent::Changed
        ));
        picker.reset("same".into(), 4).unwrap();
        assert!(matches!(
            next(&mut input, &binding(&picker, false), None).await.0,
            ComposerEvent::StaleInput
        ));
        assert_eq!(input.raw_draft(), Some(("same", 4)));
    });
    finish(input);
}

#[test]
fn skills_partial_paste_cannot_be_parked_or_relabelled_as_query() {
    let (mut input, mut writer) = raw_source();
    let picker = NativeSkillPicker::new(String::new(), 0).unwrap();
    let old = binding(&picker, false);
    runtime().block_on(async {
        writer.write_all(b"\x1b[200~partial").unwrap();
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                assert!(
                    input
                        .poll_event(cx, old.clone(), ComposerContext::default())
                        .is_pending()
                );
                if input.chunk.is_none() && input.composer.as_ref().unwrap().has_pending_input() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
        assert!(input.open_skills_query("query").is_err());
        assert_eq!(input.original_draft(), Some(("", 0)));
        writer.write_all(b" paste\x1b[201~").unwrap();
        let mut edits = Vec::new();
        let event = tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                input.poll_event_observed(
                    cx,
                    old.clone(),
                    ComposerContext::default(),
                    |_| {},
                    |identity, range, inserted, cursor| {
                        assert!(*identity == old);
                        edits.push((range, inserted.to_owned(), cursor));
                    },
                )
            }),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert!(matches!(event.0, ComposerEvent::Changed));
        assert_eq!(edits, [(0..0, "partial paste".into(), 13)]);
    });
    finish(input);
}

#[test]
fn skills_query_limit_is_1024_not_session_picker_256() {
    let (mut input, mut writer) = raw_source();
    let picker = NativeSkillPicker::new(String::new(), 0).unwrap();
    assert!(input.open_skills_query(&"x".repeat(1025)).is_err());
    input.open_skills_query("").unwrap();
    runtime().block_on(async {
        writer.write_all("x".repeat(1024).as_bytes()).unwrap();
        assert!(matches!(
            next(
                &mut input,
                &binding(&picker, true),
                Some(NativeSkillPickerMode::Menu)
            )
            .await
            .0,
            ComposerEvent::Changed
        ));
        writer.write_all(b"y").unwrap();
        assert!(matches!(
            next(
                &mut input,
                &binding(&picker, true),
                Some(NativeSkillPickerMode::Menu)
            )
            .await
            .0,
            ComposerEvent::InputError(_)
        ));
        assert_eq!(input.raw_draft().unwrap().0.len(), 1024);
        assert_eq!(input.original_draft(), Some(("", 0)));
    });
    finish(input);
}

#[test]
fn skills_enter_and_tab_never_take_draft_before_frame_selection() {
    for key in [b'\r', b'\n', b'\t'] {
        let mut composer = Composer::default();
        composer.replace(0..0, "prefix $ré suffix", 11).unwrap();
        let context = ComposerContext {
            skills: Some(NativeSkillPickerMode::Inline),
            ..ComposerContext::default()
        };
        let (used, event) = composer.feed(&[key], context);
        assert_eq!(used, 1);
        assert!(matches!(event, Some(ComposerEvent::SkillSelected)));
        assert_eq!(composer.text(), "prefix $ré suffix");
        assert_eq!(composer.cursor(), 11);
    }
}
