//! Tray title formatting.
//!
//! A percent is preferred when the provider reports quota used. Otherwise the
//! meter falls back to a compact token count: `R42% C72% O1.2k`.

use crate::model::ProviderSnapshot;

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

pub fn format_meter(prefix: &str, used_percent: Option<f64>, tokens: Option<u64>) -> String {
    if let Some(pct) = used_percent {
        format!("{prefix}{}%", format_percent(pct))
    } else if let Some(tokens) = tokens {
        format!("{prefix}{}", format_compact(tokens))
    } else {
        format!("{prefix}--")
    }
}

pub fn tray_title(providers: &[ProviderSnapshot]) -> String {
    providers
        .iter()
        .map(ProviderSnapshot::meter)
        .collect::<Vec<_>>()
        .join(" ")
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
    fn meter_prefers_percent() {
        assert_eq!(format_meter("R", Some(42.0), Some(99)), "R42%");
        assert_eq!(format_meter("O", None, Some(1_200)), "O1.2k");
        assert_eq!(format_meter("C", None, None), "C--");
    }

    #[test]
    fn offline_flag_is_exact() {
        assert!(is_offline_value(Some("1")));
        assert!(!is_offline_value(Some("true")));
        assert!(!is_offline_value(Some("0")));
        assert!(!is_offline_value(None));
    }
}
