//! Blocking HTTPS transport, always used off the egui thread. No TLS bypass flag.
use orkestr_application::{Command, Event, Service, Workspace};
use orkestr_domain::*;
use orkestr_protocol::*;
use reqwest::{blocking::Client as Http, Method};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    io::Read,
    path::Path,
    sync::{mpsc, Arc},
    time::Duration,
};
use zeroize::Zeroizing;

pub struct Client {
    http: Http,
    origin: String,
    token: Zeroizing<String>,
}
impl Client {
    pub fn new(origin: &str, token: Zeroizing<String>, ca: Option<&[u8]>) -> Result<Self, String> {
        let url = reqwest::Url::parse(origin).map_err(|_| "INVALID_SERVER_URL")?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err("HTTPS_ORIGIN_REQUIRED".into());
        }
        if token.len() != 64 || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("INVALID_TOKEN_FORMAT".into());
        }
        let mut builder = Http::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .no_proxy();
        if let Some(ca) = ca {
            builder = builder.add_root_certificate(
                reqwest::Certificate::from_pem(ca).map_err(|_| "INVALID_CA_CERTIFICATE")?,
            );
        }
        let http = builder.build().map_err(|_| "HTTPS_CLIENT_FAILED")?;
        Ok(Self {
            http,
            origin: origin.trim_end_matches('/').into(),
            token,
        })
    }
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        self.request(Method::GET, path, None, None)
    }
    pub fn post<I: Serialize, O: DeserializeOwned>(
        &self,
        path: &str,
        input: &I,
        key: Option<&str>,
    ) -> Result<O, String> {
        let body = Zeroizing::new(serde_json::to_vec(input).map_err(|_| "INVALID_REQUEST")?);
        self.request(Method::POST, path, Some(&body), key)
    }
    fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&[u8]>,
        key: Option<&str>,
    ) -> Result<T, String> {
        if !path.starts_with("/v1/") || path.contains("..") || path.contains('#') {
            return Err("INVALID_API_PATH".into());
        }
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.origin))
            .bearer_auth(&*self.token);
        if let Some(body) = body {
            if body.len() > API_BODY_LIMIT {
                return Err("BODY_TOO_LARGE".into());
            }
            request = request
                .header("Content-Type", "application/json")
                .body(body.to_vec());
        }
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key);
        }
        let response = request.send().map_err(|_| {
            "HTTPS_CONNECTION_FAILED: check server, certificate trust, SAN and validity"
        })?;
        let status = response.status();
        let mut bytes = Zeroizing::new(vec![]);
        response
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "RESPONSE_READ_FAILED")?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("RESPONSE_TOO_LARGE".into());
        }
        if !status.is_success() {
            // Server-owned fixed codes only, never raw remote body/URLs/token in error messages.
            let code = serde_json::from_slice::<ApiError>(&bytes)
                .ok()
                .map(|e| e.code)
                .filter(|s| {
                    s.len() <= 100 && s.bytes().all(|c| c.is_ascii_uppercase() || c == b'_')
                })
                .unwrap_or_else(|| format!("HTTP_{}", status.as_u16()));
            return Err(code);
        }
        serde_json::from_slice(&bytes).map_err(|_| "INVALID_API_RESPONSE".into())
    }
    pub fn service(self, wake: Arc<dyn Fn() + Send + Sync>) -> Service {
        let (sender, commands) = mpsc::sync_channel(8);
        let (events, receiver) = mpsc::sync_channel(8);
        std::thread::spawn(move || {
            let send = |e| {
                let ok = events.send(e).is_ok();
                wake();
                ok
            };
            let refresh = || self.get::<Workspace>("/v1/workspace").map(Arc::new);
            match refresh() {
                Ok(w) => {
                    if !send(Event::Loaded(w)) {
                        return;
                    }
                }
                Err(message) => {
                    send(Event::Error {
                        operation: "connect",
                        token: None,
                        message,
                    });
                    return;
                }
            }
            while let Ok(command) = commands.recv() {
                let (operation,token,result)=match command{
                    Command::ReadDiscoveryFile{token,path} => ("discovery file",Some(token),orkestr_application::read_discovery_file(&path).map(|text|Event::DiscoveryText{token,text}).map_err(|e|e.to_string())),
                    Command::PrepareDiscovery{token,input} => ("discovery preview",Some(token),self.post("/v1/discovery/preview",&input,None).map(|preview|Event::DiscoveryPrepared{token,preview})),
                    Command::ApplyDiscovery{token,input} => ("discovery apply",Some(token),self.post("/v1/discovery/apply",&input,None).map(|result|Event::DiscoveryApplied{token,result})),
                    Command::DiscardDiscovery => { let _ = self.post::<_,()>("/v1/discovery/discard",&(),None); continue; },
                    Command::InventoryStatus{token,device_id} => ("inventory status",Some(token),self.get(&format!("/v1/devices/{device_id}/inventory-status")).map(|status|Event::InventoryStatus{token,status})),
                    Command::Refresh=>("refresh",None,refresh().map(Event::Loaded)),
                    Command::Preview{token,selection}=>("preview",Some(token),self.post("/v1/preview",&selection,None).map(|preview|Event::Preview{token,preview})),
                    Command::Save{collection,expected}=>("save",None,self.post::<_,Collection>("/v1/collections",&SaveCollection{collection:*collection,expected},None).and_then(|c|refresh().map(|workspace|Event::Saved{id:c.id,workspace}))),
                    Command::Delete{id,expected}=>("delete",None,self.post::<_,()>("/v1/collections/delete",&DeleteCollection{id,expected},None).and_then(|()|refresh().map(Event::Deleted))),
                    Command::PrepareImport{token,..}|Command::ApplyImport{token,..}=>("import remote",Some(token),Err("REMOTE_IMPORT_DISABLED: import chosen copies locally before server provisioning".into())),
                    Command::DiscardImport=>continue,
                };
                if !send(result.unwrap_or_else(|message| Event::Error {
                    operation,
                    token,
                    message,
                })) {
                    break;
                }
            }
        });
        Service::from_transport(sender, receiver)
    }
}

