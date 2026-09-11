use super::{
    Authority, McpAuthError, McpAuthRemoteRevocation, Result,
    codec::{self, Credentials, Registration, Secret},
    network::Request,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use machine_god_core::CancellationToken;
use std::time::Instant;

pub(super) fn encode(value: &str) -> String {
    const HEX: &[u8] = b"0123456789ABCDEF";
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    output
}
pub(super) fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}
fn authenticated(registration: &Registration, mut body: String) -> Result<Request> {
    registration.validate()?;
    let authorization = match registration.method.as_ref() {
        "none" => {
            body.push('&');
            body.push_str(&form(&[("client_id", registration.id.text())]));
            None
        }
        "client_secret_post" => {
            body.push('&');
            body.push_str(&form(&[
                ("client_id", registration.id.text()),
                (
                    "client_secret",
                    registration
                        .secret
                        .as_ref()
                        .ok_or(McpAuthError::Missing)?
                        .text(),
                ),
            ]));
            None
        }
        "client_secret_basic" => Some(format!(
            "Basic {}",
            STANDARD.encode(format!(
                "{}:{}",
                encode(registration.id.text()),
                encode(
                    registration
                        .secret
                        .as_ref()
                        .ok_or(McpAuthError::Missing)?
                        .text()
                )
            ))
        )),
        _ => return Err(McpAuthError::Invalid),
    };
    Ok(Request::Token(body.into_bytes(), authorization))
}

#[cfg(test)]
mod tests;
impl Authority {
    pub(super) async fn token(
        &self,
        credentials: &mut Credentials,
        fields: &[(&str, &str)],
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<()> {
        let request = authenticated(&credentials.registration, form(fields))?;
        let response = self
            .request(&credentials.token_endpoint, request, cancellation, deadline)
            .await
            .map_err(|f| f.error)?;
        if response.status != 200 {
            return Err(McpAuthError::Rejected);
        }
        if !response.json {
            return Err(McpAuthError::Invalid);
        }
        let value = codec::json(&response.body)?;
        let access = Secret::new(codec::required(&value, "access_token")?.as_bytes())?;
        let refresh = codec::optional(&value, "refresh_token")?
            .map(|s| Secret::new(s.as_bytes()))
            .transpose()?;
        if codec::optional(&value, "token_type")?.is_some_and(|s| !s.eq_ignore_ascii_case("Bearer"))
        {
            return Err(McpAuthError::Invalid);
        }
        let scope: Option<Box<str>> = match value.get("scope") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(scope)) if scope.len() <= super::SECRET_LIMIT => {
                Some(scope.as_str().into())
            }
            _ => return Err(McpAuthError::Invalid),
        };
        if let Some(scope) = &scope {
            codec::scopes(&[], Some(scope), &[], None, false)?;
        }
        let expires = match value.get("expires_in") {
            None => i64::MAX,
            Some(value) => {
                let seconds = value
                    .as_i64()
                    .filter(|s| *s >= 0)
                    .ok_or(McpAuthError::Invalid)?;
                self.clock
                    .unix_millis()
                    .saturating_add(seconds.saturating_mul(1000))
            }
        };
        credentials.access = access;
        if let Some(refresh) = refresh {
            credentials.refresh = Some(refresh);
        }
        if let Some(scope) = scope {
            credentials.scope = scope;
        }
        credentials.expires_ms = expires;
        Ok(())
    }
    pub(super) async fn refresh(
        &self,
        credentials: &Credentials,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Credentials> {
        let refresh = credentials
            .refresh
            .as_ref()
            .ok_or(McpAuthError::Unavailable)?;
        let mut replacement = credentials.clone();
        self.token(
            &mut replacement,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.text()),
                ("resource", &credentials.resource),
            ],
            cancellation,
            deadline,
        )
        .await?;
        Ok(replacement)
    }
    pub(super) async fn revoke(
        &self,
        credentials: &Credentials,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> McpAuthRemoteRevocation {
        let Some(endpoint) = &credentials.revocation_endpoint else {
            return McpAuthRemoteRevocation::Unsupported;
        };
        let mut result = McpAuthRemoteRevocation::Confirmed;
        let mut attempted = false;
        for (token, hint) in credentials
            .refresh
            .as_ref()
            .map(|token| (token, "refresh_token"))
            .into_iter()
            .chain(std::iter::once((&credentials.access, "access_token")))
        {
            let Ok(request) = authenticated(
                &credentials.registration,
                form(&[("token", token.text()), ("token_type_hint", hint)]),
            ) else {
                return McpAuthRemoteRevocation::NotAttempted;
            };
            match self
                .request(endpoint, request, cancellation, deadline)
                .await
            {
                Ok(response) if response.status == 200 => {
                    attempted = true;
                }
                Ok(response) if matches!(response.status, 404 | 405 | 501) => {
                    if result != McpAuthRemoteRevocation::Ambiguous {
                        result = McpAuthRemoteRevocation::Unsupported;
                    }
                }
                Ok(_) => result = McpAuthRemoteRevocation::Ambiguous,
                Err(failure) => {
                    result = if failure.attempted
                        || attempted
                        || result != McpAuthRemoteRevocation::Confirmed
                    {
                        McpAuthRemoteRevocation::Ambiguous
                    } else {
                        McpAuthRemoteRevocation::NotAttempted
                    };
                    break;
                }
            }
        }
        result
    }
}
