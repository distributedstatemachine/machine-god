//! Persistable exact-action identities, never credentials or execution tokens.

use crate::{
    MAX_NATIVE_PERMISSION_IDENTITY_BYTES, NativePermissionRuleKey, NativePermissionRuleKind,
};
use sha2::{Digest, Sha256};

/// The runtime fingerprint deliberately has a distinct domain from the rule
/// key's own digest. Each raw binding component is length-framed before hashing;
/// neither configuration nor authentication bytes enter persistent metadata.
pub(super) fn runtime_fingerprint(fields: &[&[u8]]) -> String {
    fingerprint(b"machine-god-mcp-runtime-binding-v1\0", fields)
}

fn fingerprint(domain: &[u8], fields: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    hash.update(domain);
    for field in fields {
        hash.update(
            u64::try_from(field.len())
                .expect("bounded MCP binding")
                .to_be_bytes(),
        );
        hash.update(field);
    }
    format!("{:x}", hash.finalize())
}

pub(super) fn key(
    workspace: &str,
    server: &str,
    exposed: &str,
    remote: &str,
    runtime: &str,
    arguments: &str,
) -> Option<NativePermissionRuleKey> {
    let mut framed = String::new();
    for field in [
        "machine-god-mcp-permission-v1",
        workspace,
        server,
        exposed,
        remote,
        runtime,
    ] {
        let prefix = field.len().to_string();
        let length = framed
            .len()
            .checked_add(prefix.len())?
            .checked_add(1)?
            .checked_add(field.len())?;
        if length > MAX_NATIVE_PERMISSION_IDENTITY_BYTES {
            return None;
        }
        framed.push_str(&prefix);
        framed.push(':');
        framed.push_str(field);
    }
    // Preserve the full exact-argument-frame eligibility bound even though the
    // persisted representation is a digest. Arbitrary arguments may themselves
    // contain credentials; they must not be copied to saved-rule metadata.
    let argument_frame = arguments.len().to_string();
    let raw_length = framed
        .len()
        .checked_add(argument_frame.len())?
        .checked_add(1)?
        .checked_add(arguments.len())?;
    if raw_length > MAX_NATIVE_PERMISSION_IDENTITY_BYTES {
        return None;
    }
    let digest = fingerprint(b"machine-god-mcp-arguments-v1\0", &[arguments.as_bytes()]);
    framed.push_str(&digest.len().to_string());
    framed.push(':');
    framed.push_str(&digest);
    NativePermissionRuleKey::new(NativePermissionRuleKind::StructuredTool, &framed).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_length_framed_and_every_binding_field_matters() {
        assert_ne!(
            runtime_fingerprint(&[b"ab", b"c"]),
            runtime_fingerprint(&[b"a", b"bc"])
        );
        let baseline = [b"config".as_slice(), b"schema", b"credential"];
        let original = runtime_fingerprint(&baseline);
        for index in 0..baseline.len() {
            let mut changed = baseline;
            changed[index] = b"different";
            assert_ne!(original, runtime_fingerprint(&changed));
        }
        assert!(!original.contains("credential"));
        assert_eq!(original.len(), 64);
    }

    #[test]
    fn frames_do_not_collide_and_overflow_only_disables_reuse() {
        assert_ne!(
            key("/root", "ab", "tool", "c", "hash", "{}"),
            key("/root", "a", "tool", "bc", "hash", "{}")
        );
        let exact = key("/root", "server", "tool", "remote", "hash", "{\"x\":-0}").unwrap();
        assert_ne!(
            Some(exact),
            key("/root", "server", "tool", "remote", "hash", "{\"x\":0}")
        );
        assert!(
            key(
                "/root",
                "server",
                "tool",
                "remote",
                "hash",
                &"x".repeat(4096)
            )
            .is_none()
        );
        let largest = (0..4096)
            .rev()
            .find_map(|size| {
                key(
                    "/root",
                    "server",
                    "tool",
                    "remote",
                    "hash",
                    &"x".repeat(size),
                )
                .map(|key| (size, key))
            })
            .unwrap();
        assert!(largest.1.canonical().len() < 4096);
        assert!(
            key(
                "/root",
                "server",
                "tool",
                "remote",
                "hash",
                &"x".repeat(largest.0 + 1)
            )
            .is_none()
        );
        assert!(
            !key(
                "/root",
                "server",
                "tool",
                "remote",
                "hash",
                r#"{"password":"secret-value"}"#
            )
            .unwrap()
            .canonical()
            .contains("secret-value")
        );
    }
}
