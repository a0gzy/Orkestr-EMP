use axum::{
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use orkestr_library::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{atomic::AtomicBool, Arc},
};
#[tokio::test]
async fn guarded_https_resume_hash_redirects_limits_and_tls_fail_closed() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let pem = certificate.cert.pem();
    let key = certificate.key_pair.serialize_pem();
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem(
        pem.as_bytes().to_vec(),
        key.as_bytes().to_vec(),
    )
    .await
    .unwrap();
    let payload = Arc::new((0..32768).map(|i| (i % 251) as u8).collect::<Vec<_>>());
    let file_payload = payload.clone();
    let file = get(move |headers: HeaderMap| {
        let payload = file_payload.clone();
        async move {
            let start = headers
                .get("range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("bytes="))
                .and_then(|v| v.strip_suffix('-'))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            let body = payload[start..].to_vec();
            let mut response = body.into_response();
            response
                .headers_mut()
                .insert("etag", "\"fixture-v1\"".parse().unwrap());
            if start > 0 {
                *response.status_mut() = StatusCode::PARTIAL_CONTENT;
                response.headers_mut().insert(
                    "content-range",
                    format!("bytes {}-{}/{}", start, payload.len() - 1, payload.len())
                        .parse()
                        .unwrap(),
                );
            }
            response
        }
    });
    let reset_payload = payload.clone();
    let reset = get(move || {
        let body = reset_payload.as_ref().clone();
        async move { ([("etag", "\"fixture-v1\"")], body) }
    });
    let app = Router::new()
        .route("/file", file)
        .route("/reset", reset)
        .route(
            "/redirect",
            get(|| async {
                (
                    StatusCode::FOUND,
                    [("location", "https://169.254.169.254/latest/meta-data/")],
                )
            }),
        )
        .route(
            "/encoded",
            get(|| async { ([("content-encoding", "gzip")], "not gzip") }),
        );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = axum_server::Handle::new();
    let stop = handle.clone();
    let server = tokio::spawn(async move {
        axum_server::from_tcp_rustls(listener, tls)
            .handle(stop)
            .serve(app.into_make_service())
            .await
            .unwrap()
    });
    handle.listening().await.unwrap();
    let base = format!("https://localhost:{}", address.port());
    tokio::task::spawn_blocking(move || {
        let mut internal = BTreeMap::new();
        internal.insert(
            "localhost".into(),
            vec!["127.0.0.0/8".parse().unwrap(), "::1/128".parse().unwrap()],
        );
        let policy = NetworkPolicy {
            allowed_hosts: vec!["localhost".into()],
            allowed_ports: vec![address.port()],
            internal_hosts: internal,
            trusted_ca_pem: Some(pem),
            trusted_proxy: None,
            max_bytes: 65536,
        };
        let http = GuardedHttp::new(&policy).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("file.part");
        let url = format!("{base}/file");
        std::fs::write(&part, &payload[..10000]).unwrap();
        std::fs::write(
            part.with_extension("resume.json"),
            serde_json::to_vec(&serde_json::json!({"url":url,"etag":"\"fixture-v1\""})).unwrap(),
        )
        .unwrap();
        let downloaded = download(
            &http,
            &url,
            &part,
            65536,
            &ByteBudget::new(0),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(downloaded.bytes, payload.len() as u64);
        assert_eq!(
            downloaded.sha256,
            format!("{:x}", Sha256::digest(payload.as_slice()))
        );
        assert_eq!(std::fs::read(&part).unwrap(), *payload);
        let reset_url = format!("{base}/reset");
        std::fs::write(&part, b"garbage").unwrap();
        std::fs::write(
            part.with_extension("resume.json"),
            serde_json::to_vec(&serde_json::json!({"url":reset_url,"etag":"\"fixture-v1\""}))
                .unwrap(),
        )
        .unwrap();
        download(
            &http,
            &reset_url,
            &part,
            65536,
            &ByteBudget::new(0),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), *payload);
        assert!(http.fetch(&format!("{base}/redirect")).is_err());
        assert!(http.fetch(&format!("{base}/encoded")).is_err());
        let mut small = policy.clone();
        small.max_bytes = 100;
        assert!(GuardedHttp::new(&small).unwrap().fetch(&url).is_err());
        let mut bad_ca = policy.clone();
        bad_ca.trusted_ca_pem = None;
        assert!(GuardedHttp::new(&bad_ca).unwrap().fetch(&url).is_err());
        let mut bad_name = policy;
        bad_name.allowed_hosts.push("127.0.0.1".into());
        bad_name
            .internal_hosts
            .insert("127.0.0.1".into(), vec!["127.0.0.1/32".parse().unwrap()]);
        assert!(GuardedHttp::new(&bad_name)
            .unwrap()
            .fetch(&format!("https://127.0.0.1:{}/file", address.port()))
            .is_err());
    })
    .await
    .unwrap();
    handle.shutdown();
    server.await.unwrap();
}
