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
    let plan = value
        .get("plan_type")
        .and_then(Value::as_str)
        .map(str::to_string);
    let rate = value.get("rate_limit").unwrap_or(&value);
    let mut windows = Vec::new();
    if let Some(window) = rate_window("primary", rate.get("primary_window")) {
        windows.push(window);
    }
    if let Some(window) = rate_window("secondary", rate.get("secondary_window")) {
        windows.push(window);
    }
    if windows.is_empty() {
        if let Some(percent) = header_percent(&response.headers, "x-codex-primary-used-percent") {
            windows.push(UsageWindow {
                name: "primary".to_string(),
                used_percent: Some(percent),
                resets_at: None,
            });
        }
    }
    if windows.is_empty() {
        return Err("codex wham response had no rate-limit windows".to_string());
    }
    let headline = windows.iter().find_map(|window| window.used_percent);
    let resets_at = windows.iter().find_map(|window| window.resets_at.clone());
    Ok(WhamUsage {
        used_percent: headline,
        resets_at,
        plan,
        windows,
    })
}

fn rate_window(name: &str, value: Option<&Value>) -> Option<UsageWindow> {
    let value = value?;
    let used = value
        .get("used_percent")
        .or_else(|| value.get("usedPercent"))
        .and_then(Value::as_f64);
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
                .menu_label(crate::format::TitleMode::UsedPercent)
                .as_deref(),
            Some("O 1.2k")
        );
        assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
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
                .menu_label(crate::format::TitleMode::UsedPercent)
                .as_deref(),
            Some("O 12.5%")
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
