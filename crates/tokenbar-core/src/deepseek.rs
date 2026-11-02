//! DeepSeek prepaid balance.
//!
//! The key is one the user pastes. The endpoint returns currency balances,
//! not a subscription percentage.

use serde_json::Value;

use crate::http::{http_failure, sanitize, Http};
use crate::model::{ProviderId, ProviderSnapshot};
use crate::secrets::{SecretKind, SecretStore};

const BALANCE_URL: &str = "https://api.deepseek.com/user/balance";

pub(crate) fn fetch(http: &dyn Http, secrets: &dyn SecretStore) -> ProviderSnapshot {
    let mut snapshot = ProviderSnapshot::empty(ProviderId::Deepseek);
    let Some(key) = secrets
        .get(SecretKind::DeepseekKey)
        .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok())
        .filter(|key| !key.trim().is_empty())
    else {
        snapshot.detail = "DeepSeek key not set".to_string();
        return snapshot;
    };
    let auth = format!("Bearer {}", key.trim());
    let headers = [
        ("Authorization", auth.as_str()),
        ("Accept", "application/json"),
    ];
    match http.get(BALANCE_URL, &headers) {
        Ok(response) => match parse_balance(response) {
            Ok(parsed) => {
                snapshot.balance = Some(parsed.balance);
                snapshot.plan = Some(parsed.currency);
                snapshot.source = "balance".to_string();
                snapshot.detail = if parsed.available {
                    "prepaid balance".to_string()
                } else {
                    "balance depleted".to_string()
                };
                if !parsed.available {
                    snapshot.error = Some("余额已用完".to_string());
                }
            }
            Err(err) => snapshot.error = Some(err),
        },
        Err(err) => snapshot.error = Some(format!("deepseek: {}", sanitize(&err))),
    }
    snapshot
}

struct ParsedBalance {
    balance: String,
    currency: String,
    available: bool,
}

fn parse_balance(response: crate::http::HttpResponse) -> Result<ParsedBalance, String> {
    if response.status == 401 || response.status == 403 {
        return Err("DeepSeek key was rejected".to_string());
    }
    if !(200..300).contains(&response.status) {
        return Err(http_failure(
            "deepseek balance",
            response.status,
            &response.body,
        ));
    }
    let value: Value = serde_json::from_str(&response.body)
        .map_err(|err| format!("deepseek balance: invalid JSON ({err})"))?;
    let available = value
        .get("is_available")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let infos = value
        .get("balance_infos")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let info = infos
        .iter()
        .find(|item| item.get("currency").and_then(Value::as_str) == Some("USD"))
        .or_else(|| {
            infos
                .iter()
                .find(|item| item.get("currency").and_then(Value::as_str) == Some("CNY"))
        })
        .or_else(|| infos.first());
    let Some(info) = info else {
        return Err("deepseek balance response had no balances".to_string());
    };
    let currency = info
        .get("currency")
        .and_then(Value::as_str)
        .unwrap_or("USD")
        .to_string();
    let total = info
        .get("total_balance")
        .map(json_number_string)
        .unwrap_or_else(|| "0".to_string());
    Ok(ParsedBalance {
        balance: display_balance(&total, &currency),
        currency,
        available,
    })
}

fn json_number_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_f64().map(|number| format!("{number:.2}")))
        .or_else(|| value.as_i64().map(|number| number.to_string()))
        .unwrap_or_else(|| "0".to_string())
}

fn display_balance(amount: &str, currency: &str) -> String {
    let amount = amount.trim();
    match currency {
        "USD" => format!("${amount}"),
        "CNY" => format!("¥{amount}"),
        other => format!("{other} {amount}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpResponse;

    #[test]
    fn prefers_usd_and_formats_cny() {
        assert_eq!(display_balance("4.20", "USD"), "$4.20");
        assert_eq!(display_balance("12", "CNY"), "¥12");
        let parsed = parse_balance(HttpResponse {
            status: 200,
            body: r#"{"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"8"},{"currency":"USD","total_balance":"3.5"}]}"#.into(),
            headers: Vec::new(),
        })
        .unwrap();
        assert_eq!(parsed.balance, "$3.5");
        assert!(parsed.available);
    }
}
