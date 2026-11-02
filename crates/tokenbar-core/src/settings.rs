//! Which providers and which menu-bar style the panel shows.
//!
//! This file is not a secret store. Credentials stay in the keychain or the
//! `0600` secrets file.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::format::TitleMode;
use crate::model::{ProviderId, ProviderSnapshot};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplaySettings {
    #[serde(default = "yes")]
    pub show_cursor: bool,
    #[serde(default = "yes")]
    pub show_claude: bool,
    #[serde(default = "yes")]
    pub show_openai: bool,
    #[serde(default = "yes")]
    pub show_opencode: bool,
    #[serde(default = "yes")]
    pub show_deepseek: bool,
    /// When set, the tray shows only this provider.
    #[serde(default)]
    pub primary: Option<ProviderId>,
    #[serde(default)]
    pub title_mode: TitleMode,
    /// Seconds between refreshes. Clamped to 30 seconds through one day.
    #[serde(default = "five_minutes")]
    pub refresh_secs: u64,
}

fn yes() -> bool {
    true
}

fn five_minutes() -> u64 {
    300
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            show_cursor: true,
            show_claude: true,
            show_openai: true,
            show_opencode: true,
            show_deepseek: true,
            primary: None,
            title_mode: TitleMode::BalanceBars,
            refresh_secs: 300,
        }
    }
}

impl DisplaySettings {
    pub fn shows(&self, id: ProviderId) -> bool {
        match id {
            ProviderId::Cursor => self.show_cursor,
            ProviderId::Claude => self.show_claude,
            ProviderId::Openai => self.show_openai,
            ProviderId::Opencode => self.show_opencode,
            ProviderId::Deepseek => self.show_deepseek,
        }
    }

    pub fn menu_title(&self, providers: &[ProviderSnapshot]) -> String {
        let visible: Vec<ProviderSnapshot> = providers
            .iter()
            .filter(|provider| self.shows(provider.id))
            .cloned()
            .collect();
        if let Some(primary) = self.primary {
            if self.shows(primary) {
                if let Some(one) = visible.iter().find(|provider| provider.id == primary) {
                    if one.menu_label(self.title_mode).is_some() {
                        return crate::format::tray_title(
                            std::slice::from_ref(one),
                            self.title_mode,
                        );
                    }
                }
            }
        }
        crate::format::tray_title(&visible, self.title_mode)
    }

    pub fn normalized(mut self) -> Self {
        self.refresh_secs = self.refresh_secs.clamp(30, 24 * 60 * 60);
        self
    }
}

pub fn load_settings() -> DisplaySettings {
    let Some(text) = fs::read_to_string(settings_path()).ok() else {
        return DisplaySettings::default();
    };
    serde_json::from_str::<DisplaySettings>(&text)
        .map(DisplaySettings::normalized)
        .unwrap_or_default()
}

pub fn save_settings(settings: &DisplaySettings) -> Result<(), String> {
    let settings = settings.clone().normalized();
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create settings dir: {err}"))?;
    }
    let bytes = serde_json::to_vec_pretty(&settings).map_err(|err| err.to_string())?;
    fs::write(&path, bytes).map_err(|err| format!("write settings: {err}"))
}

fn settings_path() -> PathBuf {
    if let Some(path) = std::env::var_os("TOKENBAR_SETTINGS_PATH") {
        return PathBuf::from(path);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tokenbar")
        .join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_provider_drops_out_of_the_title() {
        let mut cursor = ProviderSnapshot::empty(ProviderId::Cursor);
        cursor.used_percent = Some(42.0);
        let mut claude = ProviderSnapshot::empty(ProviderId::Claude);
        claude.used_percent = Some(62.0);
        let openai = ProviderSnapshot::empty(ProviderId::Openai);
        let providers = vec![
            cursor.with_derived(),
            claude.with_derived(),
            openai.with_derived(),
        ];
        let hidden = DisplaySettings {
            show_cursor: false,
            ..DisplaySettings::default()
        };
        assert_eq!(hidden.menu_title(&providers), "C ███░░░░░ 38");
        let text = DisplaySettings {
            title_mode: TitleMode::PercentText,
            ..DisplaySettings::default()
        };
        assert_eq!(text.menu_title(&providers), "R 58% · C 38%");
    }
}
