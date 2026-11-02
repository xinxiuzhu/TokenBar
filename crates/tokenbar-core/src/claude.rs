//! Claude Code OAuth usage and local JSONL totals.
//!
//! Token order: a value pasted into TokenBar, then `~/.claude/.credentials.json`
//! (`claudeAiOauth.accessToken`), then the macOS keychain item
//! `Claude Code-credentials`. Live quota is `GET /api/oauth/usage`.
//! `utilization` is a 0–100 percentage. JSONL under `projects/` is summed on top.

use std::fs;

use serde_json::Value;

use crate::format::reset_string;
use crate::http::{http_failure, sanitize, Http};
use crate::jsonl::sum_claude_tree;
use crate::model::{clip_detail, tokens_or_none, ProviderId, ProviderSnapshot, UsageWindow};
use crate::paths::ProviderPaths;
use crate::secrets::{SecretKind, SecretStore};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

pub(crate) fn fetch(
    http: &dyn Http,
    secrets: &dyn SecretStore,
    paths: &ProviderPaths,
) -> ProviderSnapshot {
    let mut snapshot = ProviderSnapshot::empty(ProviderId::Claude);
    let local_tokens = if paths.claude_projects.is_dir() {
        sum_claude_tree(&paths.claude_projects)
    } else {
        0
    };
    snapshot.tokens = tokens_or_none(local_tokens);

    let found = match resolve_token(secrets, paths) {
        Ok(found) => found,
        Err(err) => {
            if snapshot.tokens.is_some() {
                snapshot.source = "jsonl".to_string();
                snapshot.detail = clip_detail([format!("{} local tokens", local_tokens), err]);
                snapshot.error = None;
            } else {
                snapshot.error = Some(err);
                snapshot.detail =
                    clip_detail([format!("expected {}", paths.claude_credentials.display())]);
            }
            return snapshot;
        }
    };
    let FoundToken {
        token,
        origin,
        plan,
    } = found;
    snapshot.plan = plan;

    let auth = format!("Bearer {token}");
    let headers = [
        ("Authorization", auth.as_str()),
        ("anthropic-beta", "oauth-2025-04-20"),
        ("anthropic-version", "2023-06-01"),
        ("Accept", "application/json"),
        ("User-Agent", "claude-code/2.1.0"),
    ];
    match http.get(USAGE_URL, &headers) {
        Ok(response) => match parse_usage(response) {
            Ok(parsed) => {
                snapshot.windows = parsed.windows;
                snapshot.used_percent = parsed.headline;
                snapshot.resets_at = parsed.resets_at;
                snapshot.source = if snapshot.tokens.is_some() {
                    "oauth+jsonl".to_string()
                } else {
                    "oauth".to_string()
                };
                snapshot.error = None;
                let mut parts = vec![format!("oauth from {origin}")];
                for window in &snapshot.windows {
                    if let Some(percent) = window.used_percent {
                        parts.push(format!("{} {percent}%", window.name));
                    }
                }
                if local_tokens > 0 {
                    parts.push(format!("jsonl {local_tokens} tokens"));
                }
                snapshot.detail = clip_detail(parts);
            }
            Err(err) => {
                snapshot.error = Some(err);
                snapshot.source = if snapshot.tokens.is_some() {
                    "jsonl".to_string()
                } else {
                    "missing".to_string()
                };
                snapshot.detail = clip_detail([
                    format!("oauth from {origin}"),
                    snapshot.error.clone().unwrap_or_default(),
                ]);
                if snapshot.tokens.is_some() {
                    snapshot.error = None;
                }
            }
        },
        Err(err) => {
            let message = format!("claude usage: {}", sanitize(&err));
            if snapshot.tokens.is_some() {
                snapshot.source = "jsonl".to_string();
                snapshot.detail = clip_detail([format!("jsonl {local_tokens} tokens"), message]);
            } else {
                snapshot.error = Some(message);
            }
        }
    }
    snapshot
}

struct FoundToken {
    token: String,
    origin: &'static str,
    plan: Option<String>,
}

fn resolve_token(secrets: &dyn SecretStore, paths: &ProviderPaths) -> Result<FoundToken, String> {
    if let Some(pasted) = secrets.get(SecretKind::ClaudeOauth) {
        return Ok(FoundToken {
            token: pasted,
            origin: "Settings",
            plan: None,
        });
    }
    if paths.claude_credentials.is_file() {
        let raw = fs::read_to_string(&paths.claude_credentials)
            .map_err(|err| format!("read {}: {err}", paths.claude_credentials.display()))?;
        if let Some(token) = access_token_from_credentials(&raw) {
            return Ok(FoundToken {
                token,
                origin: "credentials file",
                plan: subscription_type(&raw),
            });
        }
    }
    if let Some(token) = keychain_claude_token() {
        return Ok(FoundToken {
            token,
            origin: "keychain",
            plan: None,
        });
    }
    Err("no Claude OAuth token; paste one in Settings or run claude /login".to_string())
}

