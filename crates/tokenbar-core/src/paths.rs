//! On-disk locations already written by Cursor, Claude Code, and Codex.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct ProviderPaths {
    pub cursor_db: PathBuf,
    pub claude_credentials: PathBuf,
    pub claude_projects: PathBuf,
    pub codex_auth: PathBuf,
    pub codex_session_roots: Vec<PathBuf>,
    pub opencode_db: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct PathRoots {
    pub home: PathBuf,
    pub config: PathBuf,
    pub claude_config_dir: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
}

impl ProviderPaths {
    pub fn discover() -> Self {
        Self::from_roots(PathRoots::from_process())
    }

    pub fn from_roots(roots: PathRoots) -> Self {
        let claude = roots
            .claude_config_dir
            .unwrap_or_else(|| roots.home.join(".claude"));
        let codex = resolve_codex_dir(&roots.home, &roots.config, roots.codex_home.as_deref());
        let mut session_roots = vec![codex.join("sessions"), codex.join("archived_sessions")];
        session_roots.retain(|path| path.exists());
        if session_roots.is_empty() {
            session_roots.push(codex.join("sessions"));
        }
        Self {
            cursor_db: cursor_db_path(&roots.config),
            claude_credentials: claude.join(".credentials.json"),
            claude_projects: claude.join("projects"),
            codex_auth: codex.join("auth.json"),
            codex_session_roots: session_roots,
            opencode_db: opencode_db(&roots.home),
        }
    }
}

fn opencode_db(home: &Path) -> PathBuf {
    let local = home
        .join(".local")
        .join("share")
        .join("opencode")
        .join("opencode.db");
    if local.exists() {
        return local;
    }
    if let Some(data) = dirs::data_local_dir() {
        let alt = data.join("opencode").join("opencode.db");
        if alt.exists() {
            return alt;
        }
    }
    local
}

impl PathRoots {
    pub fn from_process() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
        Self {
            home,
            config,
            claude_config_dir: std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
            codex_home: std::env::var_os("CODEX_HOME").map(PathBuf::from),
        }
    }
}

pub(crate) fn cursor_db_path(config_dir: &Path) -> PathBuf {
    config_dir
        .join("Cursor")
        .join("User")
        .join("globalStorage")
        .join("state.vscdb")
}

fn resolve_codex_dir(home: &Path, config: &Path, override_dir: Option<&Path>) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir.to_path_buf();
    }
    let primary = home.join(".codex");
    let alternate = config.join("codex");
    if primary.join("auth.json").exists() || primary.join("sessions").exists() {
        primary
    } else if alternate.join("auth.json").exists() || alternate.exists() {
        alternate
    } else {
        primary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_db_uses_global_storage() {
        let path = cursor_db_path(Path::new("/cfg"));
        assert_eq!(
            path,
            PathBuf::from("/cfg/Cursor/User/globalStorage/state.vscdb")
        );
    }

    #[test]
    fn claude_and_codex_honor_overrides() {
        let paths = ProviderPaths::from_roots(PathRoots {
            home: PathBuf::from("/home/dev"),
            config: PathBuf::from("/home/dev/.config"),
            claude_config_dir: Some(PathBuf::from("/tmp/claude")),
            codex_home: Some(PathBuf::from("/tmp/codex")),
        });
        assert_eq!(
            paths.claude_credentials,
            PathBuf::from("/tmp/claude/.credentials.json")
        );
        assert_eq!(paths.claude_projects, PathBuf::from("/tmp/claude/projects"));
        assert_eq!(paths.codex_auth, PathBuf::from("/tmp/codex/auth.json"));
    }
}
