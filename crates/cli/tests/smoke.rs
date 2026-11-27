use std::{
    path::Path,
    process::{Command, Output},
};
use tempfile::tempdir;

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orkestr-cli"))
        .args(args)
        .output()
        .unwrap()
}
fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}
fn success(args: &[&str]) -> String {
    let output = cli(args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn cli_smoke_import_preview_backup_restart_and_no_implicit_creation() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("orkestr.sqlite");
    let copy = dir.path().join("copy.json");
    let backup = dir.path().join("rollback.sqlite");
    assert!(success(&["--help"]).contains("DRY-RUN"));
    assert!(!cli(&["list", text(&db)]).status.success());
    assert!(!db.exists());
    success(&["init", text(&db)]);
    assert!(!cli(&["init", text(&db)]).status.success());
    assert!(success(&["list", text(&db)]).contains("devices=0"));
    std::fs::write(&copy,r#"{"packages":[{"id":1,"name":"CLI fixture","steps":[{"kind":"Sleep","seconds":1}]}],"schedules":[],"deployments":[]}"#).unwrap();
    let before = std::fs::read(&copy).unwrap();
    assert!(
        success(&["import", "deploy", text(&db), text(&copy)]).contains("DRY-RUN: nothing written")
    );
    assert!(!success(&["list", text(&db)]).contains("CLI fixture"));
    assert!(success(&[
        "import",
        "deploy",
        text(&db),
        text(&copy),
        "--apply",
        text(&backup)
    ])
    .contains("Applied"));
    assert!(success(&["list", text(&db)]).contains("CLI fixture"));
    // Restore check on a NEW database path, never overwrite an open WAL database.
    let restored = dir.path().join("restored.sqlite");
    std::fs::copy(&backup, &restored).unwrap();
    assert!(!success(&["list", text(&restored)]).contains("CLI fixture"));
    assert_eq!(std::fs::read(&copy).unwrap(), before);
    let report = success(&["import", "deploy", text(&db), text(&copy)]);
    assert!(report.contains("DRY-RUN"));
}

#[test]
fn cli_groups_require_revision_and_safe_errors_do_not_echo_source() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("orkestr.sqlite");
    let group = dir.path().join("group.json");
    success(&["init", text(&db)]);
    std::fs::write(&group,r#"{"id":"802d76c0-34af-477a-9c52-96d4697c1ad4","parent_id":null,"name":"CLI folder","revision":0,"kind":"Folder"}"#).unwrap();
    success(&["save-group", text(&db), text(&group), "new"]);
    assert!(!cli(&["save-group", text(&db), text(&group), "new"])
        .status
        .success());
    success(&["save-group", text(&db), text(&group), "1"]);
    assert!(success(&[
        "preview-group",
        text(&db),
        "802d76c0-34af-477a-9c52-96d4697c1ad4"
    ])
    .contains("members=0"));
    let bad = dir.path().join("bad.json");
    std::fs::write(
        &bad,
        r#"{"Ram":{"op":"SYNTHETIC-SECRET-DO-NOT-ECHO","bytes":1}}"#,
    )
    .unwrap();
    let output = cli(&["preview-rule", text(&db), text(&bad)]);
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("SYNTHETIC-SECRET"));
    success(&[
        "delete-group",
        text(&db),
        "802d76c0-34af-477a-9c52-96d4697c1ad4",
        "2",
    ]);
}

#[test]
fn cli_discovery_is_dry_run_first_bounded_and_requires_backup_for_apply() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("core.sqlite");
    let source = dir.path().join("devices.csv");
    let backup = dir.path().join("before.sqlite");
    std::fs::write(
        &source,
        "host,name,site\nPC-01,Учебный ПК,Офис\npc-01,Учебный ПК,Офис\n",
    )
    .unwrap();
    let before = std::fs::read(&source).unwrap();
    assert!(!cli(&["discover", "csv", text(&db), text(&source)])
        .status
        .success());
    assert!(!db.exists());
    success(&["init", text(&db)]);
    let preview = success(&["discover", "csv", text(&db), text(&source)]);
    assert!(preview.contains("DRY-RUN"));
    assert!(preview.contains("Duplicate"));
    assert!(success(&["list", text(&db)]).contains("devices=0"));
    let applied = success(&[
        "discover",
        "csv",
        text(&db),
        text(&source),
        "--apply",
        text(&backup),
    ]);
    assert!(applied.contains("Applied 1 device changes; no scans/jobs started."));
    assert!(success(&["list", text(&db)]).contains("scan=Unknown"));
    assert!(success(&["list", text(&backup)]).contains("devices=0"));
    assert!(!cli(&[
        "discover",
        "csv",
        text(&db),
        text(&source),
        "--apply",
        text(&backup)
    ])
    .status
    .success());
    assert_eq!(std::fs::read(&source).unwrap(), before);
    std::fs::write(&source, "host,password\npc,SYNTHETIC_SECRET").unwrap();
    let error = cli(&["discover", "csv", text(&db), text(&source)]);
    assert!(!error.status.success());
    assert!(!String::from_utf8_lossy(&error.stderr).contains("SYNTHETIC_SECRET"));
    assert!(!String::from_utf8_lossy(&error.stdout).contains("SYNTHETIC_SECRET"));
    std::fs::write(&source, vec![b'x'; 256 * 1024 + 1]).unwrap();
    assert!(!cli(&["discover", "manual", text(&db), text(&source)])
        .status
        .success());
}
