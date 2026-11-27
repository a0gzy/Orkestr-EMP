//! Explicit local-only tooling. No execution dependencies or automatic import/seeds.
use orkestr_domain::*;
use orkestr_inventory::{DiscoveryBatch, MAX_DISCOVERY_BYTES};
use orkestr_storage::*;
use std::{io::Write, path::Path};

const USAGE: &str = "Orkestr EMP — local core (not the graphical console/server)\n\
  orkestr-cli init <NEW.sqlite>\n\
  orkestr-cli list <orkestr.sqlite>\n\
  orkestr-cli review <orkestr.sqlite>\n\
  orkestr-cli preview-group <orkestr.sqlite> <collection-uuid>\n\
  orkestr-cli preview-rule <orkestr.sqlite> <expression.json>\n\
  orkestr-cli save-group <orkestr.sqlite> <collection.json> <new|expected-revision>\n\
  orkestr-cli delete-group <orkestr.sqlite> <collection-uuid> <expected-revision>\n\
  orkestr-cli import <inventory|deploy> <orkestr.sqlite> <LOCAL-BACKUP-COPY> [--apply <NEW-backup.sqlite>]\n\
  orkestr-cli discover <manual|csv> <orkestr.sqlite> <UTF8-file> [--apply <NEW-backup.sqlite>]\n\
Import defaults to DRY-RUN. Apply prints the report and creates a consistent rollback backup.\n\
No network, credentials, execution, scheduling or fixture data is loaded automatically.";

