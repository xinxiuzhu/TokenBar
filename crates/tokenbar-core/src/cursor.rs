//! Cursor session + dashboard usage.
//!
//! The IDE stores `cursorAuth/accessToken` in `state.vscdb`. A JWT `sub` (or
//! `cursorAuth/cachedScopedProfile.authId` for opaque tokens) is joined with
//! that token to form the `WorkosCursorSessionToken` cookie. Quota percent
//! comes from `GET /api/usage-summary`. Token totals come from
//! `POST /api/dashboard/get-filtered-usage-events`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::format::reset_string;
use crate::http::{http_failure, sanitize, Http};
use crate::jsonl::json_u64;
use crate::model::{clip_detail, ProviderId, ProviderSnapshot, UsageWindow};
use crate::paths::ProviderPaths;
use crate::secrets::{SecretKind, SecretStore};

const SUMMARY_URL: &str = "https://cursor.com/api/usage-summary";
const EVENTS_URL: &str = "https://cursor.com/api/dashboard/get-filtered-usage-events";
const PAGE_SIZE: u32 = 100;
const MAX_PAGES: u32 = 5;

pub(crate) fn fetch(
    http: &dyn Http,
    secrets: &dyn SecretStore,
    paths: &ProviderPaths,
) -> ProviderSnapshot {
    let mut snapshot = ProviderSnapshot::empty(ProviderId::Cursor);
    let resolved = match resolve_session(secrets, paths) {
        Ok(session) => session,
        Err(err) => {
            snapshot.error = Some(err);
            snapshot.detail = clip_detail([format!("expected {}", paths.cursor_db.display())]);
            return snapshot;
        }
    };
    snapshot.detail = clip_detail([format!("session from {}", resolved.origin)]);

    let headers_owned = cursor_header_values(&resolved.cookie);
    let headers = header_refs(&headers_owned);

    match http.get(SUMMARY_URL, &headers) {
        Ok(response) => match parse_json("cursor usage-summary", response) {
            Ok(summary) => apply_summary(&mut snapshot, &summary),
            Err(err) => snapshot.error = Some(err),
        },
        Err(err) => snapshot.error = Some(format!("cursor usage-summary: {}", sanitize(&err))),
    }

    match fetch_events(http, &headers) {
        Ok(events) => {
            if events.tokens > 0 {
                snapshot.tokens = Some(events.tokens);
            }
            if events.cache_read > 0 {
                snapshot.cache_read_tokens = Some(events.cache_read);
            }
            let mut parts = vec![snapshot.detail.clone()];
            parts.push(format!(
                "{} events / {} tokens in 30d{}",
                events.count,
                events.tokens,
                if events.truncated { " (partial)" } else { "" }
            ));
            snapshot.detail = clip_detail(parts);
            if events.count > 0 || snapshot.used_percent.is_some() {
                snapshot.error = None;
            }
            push_source(&mut snapshot, "events");
        }
        Err(err) => {
            if snapshot.used_percent.is_none() && snapshot.tokens.is_none() {
                snapshot.error = Some(err);
            } else {
                snapshot.detail = clip_detail([snapshot.detail.clone(), err]);
            }
        }
    }

    if (snapshot.used_percent.is_some() || snapshot.tokens.is_some())
        && snapshot.source == "missing"
    {
        snapshot.source = "usage-summary".to_string();
    }
    snapshot
}

struct ResolvedSession {
    cookie: String,
    origin: &'static str,
}

fn resolve_session(
    secrets: &dyn SecretStore,
    paths: &ProviderPaths,
) -> Result<ResolvedSession, String> {
    if let Some(pasted) = secrets.get(SecretKind::CursorSession) {
        return Ok(ResolvedSession {
            cookie: cursor_session_cookie(&pasted, None),
            origin: "Settings",
        });
    }
    if let Ok(env) = std::env::var("CURSOR_SESSION_TOKEN") {
        if !env.trim().is_empty() {
            return Ok(ResolvedSession {
                cookie: cursor_session_cookie(env.trim(), None),
                origin: "CURSOR_SESSION_TOKEN",
            });
        }
    }
    if paths.cursor_db.exists() {
        let auth = read_cursor_auth(&paths.cursor_db)?;
        if !auth.access_token.is_empty() {
            return Ok(ResolvedSession {
                cookie: cursor_session_cookie(&auth.access_token, auth.auth_id.as_deref()),
                origin: "state.vscdb",
            });
        }
    }
    Err("no Cursor session; paste one in Settings or sign in to the Cursor IDE".to_string())
}

