use super::{McpRefreshError, Result};
use crate::mcp::protocol::{RpcEnvelope, RpcId};

/// Checks a modern listen request's terminal success, not its acknowledgement.
/// This borrows an already admitted envelope and grants no subscription authority.
/// # Errors
/// Rejects non-integer/negative expected IDs, foreign responses, errors and
/// missing or mismatched complete-result subscription metadata.
pub fn validate_subscription_response(envelope: &RpcEnvelope, expected: &RpcId) -> Result<()> {
    let RpcId::Integer(expected_id) = expected else {
        return Err(McpRefreshError::Invalid);
    };
    if *expected_id < 0 || envelope.id() != Some(expected) {
        return Err(McpRefreshError::Invalid);
    }
    let result = envelope.result().ok_or(McpRefreshError::Invalid)?;
    if result.get("resultType").and_then(serde_json::Value::as_str) != Some("complete")
        || result
            .get("_meta")
            .and_then(|meta| meta.get("io.modelcontextprotocol/subscriptionId"))
            .and_then(serde_json::Value::as_i64)
            != Some(*expected_id)
    {
        return Err(McpRefreshError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::protocol::{WireLimits, parse_envelope};

    #[test]
    fn terminal_success_requires_exact_integer_ids_and_complete_metadata() {
        let expected = RpcId::Integer(7);
        let valid = br#"{"jsonrpc":"2.0","id":7,"result":{"resultType":"complete","_meta":{"io.modelcontextprotocol/subscriptionId":7}}}"#;
        let envelope = parse_envelope(valid, WireLimits::default()).unwrap();
        assert!(validate_subscription_response(&envelope, &expected).is_ok());
        assert!(validate_subscription_response(&envelope, &RpcId::Integer(8)).is_err());
        assert!(validate_subscription_response(&envelope, &RpcId::Integer(-1)).is_err());
        assert!(validate_subscription_response(&envelope, &RpcId::String("7".into())).is_err());
        for body in [
            r#"{"jsonrpc":"2.0","id":7,"result":{}}"#,
            r#"{"jsonrpc":"2.0","id":7,"result":{"resultType":"complete"}}"#,
            r#"{"jsonrpc":"2.0","id":7,"result":{"resultType":"input_required","_meta":{"io.modelcontextprotocol/subscriptionId":7}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"result":{"resultType":"complete","_meta":{"io.modelcontextprotocol/subscriptionId":8}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"result":{"resultType":"complete","_meta":{"io.modelcontextprotocol/subscriptionId":"7"}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"result":{"resultType":"complete","_meta":{"io.modelcontextprotocol/subscriptionId":7.0}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"error":{"code":-1,"message":"cancelled"}}"#,
        ] {
            let envelope = parse_envelope(body.as_bytes(), WireLimits::default()).unwrap();
            assert!(validate_subscription_response(&envelope, &expected).is_err());
        }
    }
}
