use super::super::interactive::support;
use super::super::{AskSignalControl, DenyPermissionPrompter, OutputAcknowledgement, OutputWork};
use super::*;
use machine_god_core::{CancellationToken, ManagedAgentState, ManagedSubagentCommand};
use machine_god_native::{NativeConversation, NativeSessionMetadata, NativeSessionOrigin};
use std::{fs::File, future::Future, time::Duration};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn fixture() -> support::Fixture {
    support::Fixture::with_workspace_options_and_prompter(
        |options| options.with_managed_agents(managed_startup::base_options()),
        Arc::new(DenyPermissionPrompter),
    )
}

async fn agents(fixture: &mut support::Fixture) -> NativeManagedAgents {
    let state = File::open(fixture.state_root()).unwrap();
    let host = Arc::get_mut(&mut fixture.host).unwrap();
    let preferences = host.loaded_config().config().model_preferences();
    host.open_workspace_managed_agents(state.into(), preferences, NativeSessionOrigin::Cli)
        .await
        .unwrap()
}

fn execution(resume: bool) {
    let runtime = runtime();
    let mut fixture = fixture();
    fixture.transport.push(support::answer());
    let selection = if resume {
        let conversation = runtime
            .block_on(NativeConversation::create(
                fixture.host.session_lifecycle(),
                NativeSessionMetadata::new(&fixture.workspace, 1, NativeSessionOrigin::Cli)
                    .unwrap(),
            ))
            .unwrap();
        SessionSelection::Resume(conversation.id())
    } else {
        SessionSelection::CreateGenerated
    };
    let agents = runtime.block_on(agents(&mut fixture));
    std::thread::scope(|scope| {
        let (sender, mut guardian_commands) = tokio::sync::mpsc::channel(1);
        let control = AskSignalControlSender { sender };
        let guardian = scope.spawn(move || {
            let Some(AskSignalControl::ActivateTurn(ready)) = guardian_commands.blocking_recv()
            else {
                panic!("one-shot turn must activate the signal guardian");
            };
            ready.send(()).unwrap();
            assert!(guardian_commands.blocking_recv().is_none());
        });
        let (_signal, received) = tokio::sync::mpsc::channel(1);
        let mut signals = AskSignals::new(received);
        let (work, mut output) = tokio::sync::mpsc::channel(1);
        let (ack, acknowledgements) = tokio::sync::mpsc::channel(1);
        let mut bytes = Vec::new();
        let mut flushes = 0;
        let result = runtime.block_on(async {
            let operation = execute(
                fixture.host.clone(),
                agents,
                Setup {
                    selection,
                    prompt: "one managed prompt".into(),
                    workspace: fixture.workspace.clone(),
                    catalog: None,
                },
                OutputBridge {
                    work,
                    acknowledgements,
                    tape: None,
                },
                &mut signals,
                &control,
            );
            let mut operation = std::pin::pin!(operation);
            tokio::time::timeout(
                Duration::from_secs(20),
                poll_fn(|cx| {
                    let result = operation.as_mut().poll(cx);
                    if let Poll::Ready(Some(item)) = output.poll_recv(cx) {
                        match item {
                            OutputWork::Write(text) => bytes.extend(text),
                            OutputWork::Flush => flushes += 1,
                        }
                        ack.try_send(OutputAcknowledgement::Succeeded).unwrap();
                    }
                    result
                }),
            )
            .await
            .unwrap()
            .unwrap()
        });
        assert_eq!(result.outcome, AskCommandOutcome::Completed);
        assert_eq!(bytes, b"complete");
        assert_eq!(flushes, 1);
        assert!(signals.first_observed.is_none());
        drop(control);
        guardian.join().unwrap();
    });
    assert_eq!(fixture.transport.requests().len(), 1);
    fixture.finish();
}

#[test]
fn ask_executes_and_settles_the_actual_managed_owner() {
    execution(false);
}

#[test]
fn resume_executes_and_settles_the_actual_managed_owner() {
    execution(true);
}

#[test]
fn one_shot_settlement_retries_a_failed_original_clear_without_more_input() {
    let runtime = runtime();
    let mut fixture = fixture();
    runtime.block_on(async {
        let agents = agents(&mut fixture).await;
        let options = NativeInteractiveSessionOptions::new(
            fixture.workspace.clone(),
            fixture.host.loaded_config().config().model_preferences(),
        )
        .unwrap();
        let startup =
            NativeManagedInteractiveStartup::new(fixture.host.clone(), options, agents).unwrap();
        let (_signal, received) = tokio::sync::mpsc::channel(1);
        let mut signals = AskSignals::new(received);
        let mut owner = managed_startup::open(
            startup,
            NativeInteractiveInitialSession::Fresh,
            &mut signals,
        )
        .await
        .unwrap()
        .unwrap();
        let blocked = fixture.fence_notice_clear(&mut owner).await;
        let providers = fixture.transport.requests().len();
        let mut settlement = Box::pin(settle(&mut owner, &mut signals));
        let mut cx = Context::from_waker(std::task::Waker::noop());
        assert!(settlement.as_mut().poll(&mut cx).is_pending());
        drop(blocked);
        tokio::time::timeout(Duration::from_secs(10), settlement)
            .await
            .unwrap()
            .unwrap();
        assert!(owner.is_closed());
        assert_eq!(fixture.transport.requests().len(), providers);
        drop(owner);
    });
    fixture.finish();
}

