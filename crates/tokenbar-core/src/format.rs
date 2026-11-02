//! Tray title formatting.
//!
//! Providers with no quota and no tokens are omitted. The default reads
//! `R 42% · C 72% · O 1.2k`.

use serde::{Deserialize, Serialize};

use crate::model::ProviderSnapshot;

/// What the menu-bar title prefers when a provider has both a quota and tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TitleMode {
    /// Quota already used, for example `C 62%`.
    #[default]
    UsedPercent,
    /// Quota still left, for example `C 38%`.
    RemainingPercent,
    /// Billable token total only.
    Tokens,
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Compact token count: `999`, `1.2k`, `3M`.
pub fn format_compact(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    let (divisor, suffix) = if n < 1_000_000 {
        (1_000.0, "k")
    } else if n < 1_000_000_000 {
        (1_000_000.0, "M")
    } else if n < 1_000_000_000_000 {
        (1_000_000_000.0, "B")
    } else {
        (1_000_000_000_000.0, "T")
    };
    let rounded = ((n as f64 / divisor) * 10.0).round() / 10.0;
    if (rounded - rounded.trunc()).abs() < 0.001 {
        format!("{}{suffix}", rounded as u64)
    } else {
        format!("{rounded:.1}{suffix}")
    }
}

/// `42`, `12.5`, clamped for display.
pub fn format_percent(pct: f64) -> String {
    if !pct.is_finite() {
        return "0".to_string();
    }
    let pct = pct.clamp(0.0, 999.0);
    let rounded = (pct * 10.0).round() / 10.0;
    if (rounded - rounded.round()).abs() < 0.001 {
        format!("{}", rounded.round() as i64)
    } else {
        format!("{rounded:.1}")
    }
}

pub fn format_meter(
    prefix: &str,
    used_percent: Option<f64>,
    tokens: Option<u64>,
    mode: TitleMode,
) -> Option<String> {
    match mode {
        TitleMode::Tokens => tokens.map(|count| format!("{prefix} {}", format_compact(count))),
        TitleMode::RemainingPercent => {
            if let Some(used) = used_percent {
                let remaining = (100.0 - used).clamp(0.0, 100.0);
                Some(format!("{prefix} {}%", format_percent(remaining)))
            } else {
                tokens.map(|count| format!("{prefix} {}", format_compact(count)))
            }
        }
        TitleMode::UsedPercent => {
            if let Some(used) = used_percent {
                Some(format!("{prefix} {}%", format_percent(used)))
            } else {
                tokens.map(|count| format!("{prefix} {}", format_compact(count)))
            }
        }
    }
}

pub fn tray_title(providers: &[ProviderSnapshot], mode: TitleMode) -> String {
    let parts: Vec<String> = providers
        .iter()
        .filter_map(|provider| provider.menu_label(mode))
        .collect();
    if parts.is_empty() {
        "TokenBar".to_string()
    } else {
        parts.join(" · ")
    }
}

/// Short Chinese reset countdown for a provider row.
pub fn reset_hint(resets_at: &str) -> String {
    let Ok(when) = chrono::DateTime::parse_from_rfc3339(resets_at) else {
        return String::new();
    };
    let secs = when
        .with_timezone(&chrono::Utc)
        .signed_duration_since(chrono::Utc::now())
        .num_seconds();
    if secs <= 0 {
        return "已重置".to_string();
    }
    if secs < 90 * 60 {
        return format!("{} 分钟后重置", (secs / 60).max(1));
    }
    if secs < 48 * 3600 {
        return format!("{} 小时后重置", (secs / 3600).max(1));
    }
    format!("重置 {}", resets_at.get(5..16).unwrap_or(resets_at))
}

/// `TOKENBAR_OFFLINE=1` selects the stub snapshot. Other values do not.
pub(crate) fn is_offline_value(value: Option<&str>) -> bool {
    value == Some("1")
}

pub(crate) fn reset_string(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        if let Ok(number) = text.parse::<i64>() {
            return epoch_to_rfc3339(number).or_else(|| Some(text.to_string()));
        }
        return Some(text.to_string());
    }
    if let Some(number) = value.as_i64() {
        return epoch_to_rfc3339(number);
    }
    value
        .as_f64()
        .filter(|number| number.is_finite())
        .and_then(|number| epoch_to_rfc3339(number as i64))
}

pub(crate) fn epoch_to_rfc3339(raw: i64) -> Option<String> {
    let secs = if raw.abs() > 1_000_000_000_000 {
        raw / 1000
    } else {
        raw
    };
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|stamp| stamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_matches_tray_example() {
        assert_eq!(format_compact(999), "999");
        assert_eq!(format_compact(1_000), "1k");
        assert_eq!(format_compact(1_200), "1.2k");
        assert_eq!(format_compact(1_500), "1.5k");
        assert_eq!(format_compact(1_200_000), "1.2M");
    }

    #[test]
    fn percent_trims_trailing_zero() {
        assert_eq!(format_percent(42.0), "42");
        assert_eq!(format_percent(72.04), "72");
        assert_eq!(format_percent(12.5), "12.5");
        assert_eq!(format_percent(99.96), "100");
    }

    #[test]
    fn meter_hides_empty_and_spaces_parts() {
        assert_eq!(
            format_meter("R", Some(42.0), Some(99), TitleMode::UsedPercent).as_deref(),
            Some("R 42%")
        );
        assert_eq!(
            format_meter("O", None, Some(1_200), TitleMode::UsedPercent).as_deref(),
            Some("O 1.2k")
        );
        assert_eq!(format_meter("C", None, None, TitleMode::UsedPercent), None);
        assert_eq!(
            format_meter("C", Some(72.0), None, TitleMode::RemainingPercent).as_deref(),
            Some("C 28%")
        );
    }

    #[test]
    fn offline_flag_is_exact() {
        assert!(is_offline_value(Some("1")));
        assert!(!is_offline_value(Some("true")));
        assert!(!is_offline_value(Some("0")));
        assert!(!is_offline_value(None));
    }
}
