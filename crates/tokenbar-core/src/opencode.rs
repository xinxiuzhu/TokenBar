//! OpenCode local usage from its SQLite database.
//!
//! The database is opened read-only. OpenCode has no subscription quota, so
//! this provider reports tokens and the cost OpenCode already stored.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::model::{tokens_or_none, ModelUsage, ProviderId, ProviderSnapshot};
use crate::paths::ProviderPaths;

const QUERY: &str = "SELECT time_created, model, cost, tokens_input, tokens_output, \
     tokens_reasoning, tokens_cache_read, tokens_cache_write FROM session";

pub(crate) fn fetch(paths: &ProviderPaths) -> ProviderSnapshot {
    let mut snapshot = ProviderSnapshot::empty(ProviderId::Opencode);
    if !paths.opencode_db.is_file() {
        snapshot.detail = "OpenCode database not found".to_string();
        return snapshot;
    }
    match read_db(&paths.opencode_db) {
        Ok(summary) => {
            let headline = if summary.today_billable > 0 {
                summary.today_billable
            } else {
                summary.billable
            };
            snapshot.tokens = tokens_or_none(headline);
            snapshot.cache_read_tokens = tokens_or_none(summary.cache_read);
            snapshot.cost = (summary.cost > 0.0).then_some(summary.cost);
            snapshot.models = summary.models;
            snapshot.source = "opencode.db".to_string();
            snapshot.detail = format!(
                "today {} · all {}",
                summary.today_billable, summary.billable
            );
            if snapshot.tokens.is_none() {
                snapshot.detail = "OpenCode database has no sessions".to_string();
            }
        }
        Err(err) => {
            snapshot.error = Some(err);
            snapshot.source = "missing".to_string();
        }
    }
    snapshot
}

struct Summary {
    billable: u64,
    today_billable: u64,
    cache_read: u64,
    cost: f64,
    models: Vec<ModelUsage>,
}

fn read_db(path: &Path) -> Result<Summary, String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|err| format!("open OpenCode database: {err}"))?;
    let mut statement = conn
        .prepare(QUERY)
        .map_err(|err| format!("OpenCode session table: {err}"))?;
    let today_ms = today_start_millis();
    let mut billable = 0u64;
    let mut today_billable = 0u64;
    let mut cache_read = 0u64;
    let mut cost = 0.0f64;
    let mut models: BTreeMap<String, (u64, f64)> = BTreeMap::new();
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0).unwrap_or(0),
                row.get::<_, String>(1).unwrap_or_default(),
                row.get::<_, f64>(2).unwrap_or(0.0),
                row.get::<_, i64>(3).unwrap_or(0),
                row.get::<_, i64>(4).unwrap_or(0),
                row.get::<_, i64>(5).unwrap_or(0),
                row.get::<_, i64>(6).unwrap_or(0),
                row.get::<_, i64>(7).unwrap_or(0),
            ))
        })
        .map_err(|err| format!("OpenCode session rows: {err}"))?;
    for row in rows {
        let (created, model_json, row_cost, input, output, reasoning, cached, cache_write) =
            row.map_err(|err| format!("OpenCode session row: {err}"))?;
        let row_billable =
            positive(input) + positive(output) + positive(reasoning) + positive(cache_write);
        billable = billable.saturating_add(row_billable);
        cache_read = cache_read.saturating_add(positive(cached));
        cost += row_cost.max(0.0);
        if created >= today_ms {
            today_billable = today_billable.saturating_add(row_billable);
        }
        let name = model_name(&model_json);
        let entry = models.entry(name).or_insert((0, 0.0));
        entry.0 = entry.0.saturating_add(row_billable);
        entry.1 += row_cost.max(0.0);
    }
    let mut models: Vec<ModelUsage> = models
        .into_iter()
        .map(|(name, (tokens, model_cost))| ModelUsage {
            name,
            tokens,
            cost: (model_cost > 0.0).then_some(model_cost),
        })
        .collect();
    models.sort_by_key(|model| std::cmp::Reverse(model.tokens));
    models.truncate(8);
    Ok(Summary {
        billable,
        today_billable,
        cache_read,
        cost,
        models,
    })
}

fn positive(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn model_name(raw: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return if raw.is_empty() {
            "unknown".to_string()
        } else {
            raw.to_string()
        };
    };
    let id = value.get("id").and_then(Value::as_str).unwrap_or("unknown");
    match value.get("providerID").and_then(Value::as_str) {
        Some(provider) if !provider.is_empty() => format!("{provider}/{id}"),
        _ => id.to_string(),
    }
}

fn today_start_millis() -> i64 {
    let now = chrono::Local::now();
    let midnight = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap_or(now.naive_local());
    let local = midnight.and_local_timezone(chrono::Local).unwrap();
    local.timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn sums_billable_tokens_cost_and_models() {
        let dir = std::env::temp_dir().join(format!("tokenbar-oc-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("opencode.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (
                time_created INTEGER,
                model TEXT,
                cost REAL,
                tokens_input INTEGER,
                tokens_output INTEGER,
                tokens_reasoning INTEGER,
                tokens_cache_read INTEGER,
                tokens_cache_write INTEGER
            );",
        )
        .unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO session VALUES (?1, ?2, 1.5, 100, 20, 5, 400, 10)",
            (now, r#"{"providerID":"anthropic","id":"claude"}"#),
        )
        .unwrap();
        drop(conn);
        let summary = read_db(&path).unwrap();
        assert_eq!(summary.billable, 135);
        assert_eq!(summary.cache_read, 400);
        assert!((summary.cost - 1.5).abs() < 0.001);
        assert_eq!(summary.models[0].name, "anthropic/claude");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn model_name_reads_provider_and_id() {
        assert_eq!(
            model_name(r#"{"providerID":"openai","id":"gpt"}"#),
            "openai/gpt"
        );
    }
}
