//! Snapshot types printed by `tokenbar --check` and drawn in the panel.

use serde::{Deserialize, Serialize};

use crate::format::{tray_title, TitleMode};

/// Which product a row describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Cursor,
    Claude,
    Openai,
    Opencode,
    Deepseek,
}

impl ProviderId {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cursor => "Cursor",
            Self::Claude => "Claude",
            Self::Openai => "OpenAI",
            Self::Opencode => "OpenCode",
            Self::Deepseek => "DeepSeek",
        }
    }

    /// Tray prefix. Cursor uses `R` (the R in CuRsor) so it does not collide
    /// with Claude's `C`.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Cursor => "R",
            Self::Claude => "C",
            Self::Openai => "O",
            Self::Opencode => "OC",
            Self::Deepseek => "D",
        }
    }

    pub fn all() -> [Self; 5] {
        [
            Self::Claude,
            Self::Openai,
            Self::Opencode,
            Self::Deepseek,
            Self::Cursor,
        ]
    }
}

/// One model's local token and cost total.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelUsage {
    pub name: String,
    pub tokens: u64,
    pub cost: Option<f64>,
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
    /// `100 - used_percent` when a plan window exists.
    pub remaining_percent: Option<f64>,
    /// Billable tokens. Cache reads are not included.
    pub tokens: Option<u64>,
    /// Cache-read tokens, kept off the headline.
    pub cache_read_tokens: Option<u64>,
    /// Prepaid balance, such as `$4.20` or `¥12.00`. Not a percentage.
    pub balance: Option<String>,
    /// Provider-reported cost, when the local log already computed it.
    pub cost: Option<f64>,
    pub models: Vec<ModelUsage>,
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
            remaining_percent: None,
            tokens: None,
            cache_read_tokens: None,
            balance: None,
            cost: None,
            models: Vec::new(),
            plan: None,
            resets_at: None,
            source: "missing".to_string(),
            detail: String::new(),
            error: None,
            windows: Vec::new(),
        }
    }

    pub fn menu_label(&self, mode: TitleMode) -> Option<String> {
        if self.used_percent.is_none() && self.tokens.is_none() {
            return self
                .balance
                .as_ref()
                .map(|balance| format!("{} {balance}", self.prefix));
        }
        crate::format::format_meter(&self.prefix, self.used_percent, self.tokens, mode)
    }

    pub(crate) fn with_derived(mut self) -> Self {
        self.remaining_percent = self
            .used_percent
            .map(|used| (100.0 - used).clamp(0.0, 100.0));
        self
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
        let providers: Vec<ProviderSnapshot> = providers
            .into_iter()
            .map(ProviderSnapshot::with_derived)
            .collect();
        let title = tray_title(&providers, TitleMode::BalanceBars);
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
