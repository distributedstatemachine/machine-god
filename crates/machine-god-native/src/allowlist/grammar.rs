use super::{
    MAX_NATIVE_ALLOWLIST_REQUEST_BYTES, NativeAllowlistCommand as Command,
    NativeAllowlistParseError as Error, NativeAllowlistRequest, NativeAllowlistView as View,
};
use crate::{
    NativeConfiguredPermissionDecision, NativeConfiguredPermissionMutation as Mutation,
    NativeConfiguredPermissionReset as Reset, NativeConfiguredPermissionRule,
    NativeConfiguredPermissionScope as Scope,
};

pub(crate) fn parse(
    raw: &str,
    registered: impl Fn(&str) -> bool,
) -> Result<NativeAllowlistRequest, Error> {
    if raw.len() > MAX_NATIVE_ALLOWLIST_REQUEST_BYTES {
        return Err(Error::Limit);
    }
    let Some((first, rest)) = word(raw) else {
        return Ok(request(Command::View(View::Effective), None));
    };
    if first.eq_ignore_ascii_case("view") {
        let view = match trim(rest).to_ascii_lowercase().as_str() {
            "" | "effective" => View::Effective,
            "local" => View::Local,
            "user" => View::User,
            _ => return Err(Error::Invalid),
        };
        return Ok(request(Command::View(view), None));
    }
    let (scope, action, rest) =
        if first.eq_ignore_ascii_case("local") || first.eq_ignore_ascii_case("user") {
            let (action, rest) = word(rest).ok_or(Error::Invalid)?;
            (
                if first.eq_ignore_ascii_case("user") {
                    Scope::User
                } else {
                    Scope::Local
                },
                action,
                rest,
            )
        } else {
            (Scope::Local, first, rest)
        };
    let (mutation, tool) = if action.eq_ignore_ascii_case("reset") {
        let category = match trim(rest).to_ascii_lowercase().as_str() {
            "all" => Reset::All,
            "command" | "commands" => Reset::Commands,
            "tool" | "tools" => Reset::Tools,
            "url" | "urls" => Reset::Urls,
            "web-fetch-domain" | "web-fetch-domains" => Reset::WebFetchDomains,
            _ => return Err(Error::Invalid),
        };
        (Mutation::Reset(category), None)
    } else {
        let add = action.eq_ignore_ascii_case("add");
        if !add && !action.eq_ignore_ascii_case("remove") {
            return Err(Error::Invalid);
        }
        let (kind, tail) = word(rest).ok_or(Error::Invalid)?;
        let pattern = quoted_or_rest(tail);
        if pattern.is_empty() {
            return Err(Error::Invalid);
        }
        let (permission, pattern, tool) = match kind.to_ascii_lowercase().as_str() {
            "command" => ("bash".to_owned(), pattern.to_owned(), None),
            "url" => ("url".to_owned(), pattern.to_owned(), None),
            "web-fetch-domain" => ("web_fetch".to_owned(), canonical_domain(pattern)?, None),
            "tool" if pattern != "web_fetch" && known_tool(pattern, &registered) => (
                permission_name(pattern).to_owned(),
                "*".to_owned(),
                Some(pattern.to_owned()),
            ),
            _ => return Err(Error::Invalid),
        };
        // Native config keys trim ASCII whitespace, independently of the slash
        // parser's deliberately simple quote handling.
        let rule = NativeConfiguredPermissionRule::new(
            &permission,
            &pattern,
            NativeConfiguredPermissionDecision::Allow,
        )
        .map_err(|_| Error::Limit)?;
        let permission = rule.permission().to_owned();
        let pattern = rule.pattern().to_owned();
        (
            if add {
                Mutation::Add {
                    permission,
                    pattern,
                }
            } else {
                Mutation::Remove {
                    permission,
                    pattern,
                }
            },
            tool,
        )
    };
    Ok(request(Command::Mutate { scope, mutation }, tool))
}

fn request(command: Command, tool: Option<String>) -> NativeAllowlistRequest {
    NativeAllowlistRequest { command, tool }
}
fn trim(raw: &str) -> &str {
    raw.trim_matches([' ', '\t'])
}
fn word(raw: &str) -> Option<(&str, &str)> {
    let raw = trim(raw);
    if raw.is_empty() {
        return None;
    }
    Some(raw.find([' ', '\t']).map_or((raw, ""), |index| {
        (
            &raw[..index],
            raw[index + 1..].trim_start_matches([' ', '\t']),
        )
    }))
}
fn quoted_or_rest(raw: &str) -> &str {
    let raw = trim(raw);
    match raw.strip_prefix('"') {
        Some(raw) => raw.split_once('"').map_or(raw, |(first, _)| first),
        None => raw,
    }
}
pub(super) fn known_tool(tool: &str, registered: &impl Fn(&str) -> bool) -> bool {
    registered(tool)
        || matches!(
            tool,
            "edit"
                | "create_folder"
                | "open_file"
                | "rename_file"
                | "copy_file"
                | "read"
                | "list"
                | "glob"
                | "grep"
                | "skill"
                | "memory"
                | "semantic_search"
                | "web_search"
        )
}
fn permission_name(tool: &str) -> &str {
    match tool {
        "read_file" => "read",
        "write_file" | "edit_file" => "edit",
        "list_files" => "list",
        "glob_files" => "glob",
        "grep_files" => "grep",
        "run_command" => "bash",
        "install_skill" => "skill",
        _ => tool,
    }
}

fn canonical_domain(raw: &str) -> Result<String, Error> {
    let host = raw.strip_prefix("domain:").unwrap_or(raw);
    let host = if host.len() > 1 {
        host.strip_suffix('.').unwrap_or(host)
    } else {
        host
    };
    let valid = if let Some(inner) = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
    {
        !inner.is_empty()
            && inner.bytes().all(|byte| {
                byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
                    || matches!(byte, b':' | b'.')
            })
            && inner.parse::<std::net::Ipv6Addr>().is_ok()
    } else {
        !host.is_empty()
            && host.split('.').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
    };
    if !valid {
        return Err(Error::Invalid);
    }
    Ok(format!("domain:{}", host.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests;
