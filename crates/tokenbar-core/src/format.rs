//! Tray title formatting.
//!
//! gpui-tray exposes the macOS status item title as a plain string, so the
//! default meter is a unicode bar of remaining quota. A full bar is a healthy
//! balance. Providers with nothing to show are omitted.

use serde::{Deserialize, Serialize};

use crate::model::ProviderSnapshot;

/// How the menu bar presents quota.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TitleMode {
    /// Block meter of remaining percent. A full bar means balance left.
    #[default]
    BalanceBars,
    /// Remaining percent as text, for example `C 38%`.
    PercentText,
    /// No title text. The tray icon carries the meter.
    IconOnly,
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

pub fn remaining_percent(used_percent: f64) -> f64 {
    (100.0 - used_percent).clamp(0.0, 100.0)
}

/// `█` is remaining balance. `░` is the part already spent.
pub fn block_meter(remaining: f64, cells: usize) -> String {
    let cells = cells.max(1);
    let filled = ((remaining.clamp(0.0, 100.0) / 100.0) * cells as f64).round() as usize;
    let filled = filled.min(cells);
    let mut bar = String::with_capacity(cells * 3);
    for index in 0..cells {
        bar.push(if index < filled { '█' } else { '░' });
    }
    bar
}

pub fn format_meter(
    prefix: &str,
    used_percent: Option<f64>,
    tokens: Option<u64>,
    mode: TitleMode,
) -> Option<String> {
    match mode {
        TitleMode::IconOnly => None,
        TitleMode::PercentText => percent_label(prefix, used_percent, tokens),
        TitleMode::BalanceBars => {
            // Cell count is chosen in `tray_title`, where we know how many
            // providers are visible. A lone call uses the short bar.
            if let Some(used) = used_percent {
                let bar = block_meter(remaining_percent(used), 4);
                Some(format!("{prefix} {bar}"))
            } else {
                tokens.map(|count| format!("{prefix} {}", format_compact(count)))
            }
        }
    }
}

fn percent_label(prefix: &str, used_percent: Option<f64>, tokens: Option<u64>) -> Option<String> {
    if let Some(used) = used_percent {
        Some(format!(
            "{prefix} {}%",
            format_percent(remaining_percent(used))
        ))
    } else {
        tokens.map(|count| format!("{prefix} {}", format_compact(count)))
    }
}

pub fn tray_title(providers: &[ProviderSnapshot], mode: TitleMode) -> String {
    if mode == TitleMode::IconOnly {
        return String::new();
    }
    let visible: Vec<&ProviderSnapshot> = providers
        .iter()
        .filter(|provider| {
            provider.used_percent.is_some()
                || provider.tokens.is_some()
                || provider.balance.is_some()
        })
        .collect();
    if visible.is_empty() {
        return "TokenBar".to_string();
    }
    if mode == TitleMode::PercentText {
        let parts: Vec<String> = visible
            .iter()
            .filter_map(|provider| {
                percent_label(&provider.prefix, provider.used_percent, provider.tokens).or_else(
                    || {
                        provider
                            .balance
                            .as_ref()
                            .map(|balance| format!("{} {balance}", provider.prefix))
                    },
                )
            })
            .collect();
        return parts.join(" · ");
    }
    let quota_count = visible
        .iter()
        .filter(|provider| provider.used_percent.is_some())
        .count();
    let cells = if quota_count <= 1 { 8 } else { 4 };
    let parts: Vec<String> = visible
        .iter()
        .filter_map(|provider| balance_part(provider, cells, quota_count == 1))
        .collect();
    if parts.is_empty() {
        "TokenBar".to_string()
    } else {
        parts.join(" · ")
    }
}

fn balance_part(provider: &ProviderSnapshot, cells: usize, show_number: bool) -> Option<String> {
    if let Some(used) = provider.used_percent {
        let remaining = remaining_percent(used);
        let bar = block_meter(remaining, cells);
        if show_number {
            Some(format!(
                "{} {bar} {}",
                provider.prefix,
                format_percent(remaining)
            ))
        } else {
            Some(format!("{} {bar}", provider.prefix))
        }
    } else if let Some(balance) = &provider.balance {
        Some(format!("{} {balance}", provider.prefix))
    } else {
        provider
            .tokens
            .map(|count| format!("{} {}", provider.prefix, format_compact(count)))
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
    fn balance_bar_fills_with_remaining() {
        assert_eq!(block_meter(100.0, 4), "████");
        assert_eq!(block_meter(0.0, 4), "░░░░");
        assert_eq!(block_meter(58.0, 4), "██░░");
        assert_eq!(block_meter(28.0, 8), "██░░░░░░");
    }

    #[test]
    fn meter_hides_empty_providers() {
        assert_eq!(
            format_meter("O", None, Some(1_200), TitleMode::PercentText).as_deref(),
            Some("O 1.2k")
        );
        assert_eq!(format_meter("C", None, None, TitleMode::PercentText), None);
        assert_eq!(
            format_meter("C", Some(72.0), None, TitleMode::PercentText).as_deref(),
            Some("C 28%")
        );
        assert_eq!(
            format_meter("C", Some(72.0), None, TitleMode::BalanceBars).as_deref(),
            Some("C █░░░")
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
