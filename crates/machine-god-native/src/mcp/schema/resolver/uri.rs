//! Local identifier resolution matching the producer's Zig 0.16 URI behavior.
use super::{Error, Result};

mod authority;
use authority::Authority;

struct Parts<'a> {
    scheme: Option<&'a str>,
    authority: Option<Authority<'a>>,
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}
impl<'a> Parts<'a> {
    fn absolute(text: &'a str) -> Result<Self> {
        let (scheme, rest) = text.split_once(':').ok_or(Error::InvalidSchema)?;
        if !scheme
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
            || !scheme
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"+-.".contains(&byte))
        {
            return Err(Error::InvalidSchema);
        }
        Self::after_scheme(Some(scheme), rest)
    }
    fn after_scheme(scheme: Option<&'a str>, text: &'a str) -> Result<Self> {
        let (text, fragment) = text
            .split_once('#')
            .map_or((text, None), |(head, tail)| (head, Some(tail)));
        let (text, query) = text
            .split_once('?')
            .map_or((text, None), |(head, tail)| (head, Some(tail)));
        let (authority, path) = if let Some(tail) = text.strip_prefix("//") {
            let (authority, path) = tail
                .find('/')
                .map_or((tail, ""), |slash| (&tail[..slash], &tail[slash..]));
            if authority.is_empty() && path.is_empty() {
                return Err(Error::InvalidSchema);
            }
            (Authority::parse(authority)?, path)
        } else {
            (None, text)
        };
        Ok(Self {
            scheme,
            authority,
            path,
            query,
            fragment,
        })
    }
}
pub(super) fn resolve(base: &str, reference: &str, limit: usize) -> Result<String> {
    if base.len() > limit || reference.len() > limit {
        return Err(Error::SchemaLimitExceeded);
    }
    let base = Parts::absolute(base)?;
    let target = Parts::absolute(reference).or_else(|_| Parts::after_scheme(None, reference))?;
    let (scheme, authority, path, query) = if target.scheme.is_some() {
        (
            target.scheme,
            target.authority,
            remove_dots(target.path),
            target.query,
        )
    } else if target.authority.is_some() {
        (
            base.scheme,
            target.authority,
            remove_dots(target.path),
            target.query,
        )
    } else if target.path.is_empty() {
        (
            base.scheme,
            base.authority,
            base.path.into(),
            target.query.or(base.query),
        )
    } else if target.path.starts_with('/') {
        (
            base.scheme,
            base.authority,
            remove_dots(target.path),
            target.query,
        )
    } else {
        let prefix = if base.authority.is_some() && base.path.is_empty() {
            "/"
        } else {
            base.path
                .rfind('/')
                .map_or("", |index| &base.path[..=index])
        };
        (
            base.scheme,
            base.authority,
            remove_dots(&format!("{prefix}{}", target.path)),
            target.query,
        )
    };
    let mut output = String::new();
    output.push_str(scheme.ok_or(Error::InvalidSchema)?);
    output.push(':');
    if let Some(authority) = authority {
        authority.render(&mut output)?;
    }
    output.push_str(if path.is_empty() { "/" } else { &path });
    if let Some(query) = query {
        output.push('?');
        output.push_str(query);
    }
    if let Some(fragment) = target.fragment {
        output.push('#');
        output.push_str(fragment);
    }
    if output.len() > limit {
        return Err(Error::SchemaLimitExceeded);
    }
    Ok(output)
}
fn remove_dots(mut input: &str) -> String {
    let mut output = String::new();
    while !input.is_empty() {
        if let Some(tail) = input
            .strip_prefix("../")
            .or_else(|| input.strip_prefix("./"))
        {
            input = tail;
        } else if input.starts_with("/./") {
            input = &input[2..];
        } else if input == "/." {
            input = "/";
        } else if input.starts_with("/../") || input == "/.." {
            input = if input == "/.." { "/" } else { &input[3..] };
            output.truncate(output.rfind('/').unwrap_or(0));
        } else if input == "." || input == ".." {
            input = "";
        } else {
            let start = usize::from(input.starts_with('/'));
            let end = input[start..]
                .find('/')
                .map_or(input.len(), |index| start + index);
            output.push_str(&input[..end]);
            input = &input[end..];
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{Error, resolve};

    #[test]
    fn producer_uri_rendering_and_relative_fallback_are_preserved() {
        for (reference, expected) in [
            ("https://example.test", "https://example.test/"),
            ("https://example.test:00443", "https://example.test:443/"),
            (
                "https://user:@example.test:+0_443",
                "https://user@example.test:443/",
            ),
            ("https://example.test:-0", "https://example.test:0/"),
            ("file:///schema", "file:/schema"),
            ("1foo:bar", "fx-schema:/1foo:bar"),
            (
                "https://example.test:bad",
                "fx-schema:/https://example.test:bad",
            ),
            ("a b", "fx-schema:/a b"),
        ] {
            assert_eq!(
                resolve("fx-schema:/root", reference, 4096).unwrap(),
                expected
            );
        }
        for reference in [
            "https://bad_host/schema",
            "https://-host/schema",
            "https://[::1]/schema",
            "//example.test:99999/schema",
        ] {
            assert_eq!(
                resolve("fx-schema:/root", reference, 4096),
                Err(Error::InvalidSchema)
            );
        }
    }

    #[test]
    fn rfc3986_resolution_keeps_resource_identity_and_literal_fragments() {
        for (reference, expected) in [
            ("g:h", "g:h"),
            ("g", "http://a/b/c/g"),
            ("./g", "http://a/b/c/g"),
            ("g/", "http://a/b/c/g/"),
            ("/g", "http://a/g"),
            ("//g", "http://g/"),
            ("?y", "http://a/b/c/d;p?y"),
            ("g?y", "http://a/b/c/g?y"),
            ("#s", "http://a/b/c/d;p?q#s"),
            ("g#s", "http://a/b/c/g#s"),
            ("g?y#s", "http://a/b/c/g?y#s"),
            (";x", "http://a/b/c/;x"),
            ("g;x", "http://a/b/c/g;x"),
            ("", "http://a/b/c/d;p?q"),
            (".", "http://a/b/c/"),
            ("./", "http://a/b/c/"),
            ("..", "http://a/b/"),
            ("../", "http://a/b/"),
            ("../g", "http://a/b/g"),
            ("../..", "http://a/"),
            ("../../g", "http://a/g"),
            ("../../../g", "http://a/g"),
            ("/./g", "http://a/g"),
            ("/../g", "http://a/g"),
            ("g.", "http://a/b/c/g."),
            (".g", "http://a/b/c/.g"),
            ("g/./h", "http://a/b/c/g/h"),
            ("g/../h", "http://a/b/c/h"),
            ("g?y/../x", "http://a/b/c/g?y/../x"),
            ("g#s/../x", "http://a/b/c/g#s/../x"),
            ("%2e%2e/g", "http://a/b/c/%2e%2e/g"),
            ("#/%2F/~1", "http://a/b/c/d;p?q#/%2F/~1"),
        ] {
            assert_eq!(
                resolve("http://a/b/c/d;p?q", reference, 4096).unwrap(),
                expected
            );
        }
    }
}
