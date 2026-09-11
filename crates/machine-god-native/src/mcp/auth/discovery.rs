use super::{
    Authority, McpAuthError, Result,
    codec::{self, McpAuthChallenge, McpAuthConfig, Metadata, Registration, Secret},
    network::Request,
};
use machine_god_core::CancellationToken;
use std::time::Instant;

pub(super) struct Discovery {
    pub metadata: Metadata,
    pub resource: Box<str>,
    pub scope: Box<str>,
}
impl Authority {
    pub(super) async fn discover(
        &self,
        config: &McpAuthConfig,
        challenge: &McpAuthChallenge,
        previous: Option<&str>,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Discovery> {
        let urls = match challenge.resource_metadata.as_deref() {
            Some(url) => {
                codec::oauth_url(url, &config.resource)?;
                vec![url.to_owned()]
            }
            None => resource_urls(&config.resource)?,
        };
        let resource_value = self.metadata(&urls, cancellation, deadline).await?;
        let resource = codec::canonical(codec::required(&resource_value, "resource")?)?;
        if !codec::covers(&resource, &config.resource) {
            return Err(McpAuthError::Invalid);
        }
        let servers = codec::strings(&resource_value, "authorization_servers")?;
        let issuer = config
            .issuer
            .as_deref()
            .or_else(|| servers.first().map(AsRef::as_ref))
            .ok_or(McpAuthError::Invalid)?;
        codec::oauth_url(issuer, &resource)?;
        let metadata_value = self
            .metadata(&issuer_urls(issuer)?, cancellation, deadline)
            .await?;
        let metadata = parse_metadata(&metadata_value, issuer, &resource)?;
        let scopes = codec::strings(&resource_value, "scopes_supported")?;
        let scope = codec::scopes(
            &config.scopes,
            challenge.scope.as_deref(),
            &scopes,
            previous,
            metadata
                .scopes
                .iter()
                .any(|s| s.as_ref() == "offline_access"),
        )?;
        Ok(Discovery {
            metadata,
            resource,
            scope,
        })
    }
    async fn metadata(
        &self,
        urls: &[String],
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<serde_json::Value> {
        for url in urls {
            match self
                .request(url, Request::Metadata, cancellation, deadline)
                .await
            {
                Ok(response) if response.status == 200 => {
                    if !response.json {
                        return Err(McpAuthError::Invalid);
                    }
                    return codec::json(&response.body);
                }
                Err(failure)
                    if matches!(
                        failure.error,
                        McpAuthError::Cancelled | McpAuthError::Deadline
                    ) =>
                {
                    return Err(failure.error);
                }
                Ok(_) | Err(_) => {}
            }
        }
        Err(McpAuthError::Unavailable)
    }
    pub(super) async fn registration(
        &self,
        config: &McpAuthConfig,
        metadata: &Metadata,
        redirect: &str,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Registration> {
        let method = choose_method(metadata, config.client_secret.is_some())?;
        if let Some(id) = &config.client_id {
            let registration = Registration {
                id: Secret::new(id.as_bytes())?,
                secret: config.client_secret.clone(),
                method: method.into(),
            };
            registration.validate()?;
            return Ok(registration);
        }
        if metadata.client_document
            && let Some(url) = &config.client_metadata_url
        {
            let parsed = codec::secure_url(url)?;
            if parsed.scheme() != "https" || parsed.path().trim_matches('/').is_empty() {
                return Err(McpAuthError::Invalid);
            }
            let registration = Registration {
                id: Secret::new(url.as_bytes())?,
                secret: None,
                method: method.into(),
            };
            registration.validate()?;
            return Ok(registration);
        }
        let url = metadata
            .registration
            .as_deref()
            .ok_or(McpAuthError::Unavailable)?;
        let grants = if metadata
            .grants
            .iter()
            .any(|v| v.as_ref() == "refresh_token")
        {
            vec!["authorization_code", "refresh_token"]
        } else {
            vec!["authorization_code"]
        };
        let payload = serde_json::to_vec(&serde_json::json!({
            "client_name": "machine-god", "application_type": "native", "redirect_uris": [redirect],
            "response_types": ["code"], "grant_types": grants, "token_endpoint_auth_method": method,
        }))
        .map_err(|_| McpAuthError::Invalid)?;
        let response = self
            .request(url, Request::Registration(payload), cancellation, deadline)
            .await
            .map_err(|f| f.error)?;
        if !matches!(response.status, 200 | 201) {
            return Err(McpAuthError::Rejected);
        }
        if !response.json {
            return Err(McpAuthError::Invalid);
        }
        let value = codec::json(&response.body)?;
        let registration = Registration {
            id: Secret::new(codec::required(&value, "client_id")?.as_bytes())?,
            secret: codec::optional(&value, "client_secret")?
                .map(|s| Secret::new(s.as_bytes()))
                .transpose()?,
            method: codec::optional(&value, "token_endpoint_auth_method")?
                .unwrap_or_else(|| method.into()),
        };
        registration.validate()?;
        Ok(registration)
    }
}
pub(super) fn resource_urls(resource: &str) -> Result<Vec<String>> {
    let url = codec::secure_url(resource)?;
    let origin = codec::origin(&url);
    let mut urls = Vec::new();
    if url.path() != "/" {
        urls.push(format!(
            "{origin}/.well-known/oauth-protected-resource{}",
            url.path()
        ));
    }
    urls.push(format!("{origin}/.well-known/oauth-protected-resource"));
    Ok(urls)
}
pub(super) fn issuer_urls(issuer: &str) -> Result<Vec<String>> {
    let url = codec::secure_url(issuer)?;
    if url.query().is_some() {
        return Err(McpAuthError::Invalid);
    }
    let origin = codec::origin(&url);
    let path = url.path().trim_matches('/');
    if path.is_empty() {
        Ok(vec![
            format!("{origin}/.well-known/oauth-authorization-server"),
            format!("{origin}/.well-known/openid-configuration"),
        ])
    } else {
        Ok(vec![
            format!("{origin}/.well-known/oauth-authorization-server/{path}"),
            format!("{origin}/.well-known/openid-configuration/{path}"),
            format!("{issuer}/.well-known/openid-configuration"),
        ])
    }
}
fn parse_metadata(value: &serde_json::Value, issuer: &str, resource: &str) -> Result<Metadata> {
    if codec::required(value, "issuer")? != issuer {
        return Err(McpAuthError::IssuerMismatch);
    }
    let authorization = codec::required(value, "authorization_endpoint")?;
    let token = codec::required(value, "token_endpoint")?;
    let registration = codec::optional(value, "registration_endpoint")?;
    let revocation = codec::optional(value, "revocation_endpoint")?;
    for url in [
        Some(authorization),
        Some(token),
        registration.as_deref(),
        revocation.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        codec::oauth_url(url, resource)?;
    }
    if !codec::strings(value, "code_challenge_methods_supported")?
        .iter()
        .any(|s| s.as_ref() == "S256")
    {
        return Err(McpAuthError::Unavailable);
    }
    let methods = if value.get("token_endpoint_auth_methods_supported").is_none() {
        vec!["client_secret_basic".into()]
    } else {
        codec::strings(value, "token_endpoint_auth_methods_supported")?
    };
    Ok(Metadata {
        issuer: issuer.into(),
        authorization: authorization.into(),
        token: token.into(),
        registration,
        revocation,
        scopes: codec::strings(value, "scopes_supported")?,
        grants: codec::strings(value, "grant_types_supported")?,
        methods,
        client_document: value
            .get("client_id_metadata_document_supported")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        require_issuer: value
            .get("authorization_response_iss_parameter_supported")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}
fn choose_method(metadata: &Metadata, secret: bool) -> Result<&str> {
    let methods = if secret {
        ["client_secret_basic", "client_secret_post", "none"]
    } else {
        ["none", "client_secret_basic", "client_secret_post"]
    };
    methods
        .into_iter()
        .find(|method| metadata.methods.iter().any(|s| s.as_ref() == *method))
        .ok_or(McpAuthError::Unavailable)
}