/// CurrentUser local token file; caller chooses a new path, never overwrite or auto-load.
pub fn save_token(path: &Path, token: &str) -> Result<(), String> {
    use std::io::Write;
    let blob = orkestr_credentials::protect(token.as_bytes()).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "NEW_TOKEN_FILE_REQUIRED")?;
    file.write_all(&blob)
        .and_then(|()| file.sync_all())
        .map_err(|_| "TOKEN_FILE_WRITE_FAILED".into())
}
pub fn load_token(path: &Path) -> Result<Zeroizing<String>, String> {
    let mut blob = vec![];
    std::fs::File::open(path)
        .map_err(|_| "TOKEN_FILE_UNAVAILABLE")?
        .take(65537)
        .read_to_end(&mut blob)
        .map_err(|_| "TOKEN_FILE_UNAVAILABLE")?;
    if blob.len() > 65536 {
        return Err("INVALID_TOKEN_FILE".into());
    }
    let owner = orkestr_credentials::identity().map_err(|e| e.to_string())?;
    let plain = orkestr_credentials::unprotect(&owner, &blob).map_err(|e| e.to_string())?;
    let token = Zeroizing::new(
        std::str::from_utf8(&plain)
            .map_err(|_| "INVALID_TOKEN_FILE")?
            .to_string(),
    );
    if token.len() != 64 || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("INVALID_TOKEN_FORMAT".into());
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_http_userinfo_queries_and_paths() {
        for url in [
            "http://localhost:7443",
            "https://user@localhost",
            "https://localhost/v1",
            "https://localhost/?token=x",
            "https://localhost/#x",
        ] {
            assert!(Client::new(url, Zeroizing::new("a".repeat(64)), None).is_err());
        }
    }
    #[test]
    #[cfg(windows)]
    fn local_tokens_are_dpapi_and_never_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.dpapi");
        let token = "b".repeat(64);
        save_token(&path, &token).unwrap();
        assert_eq!(&*load_token(&path).unwrap(), &token);
        assert!(save_token(&path, &token).is_err());
        assert_ne!(std::fs::read(path).unwrap(), token.as_bytes());
    }
}
