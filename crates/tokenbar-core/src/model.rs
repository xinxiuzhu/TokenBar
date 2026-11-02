//! Snapshot types printed by `tokenbar --check` and drawn in the panel.

use serde::{Deserialize, Serialize};

use crate::format::{format_meter, tray_title};

/// Which product a row describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Cursor,
    Claude,
    Openai,
}

impl ProviderId {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cursor => "Cursor",
            Self::Claude => "Claude",
            Self::Openai => "OpenAI",
        }
    }

    /// Tray prefix. Cursor uses `R` (the R in CuRsor) so it does not collide
    /// with Claude's `C`.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Cursor => "R",
            Self::Claude => "C",
            Self::Openai => "O",
        }
    }

    pub fn all() -> [Self; 3] {
        [Self::Cursor, Self::Claude, Self::Openai]
    }
}

/// One quota window inside a provider payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageWindow {
    pub name: String,
    pub used_percent: Option<f64>,
    pub resets_at: Option<String>,
}

/// One provider's contribution to the tray and the JSON snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub id: ProviderId,
    pub label: String,
    pub prefix: String,
    /// Quota used, on a 0–100 scale, when the provider reports one.
    pub used_percent: Option<f64>,
    /// Token total when the provider has no quota percent, or a supplementary count.
    pub tokens: Option<u64>,
    pub plan: Option<String>,
    pub resets_at: Option<String>,
    /// Short source tag such as `usage-summary+events` or `missing`.
    pub source: String,
    pub detail: String,
    pub error: Option<String>,
    pub windows: Vec<UsageWindow>,
}

impl ProviderSnapshot {
    pub fn empty(id: ProviderId) -> Self {
        Self {
            id,
            label: id.label().to_string(),
            prefix: id.prefix().to_string(),
            used_percent: None,
            tokens: None,
            plan: None,
            resets_at: None,
            source: "missing".to_string(),
            detail: String::new(),
            error: None,
            windows: Vec::new(),
        }
    }

    pub fn meter(&self) -> String {
        format_meter(&self.prefix, self.used_percent, self.tokens)
    }
}

/// Full check payload. `title` is what the tray shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub generated_at: String,
    pub offline: bool,
    pub title: String,
    pub providers: Vec<ProviderSnapshot>,
}

impl UsageSnapshot {
    pub fn from_providers(offline: bool, providers: Vec<ProviderSnapshot>) -> Self {
        let title = tray_title(&providers);
        Self {
            generated_at: crate::format::now_rfc3339(),
            offline,
            title,
            providers,
        }
    }
}

pub(crate) fn tokens_or_none(value: u64) -> Option<u64> {
    if value == 0 {
        None
    } else {
        Some(value)
    }
}

pub(crate) fn clip_detail(parts: impl IntoIterator<Item = String>) -> String {
    let mut text = parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    const MAX: usize = 400;
    if text.chars().count() > MAX {
        text = text.chars().take(MAX).collect();
        text.push('…');
    }
    text
}
