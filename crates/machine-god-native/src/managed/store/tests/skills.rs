use super::*;

fn reference() -> crate::NativeSkillReference {
    serde_json::from_value(serde_json::json!({
        "version": 1, "name": "selected", "location": "/skills/selected/SKILL.md",
        "revision": ([0; 32])
    }))
    .unwrap()
}

#[test]
fn accepted_skill_references_survive_journal_reopen_without_becoming_authority() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let mut record = create("skills");
    record.initial_work.as_mut().unwrap().skills = vec![reference()];
    let snapshot = confirmed(block_on(journal.create(record)).unwrap());
    let page = snapshot.head.queue[0].page.clone();
    drop((snapshot, journal));
    let reopened = fixture.open();
    let work = block_on(reopened.read_work(page)).unwrap();
    assert_eq!(work.skills, vec![reference()]);
    assert_eq!(work.content, "standalone task");
    assert!(
        crate::NativeSkillCatalog::new(vec![])
            .unwrap()
            .resolve_reference(
                &crate::NativeSkillCatalog::new(vec![])
                    .unwrap()
                    .discover(&machine_god_core::CancellationToken::new())
                    .unwrap(),
                &work.skills[0],
            )
            .is_err()
    );
}

#[test]
fn skill_reference_limits_apply_to_typed_and_decoded_work() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let mut record = create("too-many-skills");
    let work = record.initial_work.as_mut().unwrap();
    work.skills = vec![reference(); 17];
    let encoded = serde_json::to_vec(&work).unwrap();
    assert!(serde_json::from_slice::<JournalWork>(&encoded).is_err());
    assert!(matches!(
        block_on(journal.create(record)),
        Err(JournalError::Limit)
    ));

    let mut work = super::work("bounded");
    work.skills = vec![reference(); 16];
    assert!(serde_json::from_value::<JournalWork>(serde_json::to_value(&work).unwrap()).is_ok());
    let mut missing = serde_json::to_value(work).unwrap();
    missing.as_object_mut().unwrap().remove("skills");
    assert!(serde_json::from_value::<JournalWork>(missing).is_err());
}

fn escaped_work(id: &str) -> JournalWork {
    let reference: crate::NativeSkillReference = serde_json::from_value(serde_json::json!({
        "version": 1,
        "name": "s",
        "location": format!("/{}", "\u{1}".repeat(4090)),
        "revision": ([0; 32]),
    }))
    .unwrap();
    let mut work = super::work(id);
    work.content = "\u{1}".repeat(65_536);
    work.skills = vec![reference; 16];
    crate::skills_invocation::validate_references(&work.skills).unwrap();
    let record = JournalRecord::WorkAccepted(work.clone());
    let bytes = serde_json::to_vec(&record).unwrap().len();
    assert!(bytes > 512 * 1024);
    assert!(bytes < JournalLimits::default().page_bytes);
    work
}

#[test]
fn accepted_escaped_skill_work_remains_losslessly_pageable_after_reopen() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let first = escaped_work("work-1");
    let second = escaped_work("work-2");
    let mut create = create("escaped-skills");
    create.initial_work = Some(first.clone());
    let initial = confirmed(block_on(journal.create(create)).unwrap());
    let snapshot = confirmed(
        block_on(journal.mutate(initial, JournalMutation::Enqueue(second.clone()))).unwrap(),
    );
    for (reference, expected) in snapshot.head.queue.iter().zip([&first, &second]) {
        assert_eq!(
            block_on(journal.read_work(reference.page.clone())).unwrap(),
            *expected
        );
    }
    drop((snapshot, journal));
    let journal = fixture.open();
    let snapshot = block_on(journal.inspect("escaped-skills".into())).unwrap();
    let mut cursor = None;
    let mut works = Vec::new();
    let mut controls = Vec::new();
    let mut complete = false;
    for _ in 0..10 {
        let page = block_on(journal.history(snapshot.clone(), cursor, 100)).unwrap();
        assert!(!page.records.is_empty());
        assert!(page.records.len() <= 100);
        let bytes: usize = page
            .records
            .iter()
            .map(|record| serde_json::to_vec(record).unwrap().len())
            .sum();
        assert!(bytes <= 512 * 1024 || page.records.len() == 1);
        assert!(bytes <= JournalLimits::default().page_bytes);
        for record in page.records {
            match record {
                JournalRecord::WorkAccepted(work) => works.push(work),
                JournalRecord::Control(control) => controls.push(control.revision),
                _ => {}
            }
        }
        cursor = page.next;
        if cursor.is_none() {
            complete = true;
            break;
        }
    }
    assert!(
        complete,
        "history cursor did not finish the accepted records"
    );
    assert_eq!(works, vec![second, first]);
    assert_eq!(controls, vec![2, 1]);
}