pub(crate) fn cursor_session_cookie(raw: &str, auth_id: Option<&str>) -> String {
    let trimmed = raw.trim().trim_matches('"').trim();
    let trimmed = trimmed
        .strip_prefix("WorkosCursorSessionToken=")
        .unwrap_or(trimmed)
        .trim();
    if trimmed.contains("::") {
        return trimmed.to_string();
    }
    if let Some(sub) = jwt_sub(trimmed) {
        return format!("{sub}::{trimmed}");
    }
    if let Some(auth_id) = auth_id.map(str::trim).filter(|value| !value.is_empty()) {
        return format!("{auth_id}::{trimmed}");
    }
    trimmed.to_string()
}

pub(crate) fn jwt_sub(token: &str) -> Option<String> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let signature = parts.next()?;
    if parts.next().is_some() || signature.is_empty() {
        return None;
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("sub")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sub| !sub.is_empty())
        .map(str::to_string)
}

struct CursorAuth {
    access_token: String,
    auth_id: Option<String>,
}

fn read_cursor_auth(path: &Path) -> Result<CursorAuth, String> {
    let (cleanup, conn) = open_readonly(path)?;
    let auth = read_auth_conn(&conn);
    drop(conn);
    if let Some(temp) = cleanup {
        let _ = fs::remove_file(&temp);
        for suffix in ["-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{suffix}", temp.display()));
        }
    }
    auth
}

fn read_auth_conn(conn: &Connection) -> Result<CursorAuth, String> {
    let mut statement = conn
        .prepare(
            "SELECT key, value FROM ItemTable WHERE key IN (
                'cursorAuth/accessToken',
                'cursorAuth/cachedScopedProfile',
                'cursorAuth/cachedEmail'
            )",
        )
        .map_err(|err| format!("state.vscdb: {err}"))?;
    let mut token = String::new();
    let mut profile = String::new();
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row_text(row)?)))
        .map_err(|err| format!("state.vscdb: {err}"))?;
    for row in rows {
        let (key, value) = row.map_err(|err| format!("state.vscdb: {err}"))?;
        let value = unwrap_json_string(&value);
        match key.as_str() {
            "cursorAuth/accessToken" => token = value,
            "cursorAuth/cachedScopedProfile" => profile = value,
            _ => {}
        }
    }
    if token.is_empty() {
        return Err("no cursorAuth/accessToken in state.vscdb".to_string());
    }
    Ok(CursorAuth {
        access_token: token,
        auth_id: auth_id_from_profile(&profile),
    })
}

fn row_text(row: &rusqlite::Row<'_>) -> rusqlite::Result<String> {
    if let Ok(text) = row.get::<_, String>(1) {
        return Ok(text);
    }
    let bytes: Vec<u8> = row.get(1)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn unwrap_json_string(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('\0').trim();
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::String(text)) => text,
        _ => trimmed.to_string(),
    }
}

fn auth_id_from_profile(raw: &str) -> Option<String> {
    if raw.trim().is_empty() {
        return None;
    }
    let mut value: Value = serde_json::from_str(raw.trim()).ok()?;
    if let Value::String(inner) = &value {
        value = serde_json::from_str(inner).ok()?;
    }
    ["authId", "auth_id", "sub"].iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

fn open_readonly(path: &Path) -> Result<(Option<PathBuf>, Connection), String> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY;
    if let Ok(conn) = Connection::open_with_flags(path, flags) {
        return Ok((None, conn));
    }
    let temp = std::env::temp_dir().join(format!(
        "tokenbar-cursor-{}-{}.vscdb",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::copy(path, &temp).map_err(|err| format!("copy {}: {err}", path.display()))?;
    for suffix in ["-wal", "-shm"] {
        let source = PathBuf::from(format!("{}{suffix}", path.display()));
        if source.exists() {
            let dest = PathBuf::from(format!("{}{suffix}", temp.display()));
            let _ = fs::copy(source, dest);
        }
    }
    match Connection::open_with_flags(&temp, flags) {
        Ok(conn) => Ok((Some(temp), conn)),
        Err(err) => {
            let _ = fs::remove_file(&temp);
            Err(format!("open {}: {err}", path.display()))
        }
    }
}

fn cursor_header_values(cookie: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Cookie", format!("WorkosCursorSessionToken={cookie}")),
        ("Origin", "https://cursor.com".to_string()),
        ("Accept", "application/json".to_string()),
        ("Content-Type", "application/json".to_string()),
        ("User-Agent", "TokenBar/0.1".to_string()),
    ]
}

fn header_refs<'a>(headers: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect()
}

