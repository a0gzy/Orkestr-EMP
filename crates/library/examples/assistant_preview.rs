//! Opt-in official-source read/download only; never installs software.
use orkestr_domain::ApplicationId;
use orkestr_library::*;
use std::{path::PathBuf, sync::atomic::AtomicBool};
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("output directory required")?;
    std::fs::create_dir_all(&output)?;
    let app: LibraryApplication = serde_json::from_value(
        serde_json::json!({"id":ApplicationId::new(),"revision":1,"name":"Ассистент","channel":"stable","architecture":"x86","source":{"kind":"Html","url":"https://xn--80akicokc0aablc.xn--p1ai/скачать?platformId=1","selector":"[data-platform-current-version]","attribute":null,"pattern":"Актуальная версия: (?P<version>[0-9]+\\.[0-9]+)"},"download":{"kind":"HtmlLink","page_url":"https://xn--80akicokc0aablc.xn--p1ai/скачать?platformId=1","container_selector":"[data-platform-content] .download__tab-content-item","title_selector":"h4","title":"Полная версия","link_selector":"a.button__link"},"network":{"allowed_hosts":["xn--80akicokc0aablc.xn--p1ai","lk3.xn--80akicokc0aablc.xn--p1ai"],"max_bytes":268435456},"verification":{"kind":"Authenticode","publisher":"E=admin@safib.ru, CN=SAFIB LLC, O=SAFIB LLC, L=Voronezh, S=Voronezh Oblast, C=RU"},"software_match":{"name":"Ассистент","publisher":"САФИБ","product_code":null,"version_components":2}}),
    )?;
    app.validate()?;
    let http = GuardedHttp::new(&app.network)?;
    let (bytes, url, redirects) = http.fetch(app.source.url().unwrap())?;
    let preview = preview_source(
        &app,
        &bytes,
        Some(url.clone()),
        redirects,
        Some((&url, &bytes)),
    )?;
    std::fs::write(
        output.join("preview.json"),
        serde_json::to_vec_pretty(&preview)?,
    )?;
    let downloaded = download(
        &http,
        &preview.url,
        &output.join("assistant-7.1-install.exe"),
        app.network.max_bytes,
        &ByteBudget::new(4 * 1024 * 1024),
        &AtomicBool::new(false),
    )?;
    std::fs::write(
        output.join("application.json"),
        serde_json::to_vec_pretty(&app)?,
    )?;
    println!(
        "VERSION {}\nSHA256 {}\nBYTES {}\nFINAL_URL {}",
        preview.version, downloaded.sha256, downloaded.bytes, downloaded.final_url
    );
    Ok(())
}