pub(crate) fn access_token_from_credentials(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return value
            .pointer("/claudeAiOauth/accessToken")
            .or_else(|| value.get("accessToken"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(str::to_string);
    }
    if trimmed.len() >= 20 && !trimmed.chars().any(char::is_whitespace) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn subscription_type(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    value
        .pointer("/claudeAiOauth/subscriptionType")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|plan| !plan.is_empty())
        .map(str::to_string)
}

#[cfg(target_os = "macos")]
fn keychain_claude_token() -> Option<String> {
    let user = std::env::var("USER").unwrap_or_else(|_| "claude".to_string());
    let accounts = [
        user.as_str(),
        "claude",
        "Claude Code",
        "Claude Code-credentials",
    ];
    for account in accounts {
        let Ok(entry) = keyring::Entry::new("Claude Code-credentials", account) else {
            continue;
        };
        let Ok(secret) = entry.get_password() else {
            continue;
        };
        if let Some(token) = access_token_from_credentials(&secret) {
            return Some(token);
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn keychain_claude_token() -> Option<String> {
    None
}

struct ParsedUsage {
    windows: Vec<UsageWindow>,
    headline: Option<f64>,
    resets_at: Option<String>,
}

fn parse_usage(response: crate::http::HttpResponse) -> Result<ParsedUsage, String> {
    if !(200..300).contains(&response.status) {
        return Err(http_failure(
            "claude usage",
            response.status,
            &response.body,
        ));
    }
    let value: Value = serde_json::from_str(&response.body)
        .map_err(|err| format!("claude usage: invalid JSON ({err})"))?;
    if let Some(err) = value
        .get("error")
        .and_then(|err| err.get("message").or(Some(err)))
        .and_then(Value::as_str)
    {
        return Err(format!("claude usage: {err}"));
    }
    let mut windows = Vec::new();
    for name in [
        "five_hour",
        "seven_day",
        "seven_day_sonnet",
        "seven_day_opus",
    ] {
        if let Some(window) = usage_window(name, value.get(name)) {
            windows.push(window);
        }
    }
    let headline = windows.iter().find_map(|window| window.used_percent);
    let resets_at = windows.iter().find_map(|window| window.resets_at.clone());
    if windows.is_empty() {
        return Err("claude usage response had no windows".to_string());
    }
    Ok(ParsedUsage {
        windows,
        headline,
        resets_at,
    })
}

fn usage_window(name: &str, value: Option<&Value>) -> Option<UsageWindow> {
    let value = value?;
    let used = value.get("utilization").and_then(Value::as_f64);
    let resets_at = value.get("resets_at").and_then(reset_string);
    if used.is_none() && resets_at.is_none() {
        return None;
    }
    Some(UsageWindow {
        name: name.to_string(),
        used_percent: used,
        resets_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{ScriptedHttp, ScriptedRoute};
    use crate::paths::PathRoots;
    use crate::secrets::MemoryStore;

    #[test]
    fn parses_credentials_and_utilization() {
        let raw = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-example-token","subscriptionType":"max"}}"#;
        assert_eq!(
            access_token_from_credentials(raw).as_deref(),
            Some("sk-ant-oat01-example-token")
        );
        assert_eq!(subscription_type(raw).as_deref(), Some("max"));
        let response = crate::http::HttpResponse {
            status: 200,
            body: r#"{"five_hour":{"utilization":72,"resets_at":"2026-12-01T00:00:00Z"},"seven_day":{"utilization":10,"resets_at":"2026-12-07T00:00:00Z"}}"#.into(),
            headers: Vec::new(),
        };
        let parsed = parse_usage(response).unwrap();
        assert_eq!(parsed.headline, Some(72.0));
        assert_eq!(parsed.windows.len(), 2);
    }

    #[test]
    fn oauth_percent_beats_jsonl_for_the_meter() {
        let dir = std::env::temp_dir().join(format!("tokenbar-claude-{}", unique()));
        let projects = dir.join("projects").join("demo");
        fs::create_dir_all(&projects).unwrap();
        fs::write(
            projects.join("session.jsonl"),
            "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n",
        )
        .unwrap();
        fs::write(
            dir.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-example-token","subscriptionType":"pro"}}"#,
        )
        .unwrap();
        let paths = ProviderPaths::from_roots(PathRoots {
            home: PathBuf::from("/unused"),
            config: PathBuf::from("/unused"),
            claude_config_dir: Some(dir.clone()),
            codex_home: None,
        });
        let http = ScriptedHttp {
            routes: vec![ScriptedRoute {
                method: "GET",
                url_contains: "/api/oauth/usage",
                status: 200,
                body: r#"{"five_hour":{"utilization":72,"resets_at":"2026-12-01T00:00:00Z"}}"#
                    .into(),
            }],
        };
        let snapshot = fetch(&http, &MemoryStore::new(), &paths);
        assert_eq!(snapshot.used_percent, Some(72.0));
        assert_eq!(snapshot.tokens, Some(15));
        assert_eq!(snapshot.meter(), "C72%");
        assert_eq!(snapshot.plan.as_deref(), Some("pro"));
        assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
        let _ = fs::remove_dir_all(dir);
    }

    fn unique() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    use std::path::PathBuf;
}