fn safe_label(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control()).take(200).collect()
}
fn uuid_id(raw: &str) -> Result<CollectionId> {
    uuid::Uuid::parse_str(raw)
        .map(CollectionId)
        .map_err(|_| Error::Invalid("invalid collection UUID"))
}
fn revision(raw: &str) -> Result<u64> {
    raw.parse()
        .map_err(|_| Error::Invalid("invalid expected revision"))
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => println!("{USAGE}"),
        [help] if help == "--help" => println!("{USAGE}"),
        [cmd, path] if cmd == "init" => {
            if Path::new(path).exists() {
                return Err(Error::Invalid("init requires a NEW database path"));
            }
            Store::open(Path::new(path))?;
            println!("Created empty Orkestr database. No fixtures or jobs.");
        }
        [cmd, path] if cmd == "list" => {
            let mut store = Store::open_read_only(Path::new(path))?;
            let snapshot = store.snapshot()?;
            println!(
                "generation={} devices={} groups={}",
                snapshot.generation,
                snapshot.devices.len(),
                snapshot.collections.len()
            );
            for row in &snapshot.devices {
                println!(
                    "{}  {}  scan={}",
                    row.device.id,
                    safe_label(&row.device.host),
                    row.scan
                        .as_ref()
                        .map(|s| s.scanned_at.to_rfc3339())
                        .unwrap_or_else(|| "Unknown".into())
                );
            }
            let context = EvaluationContext {
                now: chrono::Utc::now(),
                max_scan_age: chrono::Duration::hours(24),
                library: &[],
            };
            let paths = snapshot.paths()?;
            for c in &snapshot.collections {
                let p = snapshot.preview_collection(c.id, &context)?;
                println!(
                    "group {} rev={} members={} unknown={} {}",
                    c.id,
                    c.revision,
                    p.members.len(),
                    p.unknown.len(),
                    safe_label(&paths[&c.id])
                );
            }
            for p in store.packages()? {
                println!(
                    "package revision={} {} steps={} UNAPPROVED / NOT EXECUTABLE",
                    p.id,
                    safe_label(&p.name),
                    p.steps.len()
                );
            }
            println!(
                "review_only={} (automation disabled)",
                store.import_reviews()?.len()
            );
        }
        [cmd, path] if cmd == "review" => {
            let store = Store::open_read_only(Path::new(path))?;
            for r in store.import_reviews()? {
                println!(
                    "{} {} {} DISABLED / REVIEW ONLY",
                    r.id,
                    safe_label(&r.kind),
                    safe_label(&r.name)
                );
            }
        }
        [cmd, path, rule] if cmd == "preview-rule" || cmd == "preview-group" => {
            let mut store = Store::open_read_only(Path::new(path))?;
            let snapshot = store.snapshot()?;
            let context = EvaluationContext {
                now: chrono::Utc::now(),
                max_scan_age: chrono::Duration::hours(24),
                library: &[],
            };
            let p = if cmd == "preview-group" {
                snapshot.preview_collection(uuid_id(rule)?, &context)?
            } else {
                let e: Expression = read_json(Path::new(rule))?;
                snapshot.preview_expression(&e, &context)?
            };
            println!(
                "generation={} evaluated_at={} members={} unknown={}",
                p.generation,
                p.evaluated_at,
                p.members.len(),
                p.unknown.len()
            );
            for r in p.members {
                let ram = r
                    .scan
                    .as_ref()
                    .and_then(|s| s.installed_ram_bytes)
                    .map_or_else(|| "Unknown".into(), |b| b.to_string());
                println!(
                    "{} {} installed_ram_bytes={ram} software_rows={}",
                    r.device.id,
                    safe_label(&r.device.host),
                    r.scan.as_ref().map_or(0, |s| s.software.len())
                );
                if let Some(scan) = r.scan {
                    for app in scan.software {
                        println!(
                            "  {} | {} | {}",
                            safe_label(&app.name),
                            safe_label(&app.publisher),
                            safe_label(&app.version)
                        );
                    }
                }
            }
            for id in p.unknown {
                println!("Unknown: {id}");
            }
        }
        [cmd, path, file, expected] if cmd == "save-group" => {
            let c: Collection = read_json(Path::new(file))?;
            let expected = if expected == "new" {
                None
            } else {
                Some(revision(expected)?)
            };
            require_existing(path)?;
            let mut store = Store::open(Path::new(path))?;
            let saved = store.save_collection(&c, expected)?;
            println!("Saved {} revision={}", saved.id, saved.revision);
        }
        [cmd, path, id, expected] if cmd == "delete-group" => {
            require_existing(path)?;
            Store::open(Path::new(path))?.delete_collection(uuid_id(id)?, revision(expected)?)?;
            println!("Group removed. Computers and scans unchanged.");
        }
        [cmd, format, path, file, rest @ ..] if cmd == "discover" => {
            let apply = match rest {
                [] => None,
                [flag, backup] if flag == "--apply" => Some(backup),
                _ => return Err(Error::Invalid("invalid discovery arguments")),
            };
            use std::io::Read;
            let mut bytes = vec![];
            std::fs::File::open(file)?
                .take(MAX_DISCOVERY_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > MAX_DISCOVERY_BYTES {
                return Err(Error::Invalid("discovery input exceeds 256 KiB"));
            }
            let raw = std::str::from_utf8(&bytes)
                .map_err(|_| Error::Invalid("discovery requires UTF-8"))?;
            let batch = match format.as_str() {
                "manual" => DiscoveryBatch::manual(raw)?,
                "csv" => DiscoveryBatch::csv(raw)?,
                _ => return Err(Error::Invalid("unknown discovery format")),
            };
            let mut store = Store::open_read_only(Path::new(path))?;
            let plan = store.preview_discovery(&batch)?;
            println!(
                "generation={} can_apply={}",
                plan.generation(),
                plan.can_apply()
            );
            println!("{}", serde_json::to_string_pretty(plan.rows())?);
            std::io::stdout().flush()?;
            if !plan.can_apply() {
                return Err(Error::Invalid("discovery preview has conflicts"));
            }
            if let Some(backup) = apply {
                drop(store);
                let mut store = Store::open(Path::new(path))?;
                store.backup(Path::new(backup))?;
                println!(
                    "Applied {} device changes; no scans/jobs started.",
                    store.apply_discovery(&plan)?
                );
            } else {
                println!("DRY-RUN: nothing written; no DNS queries or scans.");
            }
        }
        [cmd, format, path, copy, rest @ ..] if cmd == "import" => {
            let apply = match rest {
                [] => None,
                [flag, backup] if flag == "--apply" => Some(backup),
                _ => return Err(Error::Invalid("invalid import arguments")),
            };
            // Even apply validates/dry-runs through a read-only target first.
            let mut store = Store::open_read_only(Path::new(path))?;
            let plan = match format.as_str() {
                "inventory" => store.preview_inventory_copy(Path::new(copy))?,
                "deploy" => store.preview_deploy_copy(Path::new(copy))?,
                _ => return Err(Error::Invalid("unknown import format")),
            };
            println!("{}", serde_json::to_string_pretty(plan.report())?);
            std::io::stdout().flush()?;
            if !plan.report().can_apply() {
                return Err(Error::Invalid("dry-run has blocking errors"));
            }
            if let Some(backup) = apply {
                drop(store);
                let mut store = Store::open(Path::new(path))?;
                store.backup(Path::new(backup))?;
                println!(
                    "{:?}; rollback backup created (no overwrite)",
                    store.apply_import(&plan)?
                );
            } else {
                println!("DRY-RUN: nothing written.");
            }
        }
        _ => return Err(Error::Invalid("invalid arguments; see --help")),
    }
    Ok(())
}
fn require_existing(path: &str) -> Result<()> {
    if !Path::new(path).is_file() {
        return Err(Error::Invalid(
            "database does not exist; use init explicitly",
        ));
    }
    Ok(())
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    use std::io::Read;
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(Error::Invalid("JSON input exceeds 1 MiB"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}