#[test]
fn a_signal_before_first_admission_discards_the_queued_prompt_without_provider_work() {
    let runtime = runtime();
    let mut fixture = fixture();
    let agents = runtime.block_on(agents(&mut fixture));
    let (signal, received) = tokio::sync::mpsc::channel(1);
    let mut signals = AskSignals::new(received);
    let options = NativeInteractiveSessionOptions::new(
        fixture.workspace.clone(),
        fixture.host.loaded_config().config().model_preferences(),
    )
    .unwrap()
    .with_required_mcp_startup();
    let startup =
        NativeManagedInteractiveStartup::new(fixture.host.clone(), options, agents).unwrap();
    let mut owner = runtime
        .block_on(managed_startup::open(
            startup,
            NativeInteractiveInitialSession::Fresh,
            &mut signals,
        ))
        .unwrap()
        .unwrap();
    owner
        .enqueue("must never reach the provider".into())
        .unwrap();
    signal.try_send(AskSignal::Terminate).unwrap();
    let (work, _output) = tokio::sync::mpsc::channel(1);
    let (_ack, acknowledgements) = tokio::sync::mpsc::channel(1);
    let result = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(20),
            drive(
                owner,
                &mut signals,
                OutputBridge {
                    work,
                    acknowledgements,
                    tape: None,
                },
            ),
        )
        .await
        .unwrap()
        .unwrap()
    });
    assert_eq!(result.outcome, AskCommandOutcome::Terminated);
    assert!(fixture.transport.requests().is_empty());
    fixture.finish();
}

#[test]
fn blocked_stdout_keeps_hidden_child_progress_and_signal_cleanup() {
    let runtime = runtime();
    let mut fixture = fixture();
    fixture.transport.push(support::answer());
    // This child needs human permission. The one-shot host must deny it without
    // parking an inaccessible prompt, while the parent writer remains blocked.
    fixture.transport.push(support::call(
        "write_file",
        &serde_json::json!({"path":"denied.txt","content":"no"}),
    ));
    fixture.transport.push(support::answer());
    let agents = runtime.block_on(agents(&mut fixture));
    let (signal, received) = tokio::sync::mpsc::channel(1);
    let mut signals = AskSignals::new(received);
    let options = NativeInteractiveSessionOptions::new(
        fixture.workspace.clone(),
        fixture.host.loaded_config().config().model_preferences(),
    )
    .unwrap()
    .with_required_mcp_startup();
    let startup =
        NativeManagedInteractiveStartup::new(fixture.host.clone(), options, agents).unwrap();
    let mut owner = runtime
        .block_on(managed_startup::open(
            startup,
            NativeInteractiveInitialSession::Fresh,
            &mut signals,
        ))
        .unwrap()
        .unwrap();
    owner.enqueue("parent answer".into()).unwrap();
    let owner = RefCell::new(owner);
    let mut events = Events {
        owner: &owner,
        ended: false,
    };
    let mut progress = Progress {
        owner: &owner,
        signals: &mut signals,
        signalled: false,
    };
    let (work, mut output) = tokio::sync::mpsc::channel(1);
    let (_ack, acknowledgements) = tokio::sync::mpsc::channel(1);
    let result = runtime.block_on(async {
        let operation = drive_turn_stream(
            &mut events, || owner.borrow_mut().request_shutdown(), &mut progress,
            OutputBridge { work, acknowledgements, tape: None },
        );
        let mut operation = std::pin::pin!(operation);
        let mut requested = false;
        let mut response = None;
        let mut accepted = false;
        let mut interrupted = false;
        tokio::time::timeout(Duration::from_secs(20), poll_fn(|cx| {
            let result = operation.as_mut().poll(cx);
            if !requested && let Poll::Ready(Some(OutputWork::Write(bytes))) = output.poll_recv(cx) {
                assert_eq!(bytes, b"complete");
                let command = ManagedSubagentCommand::decode(serde_json::json!({"command":{"create":{
                    "name":"hidden-worker", "mode":"persistent", "prompt":"independent child"
                }}})).unwrap();
                response = Some(owner.borrow_mut().request_managed_command(command, CancellationToken::new()).unwrap());
                requested = true;
            }
            if let Some(pending) = &mut response && let Poll::Ready(value) = pending.as_mut().poll(cx) {
                assert!(value.unwrap().ok);
                accepted = true;
                response = None;
            }
            if !interrupted && accepted && owner.borrow().managed_agents().first().is_some_and(|child| child.state == ManagedAgentState::Idle) {
                assert!(result.is_pending(), "the writer still has no ACK");
                assert_eq!(fixture.transport.requests().len(), 3);
                assert!(!fixture.workspace.join("denied.txt").exists());
                signal.try_send(AskSignal::Interrupt).unwrap();
                interrupted = true;
            }
            if result.is_ready() { assert!(interrupted); }
            result
        })).await.unwrap()
    });
    assert_eq!(result.outcome, AskCommandOutcome::Interrupted);
    assert!(result.stalled_output_after_signal);
    let mut owner = owner.into_inner();
    runtime.block_on(settle(&mut owner, &mut signals)).unwrap();
    assert!(owner.is_closed());
    drop(owner);
    fixture.finish();
}
