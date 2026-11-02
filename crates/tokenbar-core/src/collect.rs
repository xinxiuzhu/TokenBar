//! Build the snapshot the tray and `tokenbar --check` share.

use crate::format::is_offline_value;
use crate::http::{Http, ReqwestHttp};
use crate::model::{ProviderId, ProviderSnapshot, UsageSnapshot};
use crate::paths::ProviderPaths;
use crate::secrets::{open_store, MemoryStore, SecretStore};

pub fn collect_snapshot() -> UsageSnapshot {
    if is_offline_value(std::env::var("TOKENBAR_OFFLINE").ok().as_deref()) {
        return offline_snapshot();
    }
    let http = ReqwestHttp::new();
    let store = open_store().unwrap_or_else(|_| Box::new(MemoryStore::new()));
    collect_live(&http, store.as_ref(), &ProviderPaths::discover())
}

pub fn check_report() -> Result<String, String> {
    serde_json::to_string_pretty(&collect_snapshot()).map_err(|err| err.to_string())
}

pub fn offline_snapshot() -> UsageSnapshot {
    let mut cursor = ProviderSnapshot::empty(ProviderId::Cursor);
    cursor.used_percent = Some(42.0);
    cursor.source = "offline".to_string();
    cursor.detail = "offline stub".to_string();

    let mut claude = ProviderSnapshot::empty(ProviderId::Claude);
    claude.used_percent = Some(72.0);
    claude.source = "offline".to_string();
    claude.detail = "offline stub".to_string();

    let mut openai = ProviderSnapshot::empty(ProviderId::Openai);
    openai.tokens = Some(1_200);
    openai.source = "offline".to_string();
    openai.detail = "offline stub".to_string();

    UsageSnapshot::from_providers(true, vec![cursor, claude, openai])
}

pub fn placeholder_snapshot() -> UsageSnapshot {
    let providers = ProviderId::all()
        .into_iter()
        .map(|id| {
            let mut provider = ProviderSnapshot::empty(id);
            provider.source = "pending".to_string();
            provider.detail = "Refreshing".to_string();
            provider
        })
        .collect();
    UsageSnapshot::from_providers(false, providers)
}

pub(crate) fn collect_live(
    http: &dyn Http,
    secrets: &dyn SecretStore,
    paths: &ProviderPaths,
) -> UsageSnapshot {
    let providers = vec![
        crate::cursor::fetch(http, secrets, paths),
        crate::claude::fetch(http, secrets, paths),
        crate::openai::fetch(http, secrets, paths),
    ];
    UsageSnapshot::from_providers(false, providers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{ScriptedHttp, ScriptedRoute};
    use crate::paths::PathRoots;
    use crate::secrets::{MemoryStore, SecretKind};
    use std::fs;

    #[test]
    fn offline_stub_matches_tray_example() {
        let snapshot = offline_snapshot();
        assert!(snapshot.offline);
        assert_eq!(snapshot.title, "R ██░░ · C █░░░ · O 1.2k");
        assert_eq!(snapshot.providers[0].remaining_percent, Some(58.0));
        assert_eq!(snapshot.providers[1].remaining_percent, Some(28.0));
        assert_eq!(snapshot.providers[0].used_percent, Some(42.0));
        assert_eq!(snapshot.providers[1].used_percent, Some(72.0));
        assert_eq!(snapshot.providers[2].tokens, Some(1_200));
        assert!(snapshot.providers[2].used_percent.is_none());
        let json = serde_json::to_string(&snapshot).unwrap();
        let back: UsageSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.title, snapshot.title);
    }

    #[test]
    fn placeholder_title_is_dashes() {
        assert_eq!(placeholder_snapshot().title, "TokenBar");
    }

    #[test]
    fn live_sources_reproduce_the_example_title() {
        let root = std::env::temp_dir().join(format!("tokenbar-live-{}", unique()));
        let claude = root.join("claude");
        let projects = claude.join("projects").join("demo");
        fs::create_dir_all(&projects).unwrap();
        fs::write(
            projects.join("session.jsonl"),
            "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n",
        )
        .unwrap();
        fs::write(
            claude.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-example-token","subscriptionType":"max"}}"#,
        )
        .unwrap();
        let codex = root.join("codex");
        let day = codex.join("sessions").join("2026").join("11").join("02");
        fs::create_dir_all(&day).unwrap();
        fs::write(
            day.join("rollout.jsonl"),
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1000,"output_tokens":200}}}}"#,
        )
        .unwrap();
        let paths = ProviderPaths::from_roots(PathRoots {
            home: root.clone(),
            config: root.join("config"),
            claude_config_dir: Some(claude),
            codex_home: Some(codex),
        });
        let secrets = MemoryStore::new();
        secrets.insert(SecretKind::CursorSession, "user_example::session-token");
        let http = ScriptedHttp {
            routes: vec![
                ScriptedRoute {
                    method: "GET",
                    url_contains: "/api/usage-summary",
                    status: 200,
                    body: r#"{"membershipType":"pro","individualUsage":{"plan":{"totalPercentUsed":42}}}"#.into(),
                },
                ScriptedRoute {
                    method: "POST",
                    url_contains: "get-filtered-usage-events",
                    status: 200,
                    body: r#"{"totalUsageEventsCount":1,"usageEventsDisplay":[{"tokenUsage":{"inputTokens":10,"outputTokens":1}}]}"#.into(),
                },
                ScriptedRoute {
                    method: "GET",
                    url_contains: "/api/oauth/usage",
                    status: 200,
                    body: r#"{"five_hour":{"utilization":72,"resets_at":"2026-12-01T00:00:00Z"}}"#.into(),
                },
            ],
        };
        let snapshot = collect_live(&http, &secrets, &paths);
        assert_eq!(snapshot.title, "R ██░░ · C █░░░ · O 1.2k");
        assert_eq!(snapshot.providers[0].remaining_percent, Some(58.0));
        assert_eq!(snapshot.providers[1].remaining_percent, Some(28.0));
        assert!(!snapshot.offline);
        let _ = fs::remove_dir_all(root);
    }

    fn unique() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    }
}
