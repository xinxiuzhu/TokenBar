//! OpenAI API-key usage and Codex subscription usage.
//!
//! Codex login lives in `~/.codex/auth.json` (`tokens.access_token`, optional
//! `account_id`). Subscription quota is `GET https://chatgpt.com/backend-api/wham/usage`.
//! A pasted or stored `sk-` key optionally queries
//! `GET /v1/organization/usage/completions`. Codex JSONL rollouts contribute
//! a token total. The tray shows the wham percent when it exists, otherwise
//! the compact token count.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::format::reset_string;
use crate::http::{http_failure, sanitize, Http, HttpResponse};
use crate::jsonl::{json_u64, sum_codex_trees};
use crate::model::{clip_detail, tokens_or_none, ProviderId, ProviderSnapshot, UsageWindow};
use crate::paths::ProviderPaths;
use crate::secrets::{SecretKind, SecretStore};

const WHAM_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const ORG_URL: &str = "https://api.openai.com/v1/organization/usage/completions";

pub(crate) fn fetch(
    http: &dyn Http,
    secrets: &dyn SecretStore,
    paths: &ProviderPaths,
) -> ProviderSnapshot {
    let mut snapshot = ProviderSnapshot::empty(ProviderId::Openai);
    let auth = read_auth(paths);
    let pasted_key = secrets.get(SecretKind::OpenaiKey);
    let api_key = pasted_key
        .clone()
        .filter(|key| key.starts_with("sk-"))
        .or(auth.api_key.clone());
    let key_origin = if pasted_key
        .as_ref()
        .is_some_and(|key| key.starts_with("sk-"))
    {
        Some("Settings")
    } else if auth.api_key.is_some() {
        Some("auth.json")
    } else {
        None
    };

    let mut notes = Vec::new();
    let mut sources = Vec::new();
    let mut wham_error = None;
    let mut org_error = None;

    if let Some(access) = auth.access_token.clone() {
        match fetch_wham(http, &access, auth.account_id.as_deref()) {
            Ok(wham) => {
                snapshot.used_percent = wham.used_percent;
                snapshot.resets_at = wham.resets_at.clone();
                snapshot.windows = wham.windows;
                if snapshot.plan.is_none() {
                    snapshot.plan = wham.plan;
                }
                if snapshot.balance.is_none() {
                    snapshot.balance = wham.balance;
                }
                sources.push("wham");
                notes.push("codex oauth from auth.json".to_string());
            }
            Err(err) => {
                wham_error = Some(err);
            }
        }
    }

    if let Some(key) = api_key.as_deref() {
        match fetch_org(http, key) {
            Ok(tokens) => {
                if tokens > 0 {
                    snapshot.tokens = Some(tokens);
                    sources.push("org");
                    if let Some(origin) = key_origin {
                        notes.push(format!("org usage from {origin}"));
                    }
                }
            }
            Err(err) => org_error = Some(err),
        }
    }

    let local = sum_codex_trees(&paths.codex_session_roots);
    if local > 0 {
        snapshot.tokens = Some(snapshot.tokens.unwrap_or(0).saturating_add(local));
        sources.push("jsonl");
        notes.push(format!("jsonl {local} tokens"));
    }

    if snapshot.windows.is_empty() {
        if let Some(from_log) = codex_log_limits(&paths.codex_session_roots) {
            snapshot.used_percent = from_log.used_percent;
            snapshot.resets_at = from_log.resets_at;
            snapshot.windows = from_log.windows;
            if snapshot.plan.is_none() {
                snapshot.plan = from_log.plan;
            }
            if snapshot.balance.is_none() {
                snapshot.balance = from_log.balance;
            }
            sources.push("jsonl-limits");
        }
    }

    if sources.is_empty() {
        snapshot.error = Some(
            wham_error
                .or(org_error)
                .unwrap_or_else(|| "no Codex auth.json, OpenAI key, or session logs".to_string()),
        );
        snapshot.detail = clip_detail([format!("expected {}", paths.codex_auth.display())]);
        snapshot.tokens = tokens_or_none(snapshot.tokens.unwrap_or(0));
        return snapshot;
    }

    snapshot.source = sources.join("+");
    if let Some(err) = wham_error {
        notes.push(err);
    }
    if let Some(err) = org_error {
        notes.push(err);
    }
    snapshot.detail = clip_detail(notes);
    snapshot.error = None;
    if snapshot.tokens == Some(0) {
        snapshot.tokens = None;
    }
    snapshot
}

