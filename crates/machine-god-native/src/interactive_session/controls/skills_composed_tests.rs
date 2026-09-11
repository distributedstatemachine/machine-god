use super::*;
use crate::{NativeSkillsCommand, NativeSkillsServiceError, NativeSkillsServiceResult};

#[test]
fn composed_skills_control_is_inert_and_preserves_mutation_receipts() {
    executor().block_on(async {
        let fixture = Fixture::new_with_skills();
        let mut session = owner(&fixture).await;
        let service = fixture.host.skills().unwrap();
        let NativeSkillsServiceResult::Path(path) = service
            .execute(
                NativeSkillsCommand::Path,
                &fixture.workspace,
                &CancellationToken::new(),
            )
            .unwrap()
        else {
            panic!("path")
        };
        assert!(!path.exists());
        let id = session
            .request_control(
                Control::Skills {
                    command: "create review".parse().unwrap(),
                },
                200,
            )
            .unwrap();
        assert!(!path.exists());
        let receipt = control_outcome(&mut session).await;
        assert_eq!(receipt.id, id);
        assert!(!receipt.failed());
        assert!(matches!(
            receipt.result,
            Ok(Receipt::Skills(NativeSkillsServiceResult::Managed(_)))
        ));
        assert!(path.join("review/SKILL.md").is_file());
        for command in ["list", "show review", "path", "remove review"] {
            session
                .request_control(
                    Control::Skills {
                        command: command.parse().unwrap(),
                    },
                    210,
                )
                .unwrap();
            assert!(!control_outcome(&mut session).await.failed());
        }
        assert!(!path.join("review").exists());
        assert!(fixture.transport.requests().is_empty());
        drop(service);
        close(session, fixture).await;
    });
}

#[test]
fn composed_skills_cancel_before_poll_is_retained_and_does_not_publish() {
    executor().block_on(async {
        let fixture = Fixture::new_with_skills();
        let mut session = owner(&fixture).await;
        let id = session
            .request_control(
                Control::Skills {
                    command: "create cancelled".parse().unwrap(),
                },
                200,
            )
            .unwrap();
        assert!(session.request_cancel());
        let receipt = control_outcome(&mut session).await;
        assert_eq!(receipt.id, id);
        assert!(receipt.failed());
        assert!(matches!(
            receipt.result,
            Err(ControlError::Skills(NativeSkillsServiceError::Cancelled))
        ));
        assert!(fixture.transport.requests().is_empty());
        close(session, fixture).await;
    });
}

#[test]
fn composed_skills_missing_authority_and_oversized_variants_do_not_reserve_control() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        assert!(matches!(
            session.request_control(
                Control::Skills {
                    command: NativeSkillsCommand::List
                },
                200
            ),
            Err(NativeInteractiveError::Configuration)
        ));
        assert!(session.control.is_none());
        close(session, fixture).await;
        let fixture = Fixture::new_with_skills();
        let mut session = owner(&fixture).await;
        assert!(matches!(
            session.request_control(
                Control::Skills {
                    command: NativeSkillsCommand::Install {
                        arguments: "x".repeat(crate::MAX_NATIVE_SKILLS_COMMAND_BYTES + 1)
                    }
                },
                200
            ),
            Err(NativeInteractiveError::Configuration)
        ));
        assert!(session.control.is_none());
        close(session, fixture).await;
    });
}
