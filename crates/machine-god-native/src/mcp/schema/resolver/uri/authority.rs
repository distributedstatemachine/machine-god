//! Identifier authority parsing follows the producer's Zig 0.16 URI data rules.
use super::{Error, Result};

#[derive(Clone, Copy)]
pub(super) struct Authority<'a> {
    user: Option<&'a str>,
    password: Option<&'a str>,
    host: &'a str,
    port: Option<u16>,
}
impl<'a> Authority<'a> {
    pub fn parse(text: &'a str) -> Result<Option<Self>> {
        let (user, password, host_port) =
            text.split_once('@')
                .map_or((None, None, text), |(user, host)| {
                    let (user, password) = user
                        .split_once(':')
                        .map_or((user, None), |(user, password)| {
                            (user, (!password.is_empty()).then_some(password))
                        });
                    (Some(user), password, host)
                });
        if host_port.is_empty() {
            return Ok(None);
        }
        if host_port.starts_with(']') {
            return Err(Error::InvalidSchema);
        }
        let mut end = if host_port.starts_with('[') {
            host_port.rfind(']').ok_or(Error::InvalidSchema)? + 1
        } else {
            host_port.len()
        };
        let port = if let Some(colon) = host_port
            .rfind(':')
            .filter(|colon| !host_port.starts_with('[') || *colon >= end)
        {
            end = end.min(colon);
            Some(port(&host_port[colon + 1..])?)
        } else {
            None
        };
        if end == 0 {
            return Err(Error::InvalidSchema);
        }
        Ok(Some(Self {
            user,
            password,
            host: &host_port[..end],
            port,
        }))
    }
    pub fn render(self, output: &mut String) -> Result<()> {
        validate_host(self.host)?;
        output.push_str("//");
        if let Some(user) = self.user {
            output.push_str(user);
            if let Some(password) = self.password {
                output.push(':');
                output.push_str(password);
            }
            output.push('@');
        }
        output.push_str(self.host);
        if let Some(port) = self.port {
            output.push(':');
            output.push_str(&port.to_string());
        }
        Ok(())
    }
}
fn port(text: &str) -> Result<u16> {
    let negative = text.starts_with('-');
    let text = text.strip_prefix(['+', '-']).unwrap_or(text);
    if text.is_empty() || text.starts_with('_') || text.ends_with('_') {
        return Err(Error::InvalidSchema);
    }
    let mut value = 0_u16;
    for byte in text.bytes().filter(|byte| *byte != b'_') {
        if !byte.is_ascii_digit() {
            return Err(Error::InvalidSchema);
        }
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u16::from(byte - b'0')))
            .ok_or(Error::InvalidSchema)?;
    }
    if negative && value != 0 {
        return Err(Error::InvalidSchema);
    }
    Ok(value)
}
fn validate_host(host: &str) -> Result<()> {
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.is_empty() || host.len() > 255 {
        return Err(Error::InvalidSchema);
    }
    for label in host.split('.') {
        if label.is_empty()
            || label.len() > 63
            || !label.as_bytes()[0].is_ascii_alphanumeric()
            || !label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(Error::InvalidSchema);
        }
    }
    Ok(())
}
