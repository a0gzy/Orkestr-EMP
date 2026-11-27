use crate::{store::json_rows, Result, Store};
use serde::{Deserialize, Serialize};

/// Review-only metadata: never consumed by a scheduler/executor, enabled is always false.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportReview {
    pub id: String,
    pub source_sha256: String,
    pub kind: String,
    pub name: String,
    pub enabled: bool,
    pub data: serde_json::Value,
}
impl Store {
    pub fn import_reviews(&self) -> Result<Vec<ImportReview>> {
        let version: i64 = self.db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 2 {
            return Ok(vec![]);
        }
        json_rows(&self.db, "SELECT data FROM import_review ORDER BY rowid")
    }
}