fn apply_summary(snapshot: &mut ProviderSnapshot, summary: &Value) {
    if let Some(plan) = summary
        .get("membershipType")
        .or_else(|| summary.get("plan"))
        .and_then(Value::as_str)
    {
        snapshot.plan = Some(plan.to_string());
    }
    let reset = summary.get("billingCycleEnd").and_then(reset_string);
    let plan_body = summary
        .pointer("/individualUsage/plan")
        .or_else(|| summary.get("plan"));
    if let Some(plan_body) = plan_body {
        if let Some(percent) = plan_body
            .get("totalPercentUsed")
            .and_then(Value::as_f64)
            .or_else(|| used_over_limit(plan_body))
        {
            push_plan_window(snapshot, "套餐", percent, reset.clone());
        }
        if let Some(percent) = plan_body.get("apiPercentUsed").and_then(Value::as_f64) {
            push_plan_window(snapshot, "API", percent, reset.clone());
        }
        if let Some(percent) = plan_body.get("autoPercentUsed").and_then(Value::as_f64) {
            push_plan_window(snapshot, "Auto", percent, reset.clone());
        }
    }
    if snapshot.windows.is_empty() {
        if let Some(percent) = summary_percent(summary) {
            push_plan_window(snapshot, "套餐", percent, reset);
        }
    }
    if let Some(first) = snapshot.windows.first() {
        snapshot.used_percent = first.used_percent;
        snapshot.resets_at = first.resets_at.clone();
        push_source(snapshot, "usage-summary");
        snapshot.error = None;
    }
}

fn used_over_limit(plan: &Value) -> Option<f64> {
    let used = plan.get("used").and_then(Value::as_f64)?;
    let limit = plan.get("limit").and_then(Value::as_f64)?;
    if limit > 0.0 {
        Some(used / limit * 100.0)
    } else {
        None
    }
}

fn push_plan_window(
    snapshot: &mut ProviderSnapshot,
    name: &str,
    percent: f64,
    reset: Option<String>,
) {
    if snapshot.windows.iter().any(|window| {
        window.name == name
            || window
                .used_percent
                .is_some_and(|used| (used - percent).abs() < 1.0 && window.name != "套餐")
    }) {
        return;
    }
    if name != "套餐"
        && snapshot.windows.iter().any(|window| {
            window.name == "套餐"
                && window
                    .used_percent
                    .is_some_and(|used| (used - percent).abs() < 1.0)
        })
    {
        return;
    }
    snapshot.windows.push(UsageWindow {
        name: name.to_string(),
        used_percent: Some(percent),
        resets_at: reset,
    });
}

fn summary_percent(summary: &Value) -> Option<f64> {
    let plan = summary
        .pointer("/individualUsage/plan")
        .or_else(|| summary.get("plan"))?;
    if let Some(percent) = plan.get("totalPercentUsed").and_then(Value::as_f64) {
        return Some(percent);
    }
    let api = plan.get("apiPercentUsed").and_then(Value::as_f64);
    let auto = plan.get("autoPercentUsed").and_then(Value::as_f64);
    if api.is_some() || auto.is_some() {
        return Some(api.unwrap_or(0.0).max(auto.unwrap_or(0.0)));
    }
    let used = plan.get("used").and_then(Value::as_f64)?;
    let limit = plan.get("limit").and_then(Value::as_f64)?;
    if limit > 0.0 {
        Some(used / limit * 100.0)
    } else {
        None
    }
}

struct EventPage {
    tokens: u64,
    cache_read: u64,
    count: u64,
    truncated: bool,
}