struct CodexAuth {
    access_token: Option<String>,
    account_id: Option<String>,
    api_key: Option<String>,
}

fn read_auth(paths: &ProviderPaths) -> CodexAuth {
    let Ok(raw) = fs::read_to_string(&paths.codex_auth) else {
        return CodexAuth {
            access_token: None,
            account_id: None,
            api_key: None,
        };
    };
    parse_codex_auth(&raw)
}

fn parse_codex_auth(raw: &str) -> CodexAuth {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return CodexAuth {
            access_token: None,
            account_id: None,
            api_key: None,
        };
    };
    let access_token = string_at(&value, &["/tokens/access_token", "/access_token"]);
    let account_id = string_at(&value, &["/tokens/account_id", "/account_id"]);
    let api_key = string_at(&value, &["/OPENAI_API_KEY", "/openai_api_key"])
        .filter(|key| key.starts_with("sk-"));
    CodexAuth {
        access_token,
        account_id,
        api_key,
    }
}

fn string_at(value: &Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

struct WhamUsage {
    used_percent: Option<f64>,
    resets_at: Option<String>,
    plan: Option<String>,
    windows: Vec<UsageWindow>,
    balance: Option<String>,
}

fn fetch_wham(
    http: &dyn Http,
    access: &str,
    account_id: Option<&str>,
) -> Result<WhamUsage, String> {
    let auth = format!("Bearer {access}");
    let account_owned = account_id.unwrap_or("").to_string();
    let mut headers = vec![
        ("Authorization", auth.as_str()),
        ("Accept", "application/json"),
        ("User-Agent", "codex-cli"),
    ];
    if account_id.is_some() {
        headers.push(("ChatGPT-Account-Id", account_owned.as_str()));
    }
    let response = http
        .get(WHAM_URL, &headers)
        .map_err(|err| format!("codex wham: {}", sanitize(&err)))?;
    parse_wham(response)
}

fn parse_wham(response: HttpResponse) -> Result<WhamUsage, String> {
    if !(200..300).contains(&response.status) {
        return Err(http_failure("codex wham", response.status, &response.body));
    }
    let value: Value = serde_json::from_str(&response.body)
        .map_err(|err| format!("codex wham: invalid JSON ({err})"))?;
    if let Some(err) = value.get("error").and_then(|err| {
        err.as_str()
            .or_else(|| err.get("message").and_then(Value::as_str))
    }) {
        return Err(format!("codex wham: {err}"));
    }
    windows_from_payload(&value, &response.headers)
        .ok_or_else(|| "codex wham response had no rate-limit windows".to_string())
}

fn windows_from_payload(value: &Value, headers: &[(String, String)]) -> Option<WhamUsage> {
    let plan = value
        .get("plan_type")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            find_rate_limits(value)
                .and_then(|rate| rate.get("plan_type"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let rate = find_rate_limits(value);
    let mut windows = Vec::new();
    if let Some(rate) = rate {
        for (key, fallback) in [("primary", "five_hour"), ("secondary", "seven_day")] {
            if let Some(window) = modern_window(rate.get(key), fallback) {
                windows.push(window);
            }
        }
        if windows.is_empty() {
            if let Some(window) = rate_window(rate.get("primary_window"), "five_hour") {
                windows.push(window);
            }
            if let Some(window) = rate_window(rate.get("secondary_window"), "seven_day") {
                windows.push(window);
            }
        }
    }
    if windows.is_empty() {
        if let Some(percent) = header_percent(headers, "x-codex-primary-used-percent") {
            windows.push(UsageWindow {
                name: "five_hour".to_string(),
                used_percent: Some(percent),
                resets_at: None,
            });
        }
    }
    if windows.is_empty() {
        return None;
    }
    let headline = windows.iter().find_map(|window| window.used_percent);
    let resets_at = windows.iter().find_map(|window| window.resets_at.clone());
    let balance = rate
        .and_then(credits_balance)
        .or_else(|| credits_balance(value));
    Some(WhamUsage {
        used_percent: headline,
        resets_at,
        plan,
        windows,
        balance,
    })
}

fn find_rate_limits(value: &Value) -> Option<&Value> {
    if has_limit_windows(value) {
        return Some(value);
    }
    let object = value.as_object()?;
    for key in ["rate_limits", "rate_limit", "usage"] {
        if let Some(child) = object.get(key) {
            if has_limit_windows(child) {
                return Some(child);
            }
        }
    }
    for (_, child) in object {
        if has_limit_windows(child) {
            return Some(child);
        }
        let Some(nested) = child.as_object() else {
            continue;
        };
        for key in ["rate_limits", "rate_limit", "usage"] {
            if let Some(inner) = nested.get(key) {
                if has_limit_windows(inner) {
                    return Some(inner);
                }
            }
        }
    }
    None
}

fn has_limit_windows(value: &Value) -> bool {
    value.get("primary").is_some()
        || value.get("secondary").is_some()
        || value.get("primary_window").is_some()
        || value.get("secondary_window").is_some()
}

fn modern_window(value: Option<&Value>, fallback: &str) -> Option<UsageWindow> {
    let value = value?;
    let minutes = value.get("window_minutes").and_then(Value::as_u64);
    let seconds = value.get("limit_window_seconds").and_then(Value::as_u64);
    let name = canonical_window_name(minutes, seconds, fallback);
    let used = json_f64(value.get("used_percent")).or_else(|| json_f64(value.get("usedPercent")));
    let resets_at = value
        .get("reset_at")
        .or_else(|| value.get("resets_at"))
        .and_then(reset_string)
        .or_else(|| resets_in_seconds(value));
    if used.is_none() && resets_at.is_none() {
        return None;
    }
    Some(UsageWindow {
        name,
        used_percent: used,
        resets_at,
    })
}

fn canonical_window_name(minutes: Option<u64>, seconds: Option<u64>, fallback: &str) -> String {
    let minutes = minutes.or_else(|| seconds.map(|seconds| seconds / 60));
    match minutes {
        Some(minutes) if (1..600).contains(&minutes) => "five_hour".to_string(),
        Some(minutes) if (6 * 1440..=8 * 1440).contains(&minutes) => "seven_day".to_string(),
        Some(minutes) if minutes >= 1440 => format!("{} 天", minutes / 1440),
        Some(minutes) if minutes >= 60 => format!("{} 小时", minutes / 60),
        _ => fallback.to_string(),
    }
}

fn json_f64(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        .filter(|number| number.is_finite())
}

fn resets_in_seconds(value: &Value) -> Option<String> {
    let seconds = value.get("resets_in_seconds")?.as_i64()?;
    let when = chrono::Utc::now() + chrono::Duration::seconds(seconds);
    Some(when.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

fn credits_balance(value: &Value) -> Option<String> {
    let credits = value.get("credits")?;
    if credits.get("unlimited").and_then(Value::as_bool) == Some(true) {
        return Some("无限".to_string());
    }
    let raw = credits.get("balance")?;
    let text = raw
        .as_str()
        .map(str::to_string)
        .or_else(|| raw.as_f64().map(|number| format!("{number:.2}")))?;
    if text.is_empty() {
        None
    } else if text.starts_with('$') {
        Some(text)
    } else {
        Some(format!("${text}"))
    }
}

fn rate_window(value: Option<&Value>, fallback: &str) -> Option<UsageWindow> {
    let value = value?;
    let seconds = value.get("limit_window_seconds").and_then(Value::as_u64);
    let name = canonical_window_name(None, seconds, fallback);
    let used = json_f64(value.get("used_percent")).or_else(|| json_f64(value.get("usedPercent")));
    let resets_at = value
        .get("reset_at")
        .or_else(|| value.get("resets_at"))
        .and_then(reset_string);
    if used.is_none() && resets_at.is_none() {
        return None;
    }
    Some(UsageWindow {
        name: name.to_string(),
        used_percent: used,
        resets_at,
    })
}

fn header_percent(headers: &[(String, String)], name: &str) -> Option<f64> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.trim().parse().ok())
}

fn fetch_org(http: &dyn Http, key: &str) -> Result<u64, String> {
    let start = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().saturating_sub(30 * 24 * 60 * 60))
        .unwrap_or(0);
    let url = format!("{ORG_URL}?start_time={start}&bucket_width=1d&limit=31");
    let auth = format!("Bearer {key}");
    let headers = [
        ("Authorization", auth.as_str()),
        ("Accept", "application/json"),
    ];
    let response = http
        .get(&url, &headers)
        .map_err(|err| format!("openai org usage: {}", sanitize(&err)))?;
    if !(200..300).contains(&response.status) {
        return Err(http_failure(
            "openai org usage",
            response.status,
            &response.body,
        ));
    }
    let value: Value = serde_json::from_str(&response.body)
        .map_err(|err| format!("openai org usage: invalid JSON ({err})"))?;
    Ok(sum_org_usage(&value))
}

fn codex_log_limits(roots: &[std::path::PathBuf]) -> Option<WhamUsage> {
    let path = newest_jsonl(roots)?;
    let text = read_tail(&path, 256 * 1024).ok()?;
    for line in text.lines().rev() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(parsed) = windows_from_payload(&value, &[]) {
            return Some(parsed);
        }
    }
    None
}

fn newest_jsonl(roots: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    let mut best: Option<(SystemTime, std::path::PathBuf)> = None;
    let mut seen = 0usize;
    for root in roots {
        visit_jsonl(root, 0, &mut seen, &mut best);
    }
    best.map(|(_, path)| path)
}

fn visit_jsonl(
    dir: &std::path::Path,
    depth: usize,
    seen: &mut usize,
    best: &mut Option<(SystemTime, std::path::PathBuf)>,
) {
    if depth > 8 || *seen > 2_000 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if *seen > 2_000 {
            return;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            visit_jsonl(&path, depth + 1, seen, best);
            continue;
        }
        if !path.extension().is_some_and(|ext| ext == "jsonl") {
            continue;
        }
        *seen += 1;
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(when, _)| modified > *when) {
            *best = Some((modified, path));
        }
    }
}

