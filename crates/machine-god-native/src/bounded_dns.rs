//! Shared bounded DNS wire, socket and captured resolver configuration helpers.
//! Consumer-specific address policy, cancellation and timing remain with callers.

use hickory_proto::op::{Header, Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{DNSClass, Name as DnsName, RData, RecordType};
use hickory_proto::serialize::binary::{BinDecodable, BinDecoder};
use hickory_resolver::config::{ProtocolConfig, ResolverConfig, ResolverOpts};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    net::{IpAddr, SocketAddr},
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
pub(crate) const SYSTEM_RESOLVER_CONFIG_PATH: &str = "/etc/resolv.conf";
#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
pub(crate) const SYSTEM_RESOLVER_MAX_CONFIG_BYTES: usize = 64 * 1024;
pub(crate) const SYSTEM_RESOLVER_MAX_NAMESERVERS: usize = 32;
pub(crate) const SYSTEM_RESOLVER_MAX_SEARCH_DOMAINS: usize = 32;
pub(crate) const SYSTEM_RESOLVER_MAX_NAME_BYTES: usize = 8 * 1024;
pub(crate) const SYSTEM_RESOLVER_MAX_CONNECTIONS: usize = 64;
#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
pub(crate) const SYSTEM_RESOLVER_MAX_INTERRUPTED_READS: usize = 16;
#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
pub(crate) const SYSTEM_RESOLVER_MAX_READ_CALLS: usize =
    SYSTEM_RESOLVER_MAX_CONFIG_BYTES + SYSTEM_RESOLVER_MAX_INTERRUPTED_READS + 2;
pub(crate) const SYSTEM_RESOLVER_MAX_NDOTS: usize = 15;
pub(crate) const SYSTEM_RESOLVER_MAX_ATTEMPTS: usize = 5;
pub(crate) const SYSTEM_RESOLVER_MAX_AVOIDED_PORTS: usize = 1_024;
pub(crate) const SYSTEM_RESOLVER_MAX_DNS_ADDRESSES: usize = 32;
pub(crate) const SYSTEM_RESOLVER_MAX_DNS_CNAME_RECORDS: usize = 7;
pub(crate) const SYSTEM_RESOLVER_MAX_DNS_ANSWER_RECORDS: usize =
    SYSTEM_RESOLVER_MAX_DNS_ADDRESSES + SYSTEM_RESOLVER_MAX_DNS_CNAME_RECORDS;
pub(crate) const SYSTEM_RESOLVER_MAX_DNS_RESOURCE_RECORDS: usize =
    4 * SYSTEM_RESOLVER_MAX_DNS_ADDRESSES;
pub(crate) const SYSTEM_RESOLVER_MAX_DNS_MESSAGE_BYTES: usize = 4 * 1_024;

pub(crate) struct ParsedSystemResolverSnapshot {
    pub(crate) config: ResolverConfig,
    pub(crate) options: ResolverOpts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SystemNameServer {
    pub(crate) udp: Option<SocketAddr>,
    pub(crate) tcp: Option<SocketAddr>,
    pub(crate) trust_negative_responses: bool,
}

pub(crate) struct SystemResolverSnapshot {
    pub(crate) name_servers: Vec<SystemNameServer>,
    pub(crate) timeout: Duration,
    pub(crate) attempts: usize,
    pub(crate) concurrent_requests: usize,
    pub(crate) try_tcp_on_error: bool,
    pub(crate) recursion_desired: bool,
}

pub(crate) struct QueryIdSequence {
    key: [u8; 32],
    counter: AtomicU32,
}

impl QueryIdSequence {
    pub(crate) fn new(key: [u8; 32]) -> Self {
        Self::with_counter(key, 0)
    }

    pub(crate) fn with_counter(key: [u8; 32], counter: u32) -> Self {
        Self {
            key,
            counter: AtomicU32::new(counter),
        }
    }

    pub(crate) fn next(&self) -> u16 {
        let counter = self.counter.fetch_add(1, Ordering::Relaxed);
        let mut digest = Sha256::new();
        digest.update(self.key);
        digest.update(counter.to_be_bytes());
        let digest = digest.finalize();
        u16::from_be_bytes([digest[0], digest[1]])
    }
}

pub(crate) fn validate_system_resolver_snapshot(
    snapshot: &ParsedSystemResolverSnapshot,
) -> Result<SystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
    let configured_servers = snapshot.config.name_servers();
    if configured_servers.is_empty() || configured_servers.len() > SYSTEM_RESOLVER_MAX_NAMESERVERS {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let connection_count = configured_servers.iter().try_fold(0usize, |count, server| {
        if server.connections.is_empty()
            || server.connections.len() > 2
            || server.connections.iter().any(|item| {
                item.port == 0
                    || !matches!(item.protocol, ProtocolConfig::Udp | ProtocolConfig::Tcp)
            })
        {
            return None;
        }
        count.checked_add(server.connections.len())
    });
    if connection_count.is_none_or(|count| count > SYSTEM_RESOLVER_MAX_CONNECTIONS) {
        return Err(SystemResolverConfigurationUnavailable);
    }
    if snapshot.config.search().len() > SYSTEM_RESOLVER_MAX_SEARCH_DOMAINS {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let name_bytes = snapshot
        .config
        .domain()
        .into_iter()
        .chain(snapshot.config.search())
        .try_fold(0usize, |count, name| count.checked_add(name.len()));
    if name_bytes.is_none_or(|count| count > SYSTEM_RESOLVER_MAX_NAME_BYTES)
        || snapshot.options.ndots > SYSTEM_RESOLVER_MAX_NDOTS
        || snapshot.options.timeout.is_zero()
        || snapshot.options.timeout > Duration::from_secs(30)
        || !(1..=SYSTEM_RESOLVER_MAX_ATTEMPTS).contains(&snapshot.options.attempts)
        || snapshot.options.num_concurrent_reqs > SYSTEM_RESOLVER_MAX_NAMESERVERS
        || !(1..=32).contains(&snapshot.options.max_active_requests)
        || snapshot.options.avoid_local_udp_ports.len() > SYSTEM_RESOLVER_MAX_AVOIDED_PORTS
        || !snapshot.options.avoid_local_udp_ports.is_empty()
        || snapshot.options.case_randomization
        || snapshot.options.trust_anchor.is_some()
        || !snapshot.options.allow_answers.is_empty()
        || !snapshot.options.deny_answers.is_empty()
    {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let mut name_servers = Vec::with_capacity(configured_servers.len());
    for server in configured_servers {
        let udp = server
            .connections
            .iter()
            .find(|connection| connection.protocol == ProtocolConfig::Udp)
            .map(|connection| SocketAddr::new(server.ip, connection.port));
        let tcp = server
            .connections
            .iter()
            .find(|connection| connection.protocol == ProtocolConfig::Tcp)
            .map(|connection| SocketAddr::new(server.ip, connection.port));
        if udp.is_some() || tcp.is_some() {
            name_servers.push(SystemNameServer {
                udp,
                tcp,
                trust_negative_responses: server.trust_negative_responses,
            });
        }
    }
    if name_servers.is_empty() {
        return Err(SystemResolverConfigurationUnavailable);
    }
    Ok(SystemResolverSnapshot {
        name_servers,
        timeout: snapshot.options.timeout,
        attempts: snapshot.options.attempts,
        concurrent_requests: snapshot.options.num_concurrent_reqs.max(1),
        try_tcp_on_error: snapshot.options.try_tcp_on_error,
        recursion_desired: snapshot.options.recursion_desired,
    })
}

pub(crate) fn advance_system_cname_chain(
    visited: &mut Vec<DnsName>,
    cname_hops: &mut usize,
    canonical_chain: &[DnsName],
) -> Result<Option<DnsName>, SystemResolverConfigurationUnavailable> {
    for target in canonical_chain {
        *cname_hops = cname_hops
            .checked_add(1)
            .ok_or(SystemResolverConfigurationUnavailable)?;
        if *cname_hops > SYSTEM_RESOLVER_MAX_DNS_CNAME_RECORDS || visited.contains(target) {
            return Err(SystemResolverConfigurationUnavailable);
        }
        visited.push(target.clone());
    }
    Ok(canonical_chain.last().cloned())
}

pub(crate) fn system_dns_query(
    id: u16,
    name: &DnsName,
    record_type: RecordType,
    recursion_desired: bool,
) -> Result<Vec<u8>, SystemResolverConfigurationUnavailable> {
    let mut query = Message::new(id, MessageType::Query, OpCode::Query);
    query.metadata.recursion_desired = recursion_desired;
    query.add_query(Query::query(name.clone(), record_type));
    let wire = query
        .to_vec()
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    if wire.len() > SYSTEM_RESOLVER_MAX_DNS_MESSAGE_BYTES {
        return Err(SystemResolverConfigurationUnavailable);
    }
    Ok(wire)
}

pub(crate) async fn query_system_name_server(
    server: SystemNameServer,
    wire: &[u8],
    id: u16,
    name: &DnsName,
    record_type: RecordType,
    try_tcp_on_error: bool,
) -> Result<SystemDnsAnswer, SystemDnsServerError> {
    let response = if let Some(udp) = server.udp {
        match exchange_system_dns_udp(udp, wire)
            .await
            .and_then(|response| classify_system_dns_udp_response(&response, id, name, record_type))
        {
            Ok(SystemDnsUdpResponse::Complete(response)) => Ok(response),
            Ok(SystemDnsUdpResponse::Truncated) => {
                let Some(tcp) = server.tcp else {
                    return Err(SystemDnsServerError::Retry);
                };
                exchange_system_dns_tcp(tcp, wire).await
            }
            Err(_) if try_tcp_on_error => {
                let Some(tcp) = server.tcp else {
                    return Err(SystemDnsServerError::Retry);
                };
                exchange_system_dns_tcp(tcp, wire).await
            }
            Err(error) => Err(error),
        }
    } else {
        let Some(tcp) = server.tcp else {
            return Err(SystemDnsServerError::Retry);
        };
        exchange_system_dns_tcp(tcp, wire).await
    }
    .map_err(|_| SystemDnsServerError::Retry)?;
    match validate_system_dns_response(&response, id, name, record_type) {
        Ok(SystemDnsResponse::Answer(answer)) => Ok(answer),
        Ok(SystemDnsResponse::Negative) if server.trust_negative_responses => {
            Err(SystemDnsServerError::TrustedNegative)
        }
        Ok(SystemDnsResponse::Negative) | Err(_) => Err(SystemDnsServerError::Retry),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SystemDnsServerError {
    Retry,
    TrustedNegative,
}

pub(crate) struct SystemDnsAnswer {
    pub(crate) addresses: Vec<IpAddr>,
    pub(crate) canonical_chain: Vec<DnsName>,
}

impl SystemDnsAnswer {
    pub(crate) fn empty() -> Self {
        Self {
            addresses: Vec::new(),
            canonical_chain: Vec::new(),
        }
    }
}

pub(crate) async fn exchange_system_dns_udp(
    nameserver: SocketAddr,
    wire: &[u8],
) -> Result<Vec<u8>, SystemResolverConfigurationUnavailable> {
    let bind = match nameserver {
        SocketAddr::V4(_) => SocketAddr::from(([0, 0, 0, 0], 0)),
        SocketAddr::V6(_) => SocketAddr::from(([0_u16; 8], 0)),
    };
    let socket = tokio::net::UdpSocket::bind(bind)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    socket
        .connect(nameserver)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    let sent = socket
        .send(wire)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    if sent != wire.len() {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let mut response = [0_u8; SYSTEM_RESOLVER_MAX_DNS_MESSAGE_BYTES + 1];
    let received = socket
        .recv(&mut response)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    if received > SYSTEM_RESOLVER_MAX_DNS_MESSAGE_BYTES {
        return Err(SystemResolverConfigurationUnavailable);
    }
    Ok(response[..received].to_vec())
}

pub(crate) async fn exchange_system_dns_tcp(
    nameserver: SocketAddr,
    wire: &[u8],
) -> Result<Message, SystemResolverConfigurationUnavailable> {
    let wire_len = u16::try_from(wire.len()).map_err(|_| SystemResolverConfigurationUnavailable)?;
    let mut stream = tokio::net::TcpStream::connect(nameserver)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    stream
        .write_all(&wire_len.to_be_bytes())
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    stream
        .write_all(wire)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    let response_len = stream
        .read_u16()
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    let response_len = usize::from(response_len);
    if !(12..=SYSTEM_RESOLVER_MAX_DNS_MESSAGE_BYTES).contains(&response_len) {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let mut response = vec![0_u8; response_len];
    stream
        .read_exact(&mut response)
        .await
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    decode_system_dns_response(&response)
}

pub(crate) enum SystemDnsUdpResponse {
    Complete(Message),
    Truncated,
}

pub(crate) fn classify_system_dns_udp_response(
    response: &[u8],
    id: u16,
    name: &DnsName,
    record_type: RecordType,
) -> Result<SystemDnsUdpResponse, SystemResolverConfigurationUnavailable> {
    validate_system_dns_header_count_caps(response)?;
    let mut decoder = BinDecoder::new(response);
    let header = Header::read(&mut decoder).map_err(|_| SystemResolverConfigurationUnavailable)?;
    if header.metadata.id != id
        || header.metadata.message_type != MessageType::Response
        || header.metadata.op_code != OpCode::Query
        || !matches!(
            header.metadata.response_code,
            ResponseCode::NoError | ResponseCode::NXDomain
        )
    {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let query = Query::read(&mut decoder).map_err(|_| SystemResolverConfigurationUnavailable)?;
    if !query.name().is_fqdn()
        || query.name() != name
        || query.query_type() != record_type
        || query.query_class() != DNSClass::IN
    {
        return Err(SystemResolverConfigurationUnavailable);
    }
    if header.metadata.truncation {
        Ok(SystemDnsUdpResponse::Truncated)
    } else {
        decode_system_dns_response(response).map(SystemDnsUdpResponse::Complete)
    }
}

pub(crate) fn decode_system_dns_response(
    response: &[u8],
) -> Result<Message, SystemResolverConfigurationUnavailable> {
    validate_system_dns_header_counts(response)?;
    let mut decoder = BinDecoder::new(response);
    let message =
        Message::read(&mut decoder).map_err(|_| SystemResolverConfigurationUnavailable)?;
    if !decoder.is_empty() {
        return Err(SystemResolverConfigurationUnavailable);
    }
    Ok(message)
}

#[derive(Clone, Copy)]
pub(crate) struct SystemDnsHeaderCounts {
    questions: usize,
    answers: usize,
    authorities: usize,
    additionals: usize,
}

impl SystemDnsHeaderCounts {
    fn resource_records(self) -> Option<usize> {
        self.answers
            .checked_add(self.authorities)
            .and_then(|total| total.checked_add(self.additionals))
    }
}

pub(crate) fn validate_system_dns_header_count_caps(
    response: &[u8],
) -> Result<SystemDnsHeaderCounts, SystemResolverConfigurationUnavailable> {
    let header = response
        .get(..12)
        .ok_or(SystemResolverConfigurationUnavailable)?;
    let counts = SystemDnsHeaderCounts {
        questions: usize::from(u16::from_be_bytes([header[4], header[5]])),
        answers: usize::from(u16::from_be_bytes([header[6], header[7]])),
        authorities: usize::from(u16::from_be_bytes([header[8], header[9]])),
        additionals: usize::from(u16::from_be_bytes([header[10], header[11]])),
    };
    let resource_records = counts
        .resource_records()
        .ok_or(SystemResolverConfigurationUnavailable)?;
    if counts.questions != 1
        || counts.answers > SYSTEM_RESOLVER_MAX_DNS_ANSWER_RECORDS
        || counts.authorities > SYSTEM_RESOLVER_MAX_DNS_RESOURCE_RECORDS
        || counts.additionals > SYSTEM_RESOLVER_MAX_DNS_RESOURCE_RECORDS
        || resource_records > SYSTEM_RESOLVER_MAX_DNS_RESOURCE_RECORDS
    {
        return Err(SystemResolverConfigurationUnavailable);
    }
    Ok(counts)
}

pub(crate) fn validate_system_dns_header_counts(
    response: &[u8],
) -> Result<(), SystemResolverConfigurationUnavailable> {
    let counts = validate_system_dns_header_count_caps(response)?;
    let resource_records = counts
        .resource_records()
        .ok_or(SystemResolverConfigurationUnavailable)?;
    let minimum_wire_len = 12_usize
        .checked_add(
            counts
                .questions
                .checked_mul(5)
                .ok_or(SystemResolverConfigurationUnavailable)?,
        )
        .and_then(|length| {
            resource_records
                .checked_mul(11)
                .and_then(|records| length.checked_add(records))
        })
        .ok_or(SystemResolverConfigurationUnavailable)?;
    if minimum_wire_len > response.len() {
        return Err(SystemResolverConfigurationUnavailable);
    }
    Ok(())
}

pub(crate) fn validate_system_dns_response(
    response: &Message,
    id: u16,
    name: &DnsName,
    record_type: RecordType,
) -> Result<SystemDnsResponse, SystemResolverConfigurationUnavailable> {
    if response.metadata.id != id
        || response.metadata.message_type != MessageType::Response
        || response.metadata.op_code != OpCode::Query
        || response.metadata.truncation
        || response.queries.len() != 1
        || response.queries[0].name() != name
        || response.queries[0].query_type() != record_type
        || response.queries[0].query_class() != DNSClass::IN
    {
        return Err(SystemResolverConfigurationUnavailable);
    }
    if response.metadata.response_code == ResponseCode::NXDomain {
        return if response.answers.is_empty() {
            Ok(SystemDnsResponse::Negative)
        } else {
            Err(SystemResolverConfigurationUnavailable)
        };
    }
    if response.metadata.response_code != ResponseCode::NoError {
        return Err(SystemResolverConfigurationUnavailable);
    }
    let mut chain = vec![name.clone()];
    let terminal = loop {
        let current = chain
            .last()
            .expect("the DNS validation chain starts nonempty");
        let mut targets = response.answers.iter().filter_map(|record| {
            (&record.name == current)
                .then_some(record)
                .and_then(|record| match &record.data {
                    RData::CNAME(target) if record.dns_class == DNSClass::IN => {
                        Some(target.0.clone())
                    }
                    _ => None,
                })
        });
        let Some(target) = targets.next() else {
            break current.clone();
        };
        if targets.any(|candidate| candidate != target)
            || chain.contains(&target)
            || chain.len() >= 8
        {
            return Err(SystemResolverConfigurationUnavailable);
        }
        chain.push(target);
    };

    let mut addresses = Vec::new();
    for record in &response.answers {
        match &record.data {
            RData::A(address) => {
                if record_type != RecordType::A
                    || record.dns_class != DNSClass::IN
                    || record.name != terminal
                {
                    return Err(SystemResolverConfigurationUnavailable);
                }
                addresses.push(IpAddr::V4(address.0));
            }
            RData::AAAA(address) => {
                if record_type != RecordType::AAAA
                    || record.dns_class != DNSClass::IN
                    || record.name != terminal
                {
                    return Err(SystemResolverConfigurationUnavailable);
                }
                addresses.push(IpAddr::V6(address.0));
            }
            RData::CNAME(target) => {
                let Some(position) = chain.iter().position(|name| name == &record.name) else {
                    return Err(SystemResolverConfigurationUnavailable);
                };
                if record.dns_class != DNSClass::IN || chain.get(position + 1) != Some(&target.0) {
                    return Err(SystemResolverConfigurationUnavailable);
                }
            }
            _ => {}
        }
        if addresses.len() > SYSTEM_RESOLVER_MAX_DNS_ADDRESSES {
            return Err(SystemResolverConfigurationUnavailable);
        }
    }
    Ok(SystemDnsResponse::Answer(SystemDnsAnswer {
        addresses,
        canonical_chain: chain.into_iter().skip(1).collect(),
    }))
}

pub(crate) enum SystemDnsResponse {
    Answer(SystemDnsAnswer),
    Negative,
}

#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
pub(crate) fn load_system_resolver_snapshot()
-> Result<ParsedSystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
    let bytes = read_bounded_system_resolver_configuration(std::path::Path::new(
        SYSTEM_RESOLVER_CONFIG_PATH,
    ))?;
    let (config, options) = hickory_resolver::system_conf::parse_resolv_conf(bytes)
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    Ok(ParsedSystemResolverSnapshot { config, options })
}

#[cfg(any(target_os = "windows", target_vendor = "apple"))]
pub(crate) fn load_system_resolver_snapshot()
-> Result<ParsedSystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
    let (config, options) = hickory_resolver::system_conf::read_system_conf()
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    Ok(ParsedSystemResolverSnapshot { config, options })
}

#[cfg(target_os = "android")]
pub(crate) fn load_system_resolver_snapshot()
-> Result<ParsedSystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
    // Hickory's Android loader requires initialized NDK process context and
    // panics when that global context is absent. A release panic aborts this
    // process, so catalog DNS stays unavailable until a non-panicking native
    // configuration boundary is explicitly provided.
    Err(SystemResolverConfigurationUnavailable)
}

#[cfg(not(any(
    all(unix, not(any(target_os = "android", target_vendor = "apple"))),
    target_os = "android",
    target_os = "windows",
    target_vendor = "apple"
)))]
pub(crate) fn load_system_resolver_snapshot()
-> Result<ParsedSystemResolverSnapshot, SystemResolverConfigurationUnavailable> {
    Err(SystemResolverConfigurationUnavailable)
}

#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
pub(crate) fn read_bounded_system_resolver_configuration(
    path: &std::path::Path,
) -> Result<Vec<u8>, SystemResolverConfigurationUnavailable> {
    use std::fs::OpenOptions;
    use std::io::Read as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let initial_metadata =
        std::fs::metadata(path).map_err(|_| SystemResolverConfigurationUnavailable)?;
    validate_system_resolver_file_metadata(&initial_metadata)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    let metadata = file
        .metadata()
        .map_err(|_| SystemResolverConfigurationUnavailable)?;
    validate_system_resolver_file_metadata(&metadata)?;

    let mut bytes = Vec::with_capacity(SYSTEM_RESOLVER_MAX_CONFIG_BYTES + 1);
    let mut buffer = [0_u8; 8 * 1024];
    let mut interrupted = 0usize;
    let mut read_calls = 0usize;
    loop {
        if read_calls >= SYSTEM_RESOLVER_MAX_READ_CALLS {
            return Err(SystemResolverConfigurationUnavailable);
        }
        read_calls += 1;
        let remaining = SYSTEM_RESOLVER_MAX_CONFIG_BYTES
            .saturating_sub(bytes.len())
            .saturating_add(1);
        let read_limit = remaining.min(buffer.len());
        match file.read(&mut buffer[..read_limit]) {
            Ok(0) => {
                let final_metadata = file
                    .metadata()
                    .map_err(|_| SystemResolverConfigurationUnavailable)?;
                validate_system_resolver_file_metadata(&final_metadata)?;
                return Ok(bytes);
            }
            Ok(read) => {
                bytes.extend_from_slice(&buffer[..read]);
                if bytes.len() > SYSTEM_RESOLVER_MAX_CONFIG_BYTES {
                    return Err(SystemResolverConfigurationUnavailable);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                interrupted += 1;
                if interrupted > SYSTEM_RESOLVER_MAX_INTERRUPTED_READS {
                    return Err(SystemResolverConfigurationUnavailable);
                }
            }
            Err(_) => return Err(SystemResolverConfigurationUnavailable),
        }
    }
}

#[cfg(all(unix, not(any(target_os = "android", target_vendor = "apple"))))]
pub(crate) fn validate_system_resolver_file_metadata(
    metadata: &std::fs::Metadata,
) -> Result<(), SystemResolverConfigurationUnavailable> {
    if metadata.file_type().is_file() && metadata.len() <= SYSTEM_RESOLVER_MAX_CONFIG_BYTES as u64 {
        Ok(())
    } else {
        Err(SystemResolverConfigurationUnavailable)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SystemResolverConfigurationUnavailable;

impl fmt::Debug for SystemResolverConfigurationUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SystemResolverConfigurationUnavailable")
    }
}

impl fmt::Display for SystemResolverConfigurationUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("system resolver configuration unavailable")
    }
}

impl std::error::Error for SystemResolverConfigurationUnavailable {}
