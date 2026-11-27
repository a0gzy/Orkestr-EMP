# Third-party source attribution (M1–M3.2)

This is not a complete release dependency-license manifest. The license for new
Orkestr EMP code is still to be selected by the owner; Cargo packages are unpublished.

The original project paths below identify historical study copies. Those optional
copies and their tools/build output are archived outside this repository; they are
not build dependencies. The original MIT notices remain in `licenses/`.

Only the following original MIT Rust source patterns/models have been adapted:

| Origin | Source | Orkestr adaptation |
|---|---|---|
| Open Inventory contributors, 2026 | `../pdqinventoryconsole-2/app/src/model.rs`, `valid_host`, collection concepts | `crates/domain/src/model.rs`, stricter host normalization; collections reimplemented with typed predicates and Unknown |
| Open Inventory contributors, 2026 | `../pdqinventoryconsole-2/app/src/storage.rs`, transaction/backup patterns | `crates/storage/src/store.rs`; new schema and immutable scans |
| Open Deploy contributors, 2026 | `../pdqdeployconsole/app/src/model.rs`, `KINDS` and step/package field names | `crates/domain/src/model.rs`, `crates/storage/src/import.rs`; review-only import, no runtime/defaults |

The full original notices are retained in [licenses/OpenInventory-MIT.txt](licenses/OpenInventory-MIT.txt)
and [licenses/OpenDeploy-MIT.txt](licenses/OpenDeploy-MIT.txt).
No proprietary binaries, recovered constants/mechanics, runtime, vendor catalog,
keys, old executables, toolchain or build output are incorporated in this workspace.
Transitive Cargo dependency licenses must be audited before distribution (M6).

The native M1 console uses egui/eframe 0.31.1. Segoe UI/Consolas, if available, are
read from the user's Windows installation at runtime; no Windows font files have
been copied into this project or its assets. Eframe's bundled fallback fonts and
other Cargo dependencies retain their upstream licenses (see Cargo.lock); a full
release license/security manifest is still required before distribution.

M2 server/client/security modules were implemented in this workspace, not copied from
the old server or DPAPI implementation. Added Cargo dependencies include Axum/Tokio,
Rustls/reqwest, rcgen, getrandom, zeroize, windows-sys and windows-service; their exact
resolved versions are in Cargo.lock. This list is informational, not a completed
dependency/license or cryptographic security review. Do not distribute test certificates,
private keys, DPAPI blobs, tokens, protected databases or local test output as release assets.

M3.1 inventory/discovery code was implemented in this workspace. The original Inventory
scan/discovery scripts were read to preserve the F21 section inventory; they were not copied
or executed as network adapters. CSV decoding uses the Cargo `csv`/`csv-core` dependencies;
exact versions/checksums are recorded in Cargo.lock. Their addition is not a completed
release dependency/license/security audit; the full M6 manifest remains required.

M3.2 console/API/session/receipt wiring adds internal workspace dependencies only;
no new third-party runtime or source project code was copied for this increment.
