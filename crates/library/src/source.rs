use crate::*;
use orkestr_domain::NumericVersion;
use regex::{Regex, RegexBuilder};
use scraper::{Html, Selector};
use sha2::{Digest, Sha256};
use url::Url;

pub const SOURCE_LIMIT: usize = 2 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum VersionSource {
    Json {
        url: String,
        pointer: String,
        pattern: Option<String>,
    },
    Html {
        url: String,
        selector: String,
        attribute: Option<String>,
        pattern: Option<String>,
    },
    Text {
        url: String,
        pattern: String,
    },
    Manual {
        version: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DownloadSource {
    LocalArtifact {
        sha256: String,
        extension: String,
    },
    HtmlLink {
        page_url: String,
        container_selector: String,
        title_selector: String,
        title: String,
        link_selector: String,
    },
    Template {
        url: String,
    },
    Link {
        page_url: String,
        selector: String,
        pattern: String,
    },
    Manual {
        url: String,
    },
}
fn regex(pattern: &str) -> Result<Regex> {
    if pattern.len() > 2048 {
        return Err(invalid("PATTERN_TOO_LONG"));
    }
    RegexBuilder::new(pattern)
        .size_limit(1024 * 1024)
        .dfa_size_limit(1024 * 1024)
        .build()
        .map_err(|_| invalid("INVALID_PATTERN"))
}
fn capture(pattern: &str, text: &str) -> Result<String> {
    let re = regex(pattern)?;
    let mut matches = re.captures_iter(text);
    let first = matches.next().ok_or_else(|| invalid("SOURCE_NO_MATCH"))?;
    if matches.next().is_some() {
        return Err(invalid("SOURCE_AMBIGUOUS"));
    }
    first
        .name("version")
        .filter(|value| !value.as_str().is_empty())
        .map(|value| value.as_str().to_owned())
        .ok_or_else(|| invalid("VERSION_CAPTURE_REQUIRED"))
}
fn selected(body: &str, selector: &str, attribute: Option<&str>) -> Result<String> {
    let selector = Selector::parse(selector).map_err(|_| invalid("INVALID_SELECTOR"))?;
    let document = Html::parse_document(body);
    let mut nodes = document.select(&selector);
    let node = nodes.next().ok_or_else(|| invalid("SOURCE_NO_MATCH"))?;
    if nodes.next().is_some() {
        return Err(invalid("SOURCE_AMBIGUOUS"));
    }
    let value = match attribute {
        Some(attr) => node
            .value()
            .attr(attr)
            .ok_or_else(|| invalid("SOURCE_ATTRIBUTE_MISSING"))?
            .to_owned(),
        None => node.text().collect::<String>(),
    };
    Ok(value.trim().to_owned())
}
impl VersionSource {
    pub fn url(&self) -> Option<&str> {
        match self {
            Self::Json { url, .. } | Self::Html { url, .. } | Self::Text { url, .. } => Some(url),
            Self::Manual { .. } => None,
        }
    }
    pub fn validate(&self, network: &NetworkPolicy) -> Result<()> {
        if let Some(url) = self.url() {
            network.check_url(url)?;
        }
        match self {
            Self::Json {
                pointer, pattern, ..
            } => {
                if pointer.len() > 1024 || (!pointer.is_empty() && !pointer.starts_with('/')) {
                    return Err(invalid("INVALID_JSON_POINTER"));
                }
                // RFC 6901 permits only ~0 and ~1 escapes.
                let mut chars = pointer.chars();
                while let Some(c) = chars.next() {
                    if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
                        return Err(invalid("INVALID_JSON_POINTER"));
                    }
                }
                if let Some(pattern) = pattern {
                    regex(pattern)?;
                }
            }
            Self::Html {
                selector,
                attribute,
                pattern,
                ..
            } => {
                if selector.len() > 2048
                    || attribute
                        .as_ref()
                        .is_some_and(|a| a.is_empty() || a.len() > 128)
                {
                    return Err(invalid("INVALID_SELECTOR"));
                }
                Selector::parse(selector).map_err(|_| invalid("INVALID_SELECTOR"))?;
                if let Some(pattern) = pattern {
                    regex(pattern)?;
                }
            }
            Self::Text { pattern, .. } => {
                regex(pattern)?;
            }
            Self::Manual { version } => {
                NumericVersion::parse(version).map_err(|_| invalid("VERSION_UNKNOWN"))?;
            }
        }
        Ok(())
    }
    pub fn extract(&self, bytes: &[u8]) -> Result<(String, String)> {
        if bytes.len() > SOURCE_LIMIT {
            return Err(invalid("SOURCE_TOO_LARGE"));
        }
        let body = std::str::from_utf8(bytes).map_err(|_| invalid("SOURCE_NOT_UTF8"))?;
        let raw = match self {
            Self::Json {
                pointer, pattern, ..
            } => {
                let document: serde_json::Value =
                    serde_json::from_str(body).map_err(|_| invalid("INVALID_SOURCE_JSON"))?;
                let value = document
                    .pointer(pointer)
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("SOURCE_VALUE_MISSING"))?;
                if let Some(pattern) = pattern {
                    capture(pattern, value)?
                } else {
                    value.trim().to_owned()
                }
            }
            Self::Html {
                selector,
                attribute,
                pattern,
                ..
            } => {
                let value = selected(body, selector, attribute.as_deref())?;
                if let Some(pattern) = pattern {
                    capture(pattern, &value)?
                } else {
                    value
                }
            }
            Self::Text { pattern, .. } => capture(pattern, body)?,
            Self::Manual { version } => version.trim().to_owned(),
        };
        let normalized = NumericVersion::parse(&raw)
            .map_err(|_| invalid("VERSION_UNKNOWN"))?
            .to_string();
        Ok((raw, normalized))
    }
}
impl DownloadSource {
    pub fn validate(&self, network: &NetworkPolicy) -> Result<()> {
        match self {
            Self::LocalArtifact { sha256, extension } => {
                if !valid_hash(sha256) || !matches!(extension.as_str(), "msi" | "exe") {
                    return Err(invalid("INVALID_LOCAL_ARTIFACT"));
                }
            }
            Self::HtmlLink {
                page_url,
                container_selector,
                title_selector,
                title,
                link_selector,
            } => {
                network.check_url(page_url)?;
                for selector in [container_selector, title_selector, link_selector] {
                    if selector.len() > 2048 {
                        return Err(invalid("INVALID_SELECTOR"));
                    }
                    Selector::parse(selector).map_err(|_| invalid("INVALID_SELECTOR"))?;
                }
                if title.trim().is_empty() || title.len() > 128 {
                    return Err(invalid("INVALID_LINK_TITLE"));
                }
            }
            Self::Template { url } => {
                let value = url
                    .replace("{version}", "1.2")
                    .replace("{arch}", "x64")
                    .replace("{channel}", "stable");
                if value.contains(['{', '}']) {
                    return Err(invalid("UNKNOWN_URL_VARIABLE"));
                }
                let parsed = network.check_url(&value)?;
                let authority = url.split('/').take(3).collect::<Vec<_>>().join("/");
                if authority.contains(['{', '}']) || parsed.fragment().is_some() {
                    return Err(invalid("URL_TEMPLATE_AUTHORITY"));
                }
            }
            Self::Link {
                page_url,
                selector,
                pattern,
            } => {
                network.check_url(page_url)?;
                Selector::parse(selector).map_err(|_| invalid("INVALID_SELECTOR"))?;
                regex(pattern)?;
            }
            Self::Manual { url } => {
                network.check_url(url)?;
            }
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        version: &str,
        channel: &str,
        architecture: &str,
        page: Option<(&str, &[u8])>,
        network: &NetworkPolicy,
    ) -> Result<String> {
        let url = match self {
            Self::LocalArtifact { sha256, extension } => {
                return Ok(format!("artifact:{sha256}.{extension}"))
            }
            Self::HtmlLink {
                container_selector,
                title_selector,
                title,
                link_selector,
                ..
            } => {
                let (base, body) = page.ok_or_else(|| invalid("DOWNLOAD_PAGE_REQUIRED"))?;
                if body.len() > SOURCE_LIMIT {
                    return Err(invalid("SOURCE_TOO_LARGE"));
                }
                let document = Html::parse_document(
                    std::str::from_utf8(body).map_err(|_| invalid("SOURCE_NOT_UTF8"))?,
                );
                let container =
                    Selector::parse(container_selector).map_err(|_| invalid("INVALID_SELECTOR"))?;
                let heading =
                    Selector::parse(title_selector).map_err(|_| invalid("INVALID_SELECTOR"))?;
                let link =
                    Selector::parse(link_selector).map_err(|_| invalid("INVALID_SELECTOR"))?;
                let normalize =
                    |value: &str| value.split_whitespace().collect::<Vec<_>>().join(" ");
                let expected = normalize(title);
                let mut found = vec![];
                let nodes = document.select(&container).take(1025).collect::<Vec<_>>();
                if nodes.len() > 1024 {
                    return Err(invalid("LINK_CONTAINER_LIMIT"));
                }
                for node in nodes {
                    let headings = node.select(&heading).collect::<Vec<_>>();
                    if headings.len() != 1
                        || normalize(&headings[0].text().collect::<String>()) != expected
                    {
                        continue;
                    }
                    let links = node.select(&link).collect::<Vec<_>>();
                    if links.len() != 1 {
                        return Err(invalid("DOWNLOAD_LINK_AMBIGUOUS"));
                    }
                    let href = links[0]
                        .value()
                        .attr("href")
                        .ok_or_else(|| invalid("DOWNLOAD_LINK_MISSING"))?;
                    let base = Url::parse(base).map_err(|_| invalid("INVALID_URL"))?;
                    let url = base.join(href).map_err(|_| invalid("INVALID_URL"))?;
                    network.check_url(url.as_str())?;
                    found.push(url.to_string());
                }
                if found.len() != 1 {
                    return Err(invalid("DOWNLOAD_LINK_AMBIGUOUS"));
                }
                found.remove(0)
            }
            Self::Template { url } => {
                let encode = |s: &str| {
                    url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>()
                };
                url.replace("{version}", &encode(version))
                    .replace("{arch}", &encode(architecture))
                    .replace("{channel}", &encode(channel))
            }
            Self::Manual { url } => url.clone(),
            Self::Link {
                selector, pattern, ..
            } => {
                let (base, body) = page.ok_or_else(|| invalid("DOWNLOAD_PAGE_REQUIRED"))?;
                if body.len() > SOURCE_LIMIT {
                    return Err(invalid("SOURCE_TOO_LARGE"));
                }
                let body = std::str::from_utf8(body).map_err(|_| invalid("SOURCE_NOT_UTF8"))?;
                let document = Html::parse_document(body);
                let selector =
                    Selector::parse(selector).map_err(|_| invalid("INVALID_SELECTOR"))?;
                let filter = regex(pattern)?;
                let base = Url::parse(base).map_err(|_| invalid("INVALID_URL"))?;
                let mut candidates = Vec::new();
                let nodes = document.select(&selector).take(1025).collect::<Vec<_>>();
                if nodes.len() > 1024 {
                    return Err(invalid("LINK_CONTAINER_LIMIT"));
                }
                for node in nodes {
                    let Some(href) = node.value().attr("href") else {
                        continue;
                    };
                    let url = base.join(href).map_err(|_| invalid("INVALID_URL"))?;
                    let mut matches = filter.captures_iter(url.path());
                    if let Some(captures) = matches.next() {
                        if matches.next().is_some() {
                            return Err(invalid("DOWNLOAD_LINK_AMBIGUOUS"));
                        }
                        let found = captures
                            .name("version")
                            .ok_or_else(|| invalid("VERSION_CAPTURE_REQUIRED"))?;
                        if NumericVersion::compare(found.as_str(), version)
                            != Some(std::cmp::Ordering::Equal)
                        {
                            continue;
                        }
                        network.check_url(url.as_str())?;
                        candidates.push(url.to_string());
                    }
                }
                if candidates.len() != 1 {
                    return Err(invalid("DOWNLOAD_LINK_AMBIGUOUS"));
                }
                candidates.remove(0)
            }
        };
        Ok(network.check_url(&url)?.to_string())
    }
}
pub fn preview_source(
    application: &LibraryApplication,
    source: &[u8],
    source_url: Option<String>,
    redirects: Vec<String>,
    page: Option<(&str, &[u8])>,
) -> Result<SourcePreview> {
    application.validate()?;
    let (version_raw, version) = application.source.extract(source)?;
    let full_version = version_raw
        .trim()
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse::<u64>().map(|part| part.to_string()))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| invalid("VERSION_UNKNOWN"))?
        .join(".");
    let url = application.download.resolve(
        &full_version,
        &application.channel,
        &application.architecture,
        page,
        &application.network,
    )?;
    Ok(SourcePreview {
        version_raw,
        version,
        url,
        evidence: SourceEvidence {
            download_page_url: page.map(|(url, _)| url.to_owned()),
            download_page_sha256: page.map(|(_, body)| format!("{:x}", Sha256::digest(body))),
            download_page_redirects: vec![],
            checked_at: Utc::now(),
            response_sha256: format!("{:x}", Sha256::digest(source)),
            final_url: source_url,
            redirects,
            matches: 1,
        },
    })
}
