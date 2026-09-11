use super::{McpNetworkError, NativeMcpNetwork, Result};
use crate::bounded_dns::{
    SystemDnsAnswer, SystemDnsServerError, advance_system_cname_chain, query_system_name_server,
    system_dns_query,
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use hickory_proto::rr::{Name, RecordType};
use machine_god_core::CancellationToken;
use std::{net::IpAddr, time::Instant};

impl NativeMcpNetwork {
    pub(super) async fn lookup(
        &self,
        host: &str,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Vec<IpAddr>> {
        let host = host.strip_suffix('.').unwrap_or(host);
        if host.is_empty() || host.ends_with('.') {
            return Err(McpNetworkError::Invalid);
        }
        if host.len() > 253 {
            return Err(McpNetworkError::Limit);
        }
        let absolute = format!("{host}.");
        let name = Name::from_ascii(absolute).map_err(|_| McpNetworkError::Invalid)?;
        if !name.is_fqdn() {
            return Err(McpNetworkError::Invalid);
        }
        // Both families must be observed successfully: a malformed/over-budget
        // family is never silently dropped in favor of the other one's result.
        let (ipv4, ipv6) = futures_util::future::try_join(
            self.record(&name, RecordType::A, cancellation, deadline),
            self.record(&name, RecordType::AAAA, cancellation, deadline),
        )
        .await?;
        let mut addresses = Vec::new();
        for address in ipv4.into_iter().chain(ipv6) {
            if !addresses.contains(&address) {
                if addresses.len() == 32 {
                    return Err(McpNetworkError::Limit);
                }
                addresses.push(address);
            }
        }
        if addresses.is_empty() {
            return Err(McpNetworkError::Unavailable);
        }
        Ok(addresses)
    }

    async fn record(
        &self,
        name: &Name,
        kind: RecordType,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Vec<IpAddr>> {
        let mut current = name.clone();
        let mut visited = vec![current.clone()];
        let mut hops = 0;
        loop {
            self.check(cancellation, deadline)?;
            let answer = self
                .record_once(&current, kind, cancellation, deadline)
                .await?;
            let target =
                advance_system_cname_chain(&mut visited, &mut hops, &answer.canonical_chain)
                    .map_err(|_| McpNetworkError::Unavailable)?;
            if !answer.addresses.is_empty() {
                return Ok(answer.addresses);
            }
            let Some(target) = target else {
                return Ok(Vec::new());
            };
            current = target;
        }
    }

    async fn record_once(
        &self,
        name: &Name,
        kind: RecordType,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<SystemDnsAnswer> {
        let config = &self.resolver.0;
        for _ in 0..config.attempts {
            for servers in config.name_servers.chunks(config.concurrent_requests) {
                self.check(cancellation, deadline)?;
                let mut queries = FuturesUnordered::new();
                for server in servers {
                    let id = self.query_ids.next();
                    let wire = system_dns_query(id, name, kind, config.recursion_desired)
                        .map_err(|_| McpNetworkError::Unavailable)?;
                    let query_deadline = deadline.min(
                        self.clock
                            .now()
                            .checked_add(config.timeout)
                            .ok_or(McpNetworkError::Limit)?,
                    );
                    queries.push(async move {
                        self.run(cancellation, query_deadline, async {
                            match query_system_name_server(
                                *server,
                                &wire,
                                id,
                                name,
                                kind,
                                config.try_tcp_on_error,
                            )
                            .await
                            {
                                Ok(answer) => Ok(answer),
                                Err(SystemDnsServerError::TrustedNegative) => {
                                    Ok(SystemDnsAnswer::empty())
                                }
                                Err(SystemDnsServerError::Retry) => {
                                    Err(McpNetworkError::Unavailable)
                                }
                            }
                        })
                        .await
                    });
                }
                while let Some(result) = queries.next().await {
                    self.check(cancellation, deadline)?;
                    if let Ok(answer) = result {
                        return Ok(answer);
                    }
                }
            }
        }
        Err(McpNetworkError::Unavailable)
    }
}