fn read_tail(path: &std::path::Path, max: usize) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len > max as u64 {
        file.seek(SeekFrom::Start(len - max as u64))?;
    }
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    if len > max as u64 {
        if let Some(index) = text.find('\n') {
            text.drain(..=index);
        }
    }
    Ok(text)
}

pub(crate) fn sum_org_usage(value: &Value) -> u64 {
    let Some(buckets) = value.get("data").and_then(Value::as_array) else {
        return 0;
    };
    let mut total = 0u64;
    for bucket in buckets {
        let Some(results) = bucket.get("results").and_then(Value::as_array) else {
            continue;
        };
        for result in results {
            total = total
                .saturating_add(json_u64(result, &["input_tokens"]))
                .saturating_add(json_u64(result, &["output_tokens"]));
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{ScriptedHttp, ScriptedRoute};
    use crate::paths::PathRoots;
    use crate::secrets::MemoryStore;
    use std::path::PathBuf;

    #[test]
    fn parses_auth_wham_and_org_usage() {
        let auth = parse_codex_auth(
            r#"{"OPENAI_API_KEY":"sk-test","tokens":{"access_token":"chatgpt-token","account_id":"acct_1"}}"#,
        );
        assert_eq!(auth.access_token.as_deref(), Some("chatgpt-token"));
        assert_eq!(auth.account_id.as_deref(), Some("acct_1"));
        assert_eq!(auth.api_key.as_deref(), Some("sk-test"));
        let wham = parse_wham(HttpResponse {
            status: 200,
            body: r#"{"plan_type":"plus","rate_limit":{"primary_window":{"used_percent":12.5,"limit_window_seconds":18000,"reset_at":1710000000},"secondary_window":{"used_percent":40}}}"#.into(),
            headers: Vec::new(),
        })
        .unwrap();
        assert_eq!(wham.used_percent, Some(12.5));
        assert_eq!(wham.plan.as_deref(), Some("plus"));
        assert_eq!(wham.windows[0].name, "five_hour");
        assert_eq!(wham.windows[1].name, "seven_day");
        let modern = parse_wham(HttpResponse {
            status: 200,
            body: r#"{"rate_limits":{"plan_type":"plus","primary":{"used_percent":37.5,"window_minutes":300,"resets_in_seconds":3600},"secondary":{"used_percent":12,"window_minutes":10080}}}"#.into(),
            headers: Vec::new(),
        })
        .unwrap();
        assert_eq!(modern.windows[0].name, "five_hour");
        assert_eq!(modern.windows[0].used_percent, Some(37.5));
        assert!(modern.windows[0].resets_at.is_some());
        assert_eq!(modern.windows[1].name, "seven_day");
        assert_eq!(modern.plan.as_deref(), Some("plus"));
        let org = sum_org_usage(&serde_json::json!({
            "data": [{"results": [{"input_tokens": 1000, "output_tokens": 200}]}]
        }));
        assert_eq!(org, 1_200);
    }

    #[test]
    fn jsonl_tokens_format_when_wham_is_absent() {
        let dir = std::env::temp_dir().join(format!("tokenbar-codex-{}", unique()));
        let day = dir.join("sessions").join("2026").join("11").join("02");
        fs::create_dir_all(&day).unwrap();
        fs::write(
            day.join("rollout-demo.jsonl"),
            r#"{"timestamp":"t","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1000,"output_tokens":200}}}}"#,
        )
        .unwrap();
        let paths = ProviderPaths::from_roots(PathRoots {
            home: PathBuf::from("/unused"),
            config: PathBuf::from("/unused"),
            claude_config_dir: None,
            codex_home: Some(dir.clone()),
        });
        let http = ScriptedHttp { routes: Vec::new() };
        let snapshot = fetch(&http, &MemoryStore::new(), &paths);
        assert_eq!(snapshot.tokens, Some(1_200));
        assert!(snapshot.used_percent.is_none());
        assert_eq!(
            snapshot
                .menu_label(crate::format::TitleMode::PercentText)
                .as_deref(),
            Some("O 1.2k")
        );
        assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn codex_log_rate_limits_replace_a_token_headline() {
        let dir = std::env::temp_dir().join(format!("tokenbar-codex-limits-{}", unique()));
        let day = dir.join("sessions").join("2026").join("11").join("02");
        fs::create_dir_all(&day).unwrap();
        fs::write(
            day.join("rollout-demo.jsonl"),
            concat!(
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":9000000,"output_tokens":700000}}}}"#,
                "\n",
                r#"{"type":"event_msg","payload":{"rate_limits":{"plan_type":"plus","primary":{"used_percent":20,"window_minutes":300},"secondary":{"used_percent":55,"window_minutes":10080}}}}"#,
                "\n",
            ),
        )
        .unwrap();
        let paths = ProviderPaths::from_roots(PathRoots {
            home: PathBuf::from("/unused"),
            config: PathBuf::from("/unused"),
            claude_config_dir: None,
            codex_home: Some(dir.clone()),
        });
        let snapshot = fetch(
            &ScriptedHttp { routes: Vec::new() },
            &MemoryStore::new(),
            &paths,
        );
        assert_eq!(snapshot.windows.len(), 2);
        assert_eq!(snapshot.windows[0].name, "five_hour");
        assert_eq!(snapshot.windows[1].name, "seven_day");
        assert_eq!(
            crate::format::status_item_title(
                std::slice::from_ref(&snapshot),
                crate::format::TitleMode::PercentText
            ),
            "O 5h 80% · 周 45%"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn wham_percent_wins_over_tokens() {
        let dir = std::env::temp_dir().join(format!("tokenbar-codex-wham-{}", unique()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("auth.json"),
            r#"{"tokens":{"access_token":"chatgpt-token","account_id":"acct_1"}}"#,
        )
        .unwrap();
        let paths = ProviderPaths::from_roots(PathRoots {
            home: PathBuf::from("/unused"),
            config: PathBuf::from("/unused"),
            claude_config_dir: None,
            codex_home: Some(dir.clone()),
        });
        let http = ScriptedHttp {
            routes: vec![ScriptedRoute {
                method: "GET",
                url_contains: "/backend-api/wham/usage",
                status: 200,
                body:
                    r#"{"plan_type":"pro","rate_limit":{"primary_window":{"used_percent":12.5}}}"#
                        .into(),
            }],
        };
        let snapshot = fetch(&http, &MemoryStore::new(), &paths);
        assert_eq!(
            snapshot
                .menu_label(crate::format::TitleMode::PercentText)
                .as_deref(),
            Some("O 87.5%")
        );
        assert_eq!(snapshot.plan.as_deref(), Some("pro"));
        let _ = fs::remove_dir_all(dir);
    }

    fn unique() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
}
