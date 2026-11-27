use orkestr_domain::ApplicationId;
use orkestr_library::*;
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
fn app(source: VersionSource) -> LibraryApplication {
    LibraryApplication {
        id: ApplicationId::new(),
        revision: 1,
        name: "Fixture".into(),
        channel: "stable".into(),
        architecture: "x64".into(),
        source,
        download: DownloadSource::Template {
            url: "https://repo.example.test/{channel}/{version}/fixture-{arch}.exe".into(),
        },
        network: NetworkPolicy {
            allowed_hosts: vec!["repo.example.test".into()],
            allowed_ports: vec![443],
            internal_hosts: BTreeMap::new(),
            trusted_ca_pem: None,
            trusted_proxy: None,
            max_bytes: 1024 * 1024,
        },
        verification: VerificationPolicy::Hash {
            expected_sha256: "a".repeat(64),
        },
        software_match: SoftwareMatch {
            version_components: None,
            name: "Fixture".into(),
            publisher: "Publisher".into(),
            product_code: None,
        },
    }
}
#[test]
fn json_pointer_html_regex_and_manual_produce_evidence_and_reject_ambiguity() {
    let json = app(VersionSource::Json {
        url: "https://repo.example.test/releases".into(),
        pointer: "/a~1b/~0version".into(),
        pattern: None,
    });
    let preview = preview_source(
        &json,
        br#"{"a/b":{"~version":"v2.10.0"}}"#,
        None,
        vec![],
        None,
    )
    .unwrap();
    assert_eq!(preview.version, "2.10");
    assert!(preview.url.ends_with("/stable/2.10.0/fixture-x64.exe"));
    assert_eq!(preview.evidence.response_sha256.len(), 64);
    assert!(json
        .source
        .extract(br#"{"a/b":{"~version":null}}"#)
        .is_err());
    let html = app(VersionSource::Html {
        url: "https://repo.example.test/releases".into(),
        selector: ".version".into(),
        attribute: Some("data-version".into()),
        pattern: None,
    });
    assert_eq!(
        html.source
            .extract(br#"<b class="version" data-version="3.4"></b>"#)
            .unwrap()
            .1,
        "3.4"
    );
    assert!(html
        .source
        .extract(br#"<b class="version"></b><b class="version"></b>"#)
        .is_err());
    let regex = app(VersionSource::Text {
        url: "https://repo.example.test/releases".into(),
        pattern: r"release (?P<version>\d+\.\d+)".into(),
    });
    assert_eq!(regex.source.extract(b"release 1.2").unwrap().1, "1.2");
    assert!(regex.source.extract(b"release 1.2 release 1.3").is_err());
    assert!(app(VersionSource::Text {
        url: "https://repo.example.test/releases".into(),
        pattern: r"(?<=v)(?P<version>\d+)".into()
    })
    .validate()
    .is_err());
    assert!(app(VersionSource::Manual {
        version: "1.0-rc1".into()
    })
    .validate()
    .is_err());
}
#[test]
fn url_templates_cannot_replace_authority_and_link_versions_are_checked() {
    let mut app = app(VersionSource::Manual {
        version: "2.0".into(),
    });
    app.download = DownloadSource::Template {
        url: "https://{channel}/file.exe".into(),
    };
    assert!(app.validate().is_err());
    let source = DownloadSource::Link {
        page_url: "https://repo.example.test/list".into(),
        selector: "a.download".into(),
        pattern: r"/app-(?P<version>\d+\.\d+)-x64\.exe$".into(),
    };
    let page=br#"<a class="download" href="/app-2.0-x64.exe"></a><a class="download" href="/app-1.0-x64.exe"></a>"#;
    assert_eq!(
        source
            .resolve(
                "2",
                "stable",
                "x64",
                Some(("https://repo.example.test/list", page)),
                &app.network
            )
            .unwrap(),
        "https://repo.example.test/app-2.0-x64.exe"
    );
    assert!(source
        .resolve(
            "3",
            "stable",
            "x64",
            Some(("https://repo.example.test/list", page)),
            &app.network
        )
        .is_err());
    let excessive = format!(
        "{}<a class=\"download\" href=\"/app-2.0-x64.exe\"></a>",
        "<a class=\"download\" href=\"/ignored\"></a>".repeat(1024)
    );
    assert!(source
        .resolve(
            "2",
            "stable",
            "x64",
            Some(("https://repo.example.test/list", excessive.as_bytes())),
            &app.network
        )
        .is_err());
}
#[test]
fn ssrf_checks_ipv4_ipv6_credentials_ports_and_exact_internal_exceptions() {
    let app = app(VersionSource::Manual {
        version: "1".into(),
    });
    for blocked in [
        "https://user:secret@repo.example.test/x",
        "http://repo.example.test/x",
        "https://repo.example.test:8443/x",
        "https://unlisted.example.test/x",
        "file:///C:/Windows/a",
    ] {
        assert!(app.network.check_url(blocked).is_err(), "{blocked}");
    }
    for blocked in [
        "127.0.0.1",
        "169.254.169.254",
        "100.64.0.1",
        "10.0.0.1",
        "224.1.1.1",
        "192.0.0.1",
        "::1",
        "::ffff:127.0.0.1",
        "fe80::1",
        "fd00::1",
        "2001:db8::1",
        "2002:7f00:1::",
    ] {
        assert!(!public_ip(blocked.parse().unwrap()), "{blocked}");
    }
    assert!(public_ip("8.8.8.8".parse().unwrap()));
    assert!(public_ip("2606:4700:4700::1111".parse().unwrap()));
    let mut policy = app.network;
    policy.internal_hosts.insert(
        "repo.example.test".into(),
        vec!["10.4.0.0/16".parse().unwrap()],
    );
    assert!(policy
        .check_ip("repo.example.test", "10.4.1.1".parse().unwrap())
        .is_ok());
    assert!(policy
        .check_ip("other.example.test", "10.4.1.1".parse().unwrap())
        .is_err());
}
#[test]
fn rate_budget_paces_payload_and_can_be_interrupted() {
    let budget = ByteBudget::new(1024 * 1024);
    let stop = AtomicBool::new(false);
    budget.consume(128 * 1024, &stop).unwrap();
    assert!(budget.elapsed_ms() >= 120);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(budget.consume(128 * 1024, &stop).is_err());
}

#[test]
fn assistant_link_is_selected_by_heading_and_tracks_changed_download_id() {
    let mut app = app(VersionSource::Manual {
        version: "7.1".into(),
    });
    app.network
        .allowed_hosts
        .push("downloads.example.test".into());
    let link = DownloadSource::HtmlLink {
        page_url: "https://repo.example.test/download".into(),
        container_selector: ".item".into(),
        title_selector: "h4".into(),
        title: "Полная версия".into(),
        link_selector: "a.download".into(),
    };
    for id in [1638, 9999] {
        let body = format!(
            r#"<div class="item"><h4>Полная версия </h4><a class="download" href="https://downloads.example.test/WebApi/Platforms/Download/{id}">Скачать</a></div><div class="item"><h4>Полная версия (WinXP)</h4><a class="download" href="/wrong">Скачать</a></div>"#
        );
        assert!(link
            .resolve(
                "7.1",
                "stable",
                "x64",
                Some(("https://repo.example.test/download", body.as_bytes())),
                &app.network
            )
            .unwrap()
            .ends_with(&format!("/{id}")));
        let duplicate = format!("{body}{body}");
        assert!(link
            .resolve(
                "7.1",
                "stable",
                "x64",
                Some(("https://repo.example.test/download", duplicate.as_bytes())),
                &app.network
            )
            .is_err());
    }
}
