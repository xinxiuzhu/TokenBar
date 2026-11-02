//! Local session logs.
//!
//! Claude Code writes one JSON object per line under `~/.claude/projects`.
//! Codex writes rollout JSONL under `$CODEX_HOME/sessions`. Both are summed
//! here. Message text is ignored; only token counters are kept.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

const MAX_FILES: usize = 4_000;
const MAX_DEPTH: usize = 10;
const MAX_BYTES_PER_FILE: usize = 32 * 1024 * 1024;
const MAX_LINE: usize = 1_000_000;

pub(crate) fn sum_claude_tree(root: &Path) -> u64 {
    let mut total = 0u64;
    for_jsonl(root, |line| {
        total = total.saturating_add(claude_line_tokens(line));
    });
    total
}

pub(crate) fn sum_codex_trees(roots: &[PathBuf]) -> u64 {
    roots
        .iter()
        .map(|root| sum_codex_tree(root))
        .fold(0u64, u64::saturating_add)
}

fn sum_codex_tree(root: &Path) -> u64 {
    let mut files = Vec::new();
    let mut seen = 0usize;
    collect_files(root, 0, &mut files, &mut seen);
    files.iter().map(|path| codex_file_tokens(path)).sum()
}

pub(crate) fn claude_line_tokens(line: &str) -> u64 {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return 0;
    };
    if let Some(kind) = value.get("type").and_then(Value::as_str) {
        if kind != "assistant" {
            return 0;
        }
    }
    let usage = value
        .pointer("/message/usage")
        .or_else(|| value.get("usage"));
    usage.map(sum_known_token_fields).unwrap_or(0)
}

pub(crate) fn codex_text_tokens(text: &str) -> u64 {
    let mut from_last = 0u64;
    let mut saw_last = false;
    let mut previous_total: Option<u64> = None;
    let mut from_delta = 0u64;
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(info) = token_count_info(&value) else {
            continue;
        };
        if let Some(last) = info.get("last_token_usage") {
            let count = usage_block_tokens(last);
            if count > 0 {
                from_last = from_last.saturating_add(count);
                saw_last = true;
            }
            if let Some(total) = info.get("total_token_usage") {
                previous_total = Some(usage_block_tokens(total));
            }
            continue;
        }
        if let Some(total_value) = info.get("total_token_usage") {
            let total = usage_block_tokens(total_value);
            if let Some(previous) = previous_total {
                if total > previous {
                    from_delta = from_delta.saturating_add(total - previous);
                }
            } else if total > 0 {
                from_delta = from_delta.saturating_add(total);
            }
            previous_total = Some(previous_total.map_or(total, |prev| prev.max(total)));
        }
    }
    if saw_last {
        from_last
    } else {
        from_delta
    }
}

fn token_count_info(value: &Value) -> Option<&Value> {
    let payload = if value.get("type").and_then(Value::as_str) == Some("event_msg") {
        value.get("payload")?
    } else {
        value
    };
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    payload.get("info").filter(|info| !info.is_null())
}

fn usage_block_tokens(block: &Value) -> u64 {
    let input = json_u64(block, &["input_tokens", "inputTokens"]);
    let output = json_u64(block, &["output_tokens", "outputTokens"]);
    let reasoning = json_u64(block, &["reasoning_output_tokens", "reasoningOutputTokens"]);
    input.saturating_add(output).saturating_add(reasoning)
}

fn sum_known_token_fields(usage: &Value) -> u64 {
    const KEYS: &[&str] = &[
        "input_tokens",
        "output_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
        "cache_write_input_tokens",
    ];
    KEYS.iter()
        .map(|key| json_u64(usage, &[key]))
        .fold(0u64, u64::saturating_add)
}

pub(crate) fn json_u64(value: &Value, keys: &[&str]) -> u64 {
    for key in keys {
        if let Some(number) = value.get(*key) {
            if let Some(parsed) = number.as_u64() {
                return parsed;
            }
            if let Some(parsed) = number.as_i64() {
                return parsed.max(0) as u64;
            }
            if let Some(parsed) = number.as_f64() {
                if parsed.is_finite() && parsed > 0.0 {
                    return parsed as u64;
                }
            }
        }
    }
    0
}

fn codex_file_tokens(path: &Path) -> u64 {
    let Ok(file) = File::open(path) else {
        return 0;
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut bytes = 0usize;
    let mut text = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line).unwrap_or(0);
        if read == 0 {
            break;
        }
        bytes = bytes.saturating_add(read);
        if line.len() <= MAX_LINE {
            text.push_str(&line);
        }
        if bytes > MAX_BYTES_PER_FILE {
            break;
        }
    }
    codex_text_tokens(&text)
}

fn for_jsonl(root: &Path, mut on_line: impl FnMut(&str)) {
    let mut files = Vec::new();
    let mut seen = 0usize;
    collect_files(root, 0, &mut files, &mut seen);
    for path in files {
        let Ok(file) = File::open(&path) else {
            continue;
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut bytes = 0usize;
        loop {
            line.clear();
            let read = reader.read_line(&mut line).unwrap_or(0);
            if read == 0 {
                break;
            }
            bytes = bytes.saturating_add(read);
            if line.len() <= MAX_LINE {
                on_line(&line);
            }
            if bytes > MAX_BYTES_PER_FILE {
                break;
            }
        }
    }
}

fn collect_files(dir: &Path, depth: usize, out: &mut Vec<std::path::PathBuf>, seen: &mut usize) {
    if depth > MAX_DEPTH || *seen >= MAX_FILES || !dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if *seen >= MAX_FILES {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, depth + 1, out, seen);
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
        {
            out.push(path);
            *seen += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_assistant_usage_is_summed() {
        let line = r#"{"type":"assistant","message":{"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":2}}}"#;
        assert_eq!(claude_line_tokens(line), 17);
        let skipped = r#"{"type":"user","message":{"usage":{"input_tokens":99}}}"#;
        assert_eq!(claude_line_tokens(skipped), 0);
    }

    #[test]
    fn codex_prefers_last_token_usage_over_a_huge_total() {
        let text = r#"
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":900000,"output_tokens":1},"last_token_usage":{"input_tokens":1000,"output_tokens":200,"reasoning_output_tokens":0}}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":0,"output_tokens":0}}}}
"#;
        assert_eq!(codex_text_tokens(text), 1_200);
    }

    #[test]
    fn codex_total_fallback_uses_positive_deltas() {
        let text = r#"
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"output_tokens":20}}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"output_tokens":20}}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":80,"output_tokens":10}}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"output_tokens":30}}}}
"#;
        // First snapshot 120, replay ignored, regression ignored, then 180-120 = 60.
        assert_eq!(codex_text_tokens(text), 180);
    }
}
