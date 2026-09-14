//! Only Gateway construction differs in the explicitly injected network test.
//! Acquisition, catalog loading and native host observations remain shared.

use machine_god_native::{
    AiGatewayModelCatalogHttpTransport, AiGatewayModelCatalogTransport,
    DiscoveredAiGatewayCredential, LoadedNativeConfig, NativeReferenceHost,
    NativeReferenceHostConversationOptions, PermissionPrompter, PreparedNativeRoots,
    QuestionPrompter, WebSearchDeadline,
};
use std::sync::Arc;

#[derive(Clone, Default)]
pub(in crate::ask::production) enum GatewayNetwork {
    #[default]
    Production,
    #[cfg(test)]
    Loopback(Box<Loopback>),
}
#[cfg(test)]
#[derive(Clone)]
pub(in crate::ask::production) struct Loopback {
    inference: machine_god_native::AiGatewayHttpEndpoint,
    catalog: machine_god_native::AiGatewayModelCatalogHttpEndpoint,
    target: machine_god_core::NetworkTarget,
}
impl GatewayNetwork {
    #[cfg(test)]
    pub(super) fn loopback(address: std::net::SocketAddr) -> Result<Self, ()> {
        Ok(Self::Loopback(Box::new(Loopback {
            inference: machine_god_native::AiGatewayHttpEndpoint::loopback_http(&format!(
                "http://{address}/inference"
            ))
            .map_err(|_| ())?,
            catalog: machine_god_native::AiGatewayModelCatalogHttpEndpoint::loopback_http(
                &format!("http://{address}/catalog"),
            )
            .map_err(|_| ())?,
            target: machine_god_core::NetworkTarget {
                scheme: "http".into(),
                host: address.ip().to_string(),
                port: Some(address.port()),
            },
        })))
    }
    pub(in crate::ask::production) fn catalog(
        &self,
        credential: &DiscoveredAiGatewayCredential,
    ) -> Result<Arc<dyn AiGatewayModelCatalogTransport>, ()> {
        let transport = match self {
            Self::Production => {
                AiGatewayModelCatalogHttpTransport::with_discovered_credential(credential)
            }
            #[cfg(test)]
            Self::Loopback(endpoints) => {
                AiGatewayModelCatalogHttpTransport::with_discovered_credential_and_endpoint_and_limits(
                    credential,
                    endpoints.catalog.clone(),
                    machine_god_native::AiGatewayModelCatalogHttpLimits::default(),
                )
            }
        }
        .map_err(|_| ())?;
        Ok(Arc::new(transport))
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::ask::production) fn compose(
        &self,
        loaded_config: LoadedNativeConfig,
        credential: DiscoveredAiGatewayCredential,
        roots: PreparedNativeRoots,
        permission: Arc<dyn PermissionPrompter>,
        question: Arc<dyn QuestionPrompter>,
        deadline: Arc<dyn WebSearchDeadline>,
        options: NativeReferenceHostConversationOptions,
    ) -> Result<NativeReferenceHost, ()> {
        #[cfg(test)]
        let source = credential.source();
        let host = match self {
            Self::Production => NativeReferenceHost::compose_ai_gateway_http_with_prepared_roots_and_conversation_and_credential(
                loaded_config, credential, roots, permission, question, deadline, options,
            ),
            #[cfg(test)]
            Self::Loopback(endpoints) => NativeReferenceHost::compose_ai_gateway_with_prepared_roots_and_conversation_and_credential_and_transport(
                loaded_config, credential, roots, permission, question, deadline, options,
                |token| {
                    let transport = machine_god_native::AiGatewayHttpTransport::with_endpoint_and_limits(
                        token, endpoints.inference.clone(), machine_god_native::AiGatewayHttpLimits::default(),
                    )?;
                    Ok((Arc::new(transport), endpoints.target.clone()))
                },
            ),
        }
        .map_err(|_| ())?;
        #[cfg(test)]
        assert_eq!(host.credential_source(), Some(source));
        Ok(host)
    }
}
