use crate::*;
use ipnet::IpNet;
use reqwest::{
    blocking::{Client, Response},
    dns::{Addrs, Name, Resolve, Resolving},
    redirect::Policy,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    net::{IpAddr, SocketAddr},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkPolicy {
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default = "default_ports")]
    pub allowed_ports: Vec<u16>,
    #[serde(default)]
    pub internal_hosts: BTreeMap<String, Vec<IpNet>>,
    #[serde(default)]
    pub trusted_ca_pem: Option<String>,
    /// Explicitly trusted egress proxy. Its administrator must enforce the same
    /// destination DNS/IP policy: CONNECT name resolution occurs on that proxy.
    #[serde(default)]
    pub trusted_proxy: Option<String>,
    pub max_bytes: u64,
}
fn default_ports() -> Vec<u16> {
    vec![443]
}
impl NetworkPolicy {
    pub fn validate(&self) -> Result<()> {
        if self.allowed_hosts.len() > 32
            || !(1..=256 * 1024 * 1024).contains(&self.max_bytes)
            || self.allowed_ports.is_empty()
            || self.allowed_ports.len() > 8
            || self.allowed_ports.contains(&0)
        {
            return Err(invalid("INVALID_NETWORK_POLICY"));
        }
        for host in &self.allowed_hosts {
            if host != &host.to_ascii_lowercase()
                || host.ends_with('.')
                || host.contains(['/', ':', '*', '@'])
                || orkestr_domain::canonical_host(host).is_err()
            {
                return Err(invalid("EXACT_HOST_REQUIRED"));
            }
        }
        if self.internal_hosts.len() > 32
            || self.internal_hosts.iter().any(|(host, nets)| {
                !self.allowed_hosts.contains(host) || nets.is_empty() || nets.len() > 8
            })
        {
            return Err(invalid("INVALID_INTERNAL_EXCEPTION"));
        }
        if let Some(ca) = &self.trusted_ca_pem {
            if ca.len() > 65536 || reqwest::Certificate::from_pem(ca.as_bytes()).is_err() {
                return Err(invalid("INVALID_REPOSITORY_CA"));
            }
        }
        if let Some(proxy) = &self.trusted_proxy {
            let url = Url::parse(proxy).map_err(|_| invalid("INVALID_TRUSTED_PROXY"))?;
            if url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
            {
                return Err(invalid("INVALID_TRUSTED_PROXY"));
            }
        }
        Ok(())
    }
    pub fn check_url(&self, raw: &str) -> Result<Url> {
        if raw.len() > 4096 {
            return Err(invalid("URL_TOO_LONG"));
        }
        let url = Url::parse(raw).map_err(|_| invalid("INVALID_URL"))?;
        let host = url.host_str().ok_or_else(|| invalid("INVALID_URL"))?;
        if url.scheme() != "https"
            || !url
                .port_or_known_default()
                .is_some_and(|port| self.allowed_ports.contains(&port))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || !self.allowed_hosts.iter().any(|h| h == host)
        {
            return Err(invalid("URL_NOT_ALLOWED"));
        }
        if let Ok(ip) = host.parse::<IpAddr>() {
            self.check_ip(host, ip)?;
        }
        Ok(url)
    }
    pub fn check_ip(&self, host: &str, ip: IpAddr) -> Result<()> {
        if self
            .internal_hosts
            .get(host)
            .is_some_and(|nets| nets.iter().any(|net| net.contains(&ip)))
        {
            return Ok(());
        }
        if !public_ip(ip) {
            return Err(invalid("SSRF_ADDRESS_BLOCKED"));
        }
        Ok(())
    }
}
pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && ![
                    "0.0.0.0/8",
                    "100.64.0.0/10",
                    "127.0.0.0/8",
                    "169.254.0.0/16",
                    "172.16.0.0/12",
                    "192.0.0.0/24",
                    "192.168.0.0/16",
                    "198.18.0.0/15",
                    "224.0.0.0/4",
                    "240.0.0.0/4",
                ]
                .iter()
                .any(|cidr| cidr.parse::<IpNet>().unwrap().contains(&IpAddr::V4(ip)))
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(mapped));
            }
            "2000::/3"
                .parse::<IpNet>()
                .unwrap()
                .contains(&IpAddr::V6(ip))
                && !["2001::/23", "2001:db8::/32", "2002::/16", "3fff::/20"]
                    .iter()
                    .any(|net| net.parse::<IpNet>().unwrap().contains(&IpAddr::V6(ip)))
        }
    }
}
struct GuardedDns(NetworkPolicy);
impl Resolve for GuardedDns {
    fn resolve(&self, name: Name) -> Resolving {
        let policy = self.0.clone();
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let found = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::net::lookup_host((host.as_str(), 443)),
            )
            .await??;
            let addresses: Vec<SocketAddr> = found.collect();
            if addresses.is_empty() {
                return Err("DNS_EMPTY".into());
            }
            for address in &addresses {
                policy
                    .check_ip(&host, address.ip())
                    .map_err(|_| "SSRF_ADDRESS_BLOCKED")?;
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}
pub struct GuardedHttp {
    client: Client,
    policy: NetworkPolicy,
}
pub struct OpenResponse {
    pub response: Response,
    pub final_url: String,
    pub redirects: Vec<String>,
}
impl GuardedHttp {
    pub fn new(policy: &NetworkPolicy) -> Result<Self> {
        policy.validate()?;
        let mut builder = Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(Policy::none())
            .referer(false)
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(0)
            .dns_resolver(Arc::new(GuardedDns(policy.clone())))
            .user_agent("OrkestrEMP/0.1");
        if let Some(ca) = &policy.trusted_ca_pem {
            builder = builder.add_root_certificate(
                reqwest::Certificate::from_pem(ca.as_bytes())
                    .map_err(|_| invalid("INVALID_REPOSITORY_CA"))?,
            );
        }
        if let Some(proxy) = &policy.trusted_proxy {
            builder = builder
                .proxy(reqwest::Proxy::all(proxy).map_err(|_| invalid("INVALID_TRUSTED_PROXY"))?);
        }
        Ok(Self {
            client: builder.build().map_err(|_| invalid("HTTP_CLIENT_FAILED"))?,
            policy: policy.clone(),
        })
    }
    pub fn open(&self, raw: &str, range: Option<(u64, &str)>) -> Result<OpenResponse> {
        let mut url = self.policy.check_url(raw)?;
        let mut redirects = Vec::new();
        for hop in 0..=5 {
            let mut request = self
                .client
                .get(url.clone())
                .header("accept-encoding", "identity");
            if let Some((start, etag)) = range {
                request = request
                    .header("range", format!("bytes={start}-"))
                    .header("if-range", etag);
            }
            let response = request.send().map_err(|_| invalid("HTTP_REQUEST_FAILED"))?;
            if response.status().is_redirection() {
                if hop == 5 {
                    return Err(invalid("REDIRECT_LIMIT"));
                }
                let next = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| invalid("INVALID_REDIRECT"))?;
                let next = url.join(next).map_err(|_| invalid("INVALID_REDIRECT"))?;
                url = self.policy.check_url(next.as_str())?;
                redirects.push(url.to_string());
                continue;
            }
            if !response.status().is_success() {
                return Err(invalid(if response.status().as_u16() == 429 {
                    "HTTP_RATE_LIMITED"
                } else {
                    "HTTP_STATUS_FAILED"
                }));
            }
            if response
                .headers()
                .get("content-encoding")
                .is_some_and(|v| v != "identity")
            {
                return Err(invalid("ENCODED_BODY_NOT_ALLOWED"));
            }
            if response
                .content_length()
                .is_some_and(|size| size > self.policy.max_bytes)
            {
                return Err(invalid("DOWNLOAD_TOO_LARGE"));
            }
            return Ok(OpenResponse {
                response,
                final_url: url.to_string(),
                redirects,
            });
        }
        Err(invalid("REDIRECT_LIMIT"))
    }
    pub fn fetch(&self, url: &str) -> Result<(Vec<u8>, String, Vec<String>)> {
        let mut opened = self.open(url, None)?;
        let mut bytes = Vec::new();
        opened
            .response
            .by_ref()
            .take((SOURCE_LIMIT as u64).min(self.policy.max_bytes) + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > SOURCE_LIMIT || bytes.len() as u64 > self.policy.max_bytes {
            return Err(invalid("SOURCE_TOO_LARGE"));
        }
        Ok((bytes, opened.final_url, opened.redirects))
    }
}

/// Shared pacing gate: callers reserve byte slots against one monotonic timeline.
pub struct ByteBudget {
    rate: u64,
    next: Mutex<Instant>,
    pub bytes: AtomicU64,
    started: Instant,
}
impl ByteBudget {
    pub fn new(bytes_per_second: u64) -> Self {
        Self {
            rate: bytes_per_second,
            next: Mutex::new(Instant::now()),
            bytes: AtomicU64::new(0),
            started: Instant::now(),
        }
    }
    pub fn consume(&self, bytes: usize, stopping: &AtomicBool) -> Result<()> {
        if self.rate == 0 {
            self.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
            return Ok(());
        }
        let until = {
            let mut next = self
                .next
                .lock()
                .map_err(|_| invalid("RATE_BUDGET_FAILED"))?;
            let base = (*next).max(Instant::now());
            *next = base + Duration::from_secs_f64(bytes as f64 / self.rate as f64);
            *next
        };
        while Instant::now() < until {
            if stopping.load(Ordering::Relaxed) {
                return Err(invalid("SERVER_STOPPING"));
            }
            std::thread::sleep(
                until
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(50)),
            );
        }
        self.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        Ok(())
    }
    pub fn elapsed_ms(&self) -> u128 {
        self.started.elapsed().as_millis()
    }
}
#[derive(Serialize, Deserialize)]
struct Resume {
    url: String,
    etag: String,
}
pub struct Downloaded {
    pub sha256: String,
    pub bytes: u64,
    pub final_url: String,
}
pub fn download(
    http: &GuardedHttp,
    url: &str,
    part: &Path,
    max_bytes: u64,
    budget: &ByteBudget,
    stopping: &AtomicBool,
) -> Result<Downloaded> {
    let sidecar = part.with_extension("resume.json");
    let resume = std::fs::read(&sidecar)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Resume>(&bytes).ok());
    let mut offset = std::fs::metadata(part).map_or(0, |m| m.len());
    if offset > max_bytes {
        return Err(invalid("DOWNLOAD_TOO_LARGE"));
    }
    let resume = resume.filter(|r| r.url == url && !r.etag.starts_with("W/") && !r.etag.is_empty());
    if resume.is_none() {
        offset = 0;
    }
    let mut opened = http.open(
        url,
        resume
            .as_ref()
            .filter(|_| offset > 0)
            .map(|r| (offset, r.etag.as_str())),
    )?;
    if offset > 0 && opened.response.status().as_u16() == 206 {
        let range = opened
            .response
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| invalid("INVALID_CONTENT_RANGE"))?;
        let parsed = range
            .strip_prefix("bytes ")
            .and_then(|s| s.split_once('/'))
            .and_then(|(span, total)| Some((span.split_once('-')?, total.parse::<u64>().ok()?)));
        let Some(((start, end), total)) = parsed else {
            return Err(invalid("INVALID_CONTENT_RANGE"));
        };
        if start.parse::<u64>().ok() != Some(offset)
            || end.parse::<u64>().ok() != total.checked_sub(1)
            || total > max_bytes
            || opened
                .response
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                != resume.as_ref().map(|r| r.etag.as_str())
        {
            return Err(invalid("RESUME_CHANGED"));
        }
    } else if opened.response.status().as_u16() == 200 {
        offset = 0;
    } else {
        return Err(invalid("UNEXPECTED_PARTIAL_RESPONSE"));
    }
    let expected = opened.response.content_length();
    if expected.is_some_and(|size| size.saturating_add(offset) > max_bytes) {
        return Err(invalid("DOWNLOAD_TOO_LARGE"));
    }
    let etag = opened
        .response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    std::fs::write(
        &sidecar,
        serde_json::to_vec(&Resume {
            url: url.to_owned(),
            etag,
        })
        .map_err(|_| invalid("RESUME_METADATA_FAILED"))?,
    )?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(offset == 0)
        .read(true)
        .write(true)
        .open(part)?;
    let mut hash = Sha256::new();
    if offset > 0 {
        let mut prefix = Read::by_ref(&mut file).take(offset);
        let copied = std::io::copy(&mut prefix, &mut HashWriter(&mut hash))?;
        if copied != offset {
            return Err(invalid("RESUME_TRUNCATED"));
        }
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut received = 0u64;
    let mut buffer = [0; 64 * 1024];
    let started = Instant::now();
    // Keep each pacing reservation small even at very low configured rates.
    // HTTP timeout and strong-validator resume bound each attempt to 120 s.
    let chunk = if budget.rate == 0 {
        buffer.len()
    } else {
        budget.rate.min(buffer.len() as u64) as usize
    };
    loop {
        if stopping.load(Ordering::Relaxed) {
            return Err(invalid("SERVER_STOPPING"));
        }
        if started.elapsed() >= Duration::from_secs(120) {
            return Err(invalid("DOWNLOAD_TRUNCATED"));
        }
        let count = opened.response.read(&mut buffer[..chunk])?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if offset + received > max_bytes {
            return Err(invalid("DOWNLOAD_TOO_LARGE"));
        }
        budget.consume(count, stopping)?;
        hash.update(&buffer[..count]);
        file.write_all(&buffer[..count])?;
    }
    if expected.is_some_and(|size| size != received) {
        return Err(invalid("DOWNLOAD_TRUNCATED"));
    }
    file.sync_all()?;
    std::fs::remove_file(&sidecar)?;
    Ok(Downloaded {
        sha256: format!("{:x}", hash.finalize()),
        bytes: offset + received,
        final_url: opened.final_url,
    })
}
struct HashWriter<'a>(&'a mut Sha256);
impl Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
