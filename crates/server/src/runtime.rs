use crate::{
    api::{self, Api},
    platform::protected_directory,
};
use orkestr_storage::Store;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::sync::{mpsc, Semaphore};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub listen: SocketAddr,
}
pub fn init(directory: &Path, listen: SocketAddr, dns: String) -> Result<(), String> {
    if orkestr_domain::canonical_host(&dns).is_err() || dns.parse::<std::net::IpAddr>().is_ok() {
        return Err("TLS_DNS_NAME_REQUIRED".into());
    }
    protected_directory(directory, true)?;
    let sid = orkestr_credentials::identity().map_err(|e| e.to_string())?;
    let canary = orkestr_credentials::protect(b"Orkestr EMP service identity v1")
        .map_err(|e| e.to_string())?;
    let issued = crate::new_token(
        "Bootstrap administrator".into(),
        orkestr_protocol::Role::Admin,
        7,
    )?;
    let mut store = Store::open(&directory.join("orkestr.sqlite")).map_err(|e| e.to_string())?;
    store
        .bootstrap_server(
            &sid,
            &canary,
            &issued.actor,
            &crate::hash_token(&issued.token),
        )
        .map_err(|e| e.to_string())?;
    let cert =
        rcgen::generate_simple_self_signed(vec![dns]).map_err(|_| "TLS_CERTIFICATE_FAILED")?;
    write_new(&directory.join("server.crt"), cert.cert.pem().as_bytes())?;
    let key = zeroize::Zeroizing::new(cert.key_pair.serialize_pem());
    write_new(&directory.join("server.key"), key.as_bytes())?;
    write_new(
        &directory.join("server.json"),
        &serde_json::to_vec_pretty(&Config { listen }).map_err(|_| "CONFIG_FAILED")?,
    )?;
    let bootstrap =
        orkestr_credentials::protect(issued.token.as_bytes()).map_err(|e| e.to_string())?;
    write_new(&directory.join("bootstrap-admin.dpapi"), &bootstrap)?;
    protected_directory(directory, false)?;
    Ok(())
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| "NEW_FILE_REQUIRED")?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "FILE_WRITE_FAILED".into())
}
pub fn startup(directory: &Path) -> Result<(Config, Api, mpsc::Receiver<uuid::Uuid>), String> {
    protected_directory(directory, false)?;
    let instance_guard = instance_lock(directory)?;
    let config: Config = serde_json::from_slice(
        &std::fs::read(directory.join("server.json")).map_err(|_| "CONFIG_UNAVAILABLE")?,
    )
    .map_err(|_| "INVALID_CONFIG")?;
    let database = directory.join("orkestr.sqlite");
    if !database.is_file() {
        return Err("DATABASE_MISSING_NO_IMPLICIT_CREATE".into());
    }
    let mut store = Store::open(&database).map_err(|e| e.to_string())?;
    let (owner, blob) = store
        .server_identity()
        .map_err(|e| e.to_string())?
        .ok_or("BOOTSTRAP_REQUIRED")?;
    let canary = orkestr_credentials::unprotect(&owner, &blob).map_err(|e| e.to_string())?;
    if canary.as_slice() != b"Orkestr EMP service identity v1" {
        return Err("DPAPI_CANARY_FAILED".into());
    }
    let trial = orkestr_credentials::protect(b"startup roundtrip").map_err(|e| e.to_string())?;
    if orkestr_credentials::unprotect(&owner, &trial)
        .map_err(|e| e.to_string())?
        .as_slice()
        != b"startup roundtrip"
    {
        return Err("DPAPI_SELF_CHECK_FAILED".into());
    }
    for credential in store.credentials().map_err(|e| e.to_string())? {
        let (_, blob) = store
            .credential_secret(credential.id)
            .map_err(|e| e.to_string())?;
        let _plaintext = orkestr_credentials::unprotect(&credential.owner_sid, &blob)
            .map_err(|e| e.to_string())?;
    }
    store.interrupt_jobs().map_err(|e| e.to_string())?;
    store.recover_deployments().map_err(|e| e.to_string())?;
    store.recover_library_jobs().map_err(|e| e.to_string())?;
    store.recover_resources().map_err(|e| e.to_string())?;
    store.recover_workflows().map_err(|e| e.to_string())?;
    let (jobs, receiver) = mpsc::channel(8);
    Ok((
        config,
        Api {
            store: Arc::new(Mutex::new(store)),
            jobs,
            sid: owner,
            requests: Arc::new(Semaphore::new(16)),
            rate: Arc::new(Mutex::new((std::time::Instant::now(), 0))),
            instance_guard: Some(Arc::new(instance_guard)),
            artifact_directory: Some(directory.join("artifacts")),
            stopping: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        },
        receiver,
    ))
}
pub async fn serve(directory: &Path, handle: axum_server::Handle) -> Result<(), String> {
    serve_ready(directory, handle, |_| {}).await
}
pub async fn serve_ready(
    directory: &Path,
    handle: axum_server::Handle,
    ready: impl FnOnce(SocketAddr),
) -> Result<(), String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (config, api, receiver) = startup(directory)?;
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(
        directory.join("server.crt"),
        directory.join("server.key"),
    )
    .await
    .map_err(|_| "TLS_CONFIGURATION_FAILED")?;
    let listener = std::net::TcpListener::bind(config.listen).map_err(|_| "HTTPS_BIND_FAILED")?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "HTTPS_BIND_FAILED")?;
    let (shutdown, stopping) = tokio::sync::watch::channel(false);
    let workers = api::workers(api.clone(), receiver, stopping).await;
    let stopping_flag = api.stopping.clone();
    ready(listener.local_addr().map_err(|_| "HTTPS_BIND_FAILED")?);
    let result = axum_server::from_tcp_rustls(listener, tls)
        .handle(handle)
        .serve(api::router(api).into_make_service())
        .await
        .map_err(|_| "HTTPS_LISTENER_FAILED".into());
    stopping_flag.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = shutdown.send(true);
    for worker in workers {
        let _ = worker.await;
    }
    result
}
fn instance_lock(directory: &Path) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    options
        .open(directory.join("server.lock"))
        .map_err(|_| "SERVER_ALREADY_RUNNING_OR_LOCK_UNAVAILABLE".into())
}
