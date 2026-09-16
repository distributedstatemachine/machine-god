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