fn fetch_events(http: &dyn Http, headers: &[(&str, &str)]) -> Result<EventPage, String> {
    let (start, end) = thirty_day_millis();
    let mut page = 1u32;
    let mut tokens = 0u64;
    let mut cache_read = 0u64;
    let mut count = 0u64;
    let mut total_count: Option<u64> = None;
    loop {
        let body = serde_json::json!({
            "startDate": start,
            "endDate": end,
            "page": page,
            "pageSize": PAGE_SIZE,
        })
        .to_string();
        let response = http
            .post(EVENTS_URL, headers, &body)
            .map_err(|err| format!("cursor events: {}", sanitize(&err)))?;
        let value = parse_json("cursor events", response)?;
        if value.is_null() || (value.as_object().is_some_and(|map| map.is_empty())) {
            break;
        }
        if total_count.is_none() {
            total_count = value.get("totalUsageEventsCount").and_then(Value::as_u64);
        }
        let events = value
            .get("usageEventsDisplay")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if events.is_empty() {
            break;
        }
        for event in &events {
            let (billable, cached) = event_tokens(event);
            tokens = tokens.saturating_add(billable);
            cache_read = cache_read.saturating_add(cached);
            count = count.saturating_add(1);
        }
        let exhausted = events.len() < PAGE_SIZE as usize
            || total_count.is_some_and(|total| count >= total)
            || page >= MAX_PAGES;
        if exhausted {
            let truncated = total_count.is_some_and(|total| count < total)
                || (page >= MAX_PAGES
                    && events.len() == PAGE_SIZE as usize
                    && total_count.is_none());
            return Ok(EventPage {
                tokens,
                cache_read,
                count,
                truncated,
            });
        }
        page += 1;
    }
    Ok(EventPage {
        tokens,
        cache_read,
        count,
        truncated: false,
    })
}

fn event_tokens(event: &Value) -> (u64, u64) {
    let usage = event
        .get("tokenUsage")
        .or_else(|| event.get("token_usage"))
        .unwrap_or(event);
    let billable = json_u64(usage, &["inputTokens", "input_tokens"])
        .saturating_add(json_u64(usage, &["outputTokens", "output_tokens"]))
        .saturating_add(json_u64(usage, &["cacheWriteTokens", "cache_write_tokens"]));
    let cache_read = json_u64(usage, &["cacheReadTokens", "cache_read_tokens"]);
    (billable, cache_read)
}

fn thirty_day_millis() -> (String, String) {
    let end = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let start = end.saturating_sub(30 * 24 * 60 * 60 * 1000);
    (start.to_string(), end.to_string())
}

fn parse_json(context: &str, response: crate::http::HttpResponse) -> Result<Value, String> {
    if !(200..300).contains(&response.status) {
        return Err(http_failure(context, response.status, &response.body));
    }
    if response.body.trim().is_empty() {
        return Ok(Value::Null);
    }
    let value: Value = serde_json::from_str(&response.body)
        .map_err(|err| format!("{context}: invalid JSON ({err})"))?;
    if let Some(err) = value.get("error").and_then(Value::as_str) {
        if !err.is_empty() {
            return Err(format!("{context}: {err}"));
        }
    }
    Ok(value)
}

