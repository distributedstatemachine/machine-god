use machine_god_core::{SessionId, SessionIncarnationId, ToolCallId};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub(crate) const READ_TOOL_RESULT_TOOL_NAME: &str = "read_tool_result";
pub(crate) const TOOL_RESULT_PROJECTION_THRESHOLD_BYTES: usize = 16_384;
pub(crate) const TOOL_RESULT_PROJECTION_MAX_BYTES: usize = 4_096;

const TOOL_RESULT_HANDLE_PREFIX: &str = "tool-result-sha256-";
const TOOL_RESULT_HANDLE_DOMAIN: &[u8] = b"machine-god/tool-result-handle/v1\0";

#[derive(Serialize)]
struct ProjectedToolResult<'a> {
    r#type: &'static str,
    handle: &'a str,
    total_bytes: usize,
    is_error: bool,
    read_more_with: &'static str,
    preview: &'a str,
}

pub(crate) fn project_tool_result(
    session_id: &SessionId,
    incarnation_id: &SessionIncarnationId,
    call_id: &ToolCallId,
    serialized_output: &[u8],
    is_error: bool,
) -> String {
    let source = std::str::from_utf8(serialized_output)
        .expect("compact serde_json ToolOutput serialization is valid UTF-8");
    let handle = tool_result_handle(session_id, incarnation_id, call_id, serialized_output);
    let preview_end = utf8_prefix(source, TOOL_RESULT_PROJECTION_MAX_BYTES);
    let projected = ProjectedToolResult {
        r#type: "tool_result_preview",
        handle: &handle,
        total_bytes: serialized_output.len(),
        is_error,
        read_more_with: READ_TOOL_RESULT_TOOL_NAME,
        preview: &source[..preview_end],
    };
    serde_json::to_string(&projected).expect("projected tool result has infallible serialization")
}

pub(crate) fn tool_result_handle(
    session_id: &SessionId,
    incarnation_id: &SessionIncarnationId,
    call_id: &ToolCallId,
    serialized_output: &[u8],
) -> String {
    let mut digest = Sha256::new();
    digest.update(TOOL_RESULT_HANDLE_DOMAIN);
    update_length_prefixed(&mut digest, session_id.as_str().as_bytes());
    update_length_prefixed(&mut digest, incarnation_id.as_str().as_bytes());
    update_length_prefixed(&mut digest, call_id.as_str().as_bytes());
    update_length_prefixed(&mut digest, serialized_output);
    format!("{TOOL_RESULT_HANDLE_PREFIX}{:x}", digest.finalize())
}

pub(crate) fn valid_tool_result_handle(handle: &str) -> bool {
    let Some(digest) = handle.strip_prefix(TOOL_RESULT_HANDLE_PREFIX) else {
        return false;
    };
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn update_length_prefixed(digest: &mut Sha256, bytes: &[u8]) {
    let length = u64::try_from(bytes.len()).expect("supported targets use at most 64-bit usize");
    digest.update(length.to_be_bytes());
    digest.update(bytes);
}

fn utf8_prefix(value: &str, max_bytes: usize) -> usize {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::{
        TOOL_RESULT_PROJECTION_MAX_BYTES, project_tool_result, tool_result_handle,
        valid_tool_result_handle,
    };
    use machine_god_core::{SessionId, SessionIncarnationId, ToolCallId};

    fn context() -> (SessionId, SessionIncarnationId, ToolCallId) {
        (
            SessionId::new("session-1").unwrap(),
            SessionIncarnationId::new("incarnation-1").unwrap(),
            ToolCallId::new("call-1").unwrap(),
        )
    }

    #[test]
    fn handles_are_exact_lowercase_and_domain_isolated() {
        let (session, incarnation, call) = context();
        let output = br#"{"content":"hello","is_error":false}"#;
        let handle = tool_result_handle(&session, &incarnation, &call, output);
        assert!(valid_tool_result_handle(&handle));
        assert_eq!(
            handle,
            "tool-result-sha256-25f46173de6f65550459be1d3692774f4b0a7c7e7034a9503586cfc508d10dae"
        );
        assert_eq!(
            handle,
            tool_result_handle(&session, &incarnation, &call, output)
        );
        assert_ne!(
            handle,
            tool_result_handle(
                &SessionId::new("session-2").unwrap(),
                &incarnation,
                &call,
                output,
            )
        );
        assert_ne!(
            handle,
            tool_result_handle(
                &session,
                &SessionIncarnationId::new("incarnation-2").unwrap(),
                &call,
                output,
            )
        );
        assert_ne!(
            handle,
            tool_result_handle(
                &session,
                &incarnation,
                &ToolCallId::new("call-2").unwrap(),
                output,
            )
        );
        assert_ne!(
            handle,
            tool_result_handle(&session, &incarnation, &call, b"different")
        );
        for invalid in [
            "",
            "tool-result-sha256-",
            "tool-result-sha256-0",
            "tool-result-sha256-gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg",
            "tool-result-sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "other-result-sha256-0000000000000000000000000000000000000000000000000000000000000000",
        ] {
            assert!(!valid_tool_result_handle(invalid), "accepted {invalid:?}");
        }
    }

    #[test]
    fn projected_value_is_bounded_and_uses_a_valid_utf8_prefix() {
        let (session, incarnation, call) = context();
        let source = format!(r#"{{"content":"{}","is_error":true}}"#, "🦀".repeat(8_000));
        let projected = project_tool_result(&session, &incarnation, &call, source.as_bytes(), true);
        let value: serde_json::Value = serde_json::from_str(&projected).unwrap();
        let preview = value["preview"].as_str().unwrap();
        assert!(preview.len() <= TOOL_RESULT_PROJECTION_MAX_BYTES);
        assert!(source.starts_with(preview));
        assert!(source.is_char_boundary(preview.len()));
        assert_eq!(value["type"], "tool_result_preview");
        assert_eq!(value["total_bytes"], source.len());
        assert_eq!(value["is_error"], true);
        assert_eq!(value["read_more_with"], "read_tool_result");
        assert!(valid_tool_result_handle(value["handle"].as_str().unwrap()));
    }
}
