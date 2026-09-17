use super::*;
use crate::ask::production::interactive::composer::AgentMenu;

async fn menu_event(
    input: &mut InputLines,
    binding: InputBinding,
    menu: AgentMenu,
) -> (ComposerEvent, InputBinding) {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            input.poll_event(
                cx,
                binding.clone(),
                ComposerContext {
                    agent_menu: Some(menu),
                    ..ComposerContext::default()
                },
            )
        }),
    )
    .await
    .unwrap()
    .expect("event before EOF")
    .expect("valid native input")
}

#[test]
fn hidden_agent_menus_preserve_full_modal_answer_input() {
    for menu in [AgentMenu::Models, AgentMenu::Skills] {
        for binding in [
            prompt_binding(),
            InputBinding::SavedRule(42),
            InputBinding::AwaitingPrompt,
        ] {
            for bytes in [
                257,
                machine_god_native::MAX_ASK_USER_QUESTION_RAW_ANSWER_BYTES,
            ] {
                let (mut input, mut write) = raw_source();
                runtime().block_on(async {
                    let text = format!("other {}", "x".repeat(bytes));
                    write
                        .write_all(format!("\x1b[200~{text}\x1b[201~").as_bytes())
                        .unwrap();
                    let result = menu_event(&mut input, binding.clone(), menu).await;
                    assert!(
                        matches!(result.0, ComposerEvent::Changed),
                        "{menu:?}: {:?}",
                        result.0
                    );
                    assert!(result.1 == binding);
                    assert_eq!(input.raw_draft().unwrap().0, text);
                    write.write_all(b"\r").unwrap();
                    let result = menu_event(&mut input, binding.clone(), menu).await;
                    assert!(matches!(result.0, ComposerEvent::Submit(answer) if answer == text));
                    assert!(result.1 == binding);
                    assert_eq!(input.raw_draft().unwrap(), ("", 0));
                });
                finish(input);
            }
        }
    }
}

#[test]
fn hidden_agent_menus_do_not_keep_previous_question_answers_or_capture_ctrl_j() {
    for menu in [AgentMenu::Models, AgentMenu::Skills] {
        let first = prompt_binding();
        let InputBinding::Prompt { token, .. } = &first else {
            unreachable!()
        };
        let second = InputBinding::Prompt {
            token: token.clone(),
            question: 1,
        };
        let (mut input, mut write) = raw_source();
        runtime().block_on(async {
            for (binding, terminator) in [(first, b'\r'), (second, b'\n')] {
                write.write_all(b"1").unwrap();
                assert!(matches!(
                    menu_event(&mut input, binding.clone(), menu).await.0,
                    ComposerEvent::Changed
                ));
                write.write_all(&[terminator]).unwrap();
                let result = menu_event(&mut input, binding.clone(), menu).await;
                assert!(matches!(result.0, ComposerEvent::Submit(answer) if answer == "1"));
                assert!(result.1 == binding);
                assert_eq!(input.raw_draft().unwrap(), ("", 0));
            }
        });
        finish(input);
    }
}
