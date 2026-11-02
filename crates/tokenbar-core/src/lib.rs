//! Provider core for TokenBar.
//!
//! This crate talks to Cursor, Claude, and OpenAI/Codex, reads the credential
//! files those tools already keep on disk, and shapes a tray title. It does
//! not link GPUI, so `tokenbar --check` and the unit tests stay headless.
//!
//! Credential locations and quota endpoints follow the same public integration
//! points used by the MIT-licensed Syrtis project. The implementation here is
//! original; see `NOTICE`.

mod claude;
mod collect;
mod cursor;
mod format;
mod http;
mod jsonl;
mod model;
mod openai;
mod paths;
mod secrets;
mod settings;

pub use collect::{check_report, collect_snapshot, offline_snapshot, placeholder_snapshot};
pub use format::{
    format_compact, format_percent, remaining_percent, reset_hint, tray_title, TitleMode,
};
pub use model::{ProviderId, ProviderSnapshot, UsageSnapshot, UsageWindow};
pub use secrets::{delete_secret, save_secret, secret_presence, SecretKind, SecretPresence};
pub use settings::{load_settings, save_settings, DisplaySettings};
