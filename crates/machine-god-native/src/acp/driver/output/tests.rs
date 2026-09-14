use super::*;
use crate::acp::resources::NativeAcpResourceContextError;
use crate::{NativeConversationError, NativeConversationRuntimeError, NativeInteractiveError};

#[test]
fn settled_resource_cancellation_is_a_cancelled_prompt_result() {
    let outcome = NativeInteractiveError::Runtime(NativeConversationRuntimeError::Resources(
        NativeAcpResourceContextError::Cancelled,
    ));
    assert_eq!(
        stop_result(Err(&outcome)).unwrap(),
        json!({"stopReason":"cancelled"})
    );
}

#[test]
fn unrelated_settled_native_failures_remain_rpc_errors() {
    for outcome in [
        NativeInteractiveError::Runtime(NativeConversationRuntimeError::Resources(
            NativeAcpResourceContextError::WorkerUnavailable,
        )),
        NativeInteractiveError::Runtime(NativeConversationRuntimeError::Conversation(
            NativeConversationError::Persistence,
        )),
        NativeInteractiveError::Conversation(NativeConversationError::Persistence),
        NativeInteractiveError::Runtime(NativeConversationRuntimeError::Retired),
        NativeInteractiveError::Unavailable,
        NativeInteractiveError::Closed,
    ] {
        let error = stop_result(Err(&outcome)).unwrap_err();
        assert_eq!(error.code, -32603);
        assert_eq!(error.message, "ACP native turn failed");
        assert!(error.data.is_none());
    }
}

#[test]
fn settled_engine_completion_keeps_its_actual_stop_reason() {
    for (reason, expected) in [
        (StopReason::Completed, "end_turn"),
        (StopReason::Cancelled, "cancelled"),
        (StopReason::MaxOutputTokens, "max_tokens"),
        (StopReason::ContentFilter, "refusal"),
    ] {
        let event = TurnEvent::Completed {
            reason,
            usage: Default::default(),
        };
        assert_eq!(
            stop_result(Ok(&event)).unwrap(),
            json!({"stopReason":expected})
        );
    }
    assert_eq!(
        stop_result(Ok(&TurnEvent::Started)).unwrap_err().code,
        -32603
    );
    let failure = TurnEvent::Failed {
        component: "store".into(),
        code: "cancelled".into(),
        message: "cancellation intent is not a settled cancellation receipt".into(),
        retryable: false,
    };
    assert_eq!(stop_result(Ok(&failure)).unwrap_err().code, -32603);
}