fn push_source(snapshot: &mut ProviderSnapshot, source: &str) {
    if snapshot.source == "missing" || snapshot.source.is_empty() {
        snapshot.source = source.to_string();
    } else if !snapshot.source.split('+').any(|part| part == source) {
        snapshot.source = format!("{}+{source}", snapshot.source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpResponse, ScriptedHttp, ScriptedRoute};
    use crate::secrets::MemoryStore;
    use std::path::PathBuf;
    use std::sync::Mutex;

    fn test_jwt(sub: &str) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(format!(r#"{{"sub":"{sub}"}}"#));
        format!("{header}.{payload}.sig")
    }

    #[test]
    fn jwt_cookie_uses_sub() {
        let token = test_jwt("auth0|user_123");
        assert_eq!(jwt_sub(&token).as_deref(), Some("auth0|user_123"));
        assert_eq!(
            cursor_session_cookie(&token, None),
            format!("auth0|user_123::{token}")
        );
        assert_eq!(
            cursor_session_cookie("user_9::already", None),
            "user_9::already"
        );
        assert_eq!(
            cursor_session_cookie("opaque-token", Some("auth0|user_9")),
            "auth0|user_9::opaque-token"
        );
    }

    #[test]
    fn reads_token_and_profile_from_vscdb() {
        let dir = std::env::temp_dir().join(format!("tokenbar-vscdb-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("state.vscdb");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT PRIMARY KEY, value TEXT);
             DELETE FROM ItemTable;",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES ('cursorAuth/accessToken', ?1)",
            ["curs_opaque"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES ('cursorAuth/cachedScopedProfile', ?1)",
            [r#"{"authId":"auth0|user_9","email":"a@b.c"}"#],
        )
        .unwrap();
        drop(conn);
        let auth = read_cursor_auth(&path).unwrap();
        assert_eq!(auth.access_token, "curs_opaque");
        assert_eq!(auth.auth_id.as_deref(), Some("auth0|user_9"));
        assert_eq!(
            cursor_session_cookie(&auth.access_token, auth.auth_id.as_deref()),
            "auth0|user_9::curs_opaque"
        );
        let _ = fs::remove_dir_all(dir);
    }

    type HeaderList = Vec<(String, String)>;

    struct RecordingHttp {
        inner: ScriptedHttp,
        hits: Mutex<Vec<(String, HeaderList)>>,
    }

    impl Http for RecordingHttp {
        fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, String> {
            self.record("GET", headers);
            self.inner.get(url, headers)
        }
        fn post(
            &self,
            url: &str,
            headers: &[(&str, &str)],
            body: &str,
        ) -> Result<HttpResponse, String> {
            self.record("POST", headers);
            assert!(body.contains("\"page\""));
            self.inner.post(url, headers, body)
        }
    }

    impl RecordingHttp {
        fn record(&self, method: &str, headers: &[(&str, &str)]) {
            self.hits.lock().unwrap().push((
                method.to_string(),
                headers
                    .iter()
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect(),
            ));
        }
    }

    #[test]
    fn summary_percent_and_events_tokens() {
        let token = test_jwt("auth0|user_123");
        let http = RecordingHttp {
            inner: ScriptedHttp {
                routes: vec![
                    ScriptedRoute {
                        method: "GET",
                        url_contains: "/api/usage-summary",
                        status: 200,
                        body: r#"{"membershipType":"pro","individualUsage":{"plan":{"totalPercentUsed":42,"used":42,"limit":100}}}"#.into(),
                    },
                    ScriptedRoute {
                        method: "POST",
                        url_contains: "get-filtered-usage-events",
                        status: 200,
                        body: r#"{"totalUsageEventsCount":1,"usageEventsDisplay":[{"tokenUsage":{"inputTokens":100,"outputTokens":20,"cacheReadTokens":5,"cacheWriteTokens":0}}]}"#.into(),
                    },
                ],
            },
            hits: Mutex::new(Vec::new()),
        };
        let secrets = MemoryStore::new();
        secrets.insert(SecretKind::CursorSession, token.clone());
        let paths = ProviderPaths::from_roots(crate::paths::PathRoots {
            home: PathBuf::from("/nope"),
            config: PathBuf::from("/nope"),
            claude_config_dir: None,
            codex_home: None,
        });
        let snapshot = fetch(&http, &secrets, &paths);
        assert_eq!(snapshot.used_percent, Some(42.0));
        assert_eq!(snapshot.tokens, Some(120));
        assert_eq!(snapshot.cache_read_tokens, Some(5));
        assert_eq!(snapshot.plan.as_deref(), Some("pro"));
        assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
        let hits = http.hits.lock().unwrap();
        assert!(hits.iter().any(|(_, headers)| {
            headers.iter().any(|(name, value)| {
                name == "Cookie" && value.starts_with("WorkosCursorSessionToken=auth0|user_123::")
            })
        }));
        assert!(hits.iter().any(|(_, headers)| {
            headers
                .iter()
                .any(|(name, value)| name == "Origin" && value == "https://cursor.com")
        }));
    }

    #[test]
    fn summary_used_over_limit_when_percent_missing() {
        let summary = serde_json::json!({
            "individualUsage": {"plan": {"used": 21, "limit": 50}}
        });
        assert_eq!(summary_percent(&summary), Some(42.0));
    }
}
