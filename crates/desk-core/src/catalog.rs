//! Built-in CLI catalog. Probe scans the machine for these, then runs a short ACP handshake.

use std::path::PathBuf;

pub struct CliSpec {
    pub name: &'static str,
    pub env_bin: &'static str,
    pub detect: &'static [&'static str],
    pub acp_bins: &'static [&'static str],
    pub acp_args: &'static [&'static str],
    pub auth: Option<&'static str>,
    pub adapter_hint: Option<&'static str>,
    pub adapter_npm: Option<&'static str>,
}

pub const SUPPORTED: &[CliSpec] = &[
    CliSpec {
        name: "cursor",
        env_bin: "DESK_CURSOR_AGENT",
        detect: &["agent", "cursor-agent"],
        acp_bins: &["agent", "cursor-agent"],
        acp_args: &["acp"],
        auth: Some("cursor_login"),
        adapter_hint: None,
        adapter_npm: None,
    },
    CliSpec {
        name: "gemini",
        env_bin: "DESK_GEMINI",
        detect: &["gemini"],
        acp_bins: &["gemini"],
        acp_args: &["--acp"],
        auth: None,
        adapter_hint: None,
        adapter_npm: None,
    },
    CliSpec {
        name: "codex",
        env_bin: "DESK_CODEX_ACP",
        detect: &["codex", "codex-acp"],
        acp_bins: &["codex-acp"],
        acp_args: &[],
        auth: None,
        adapter_hint: Some("已安装 Codex，缺少 ACP 适配器"),
        adapter_npm: Some("@agentclientprotocol/codex-acp"),
    },
    CliSpec {
        name: "claude",
        env_bin: "DESK_CLAUDE_ACP",
        detect: &["claude", "claude-acp", "claude-agent-acp"],
        acp_bins: &["claude-acp", "claude-agent-acp"],
        acp_args: &[],
        auth: None,
        adapter_hint: Some("已安装 Claude Code，缺少 ACP 适配器"),
        adapter_npm: Some("@agentclientprotocol/claude-agent-acp"),
    },
    CliSpec {
        name: "antigravity",
        env_bin: "DESK_ANTIGRAVITY_ACP",
        detect: &["agy", "agy_acp_server", "agy_acp_server.par"],
        acp_bins: &["agy_acp_server", "agy_acp_server.par"],
        acp_args: &[],
        auth: None,
        adapter_hint: Some(
            "已安装 Antigravity CLI（agy），但 ACP 要用独立的 agy_acp_server；agy 本身没有 --acp",
        ),
        adapter_npm: None,
    },
];

pub enum Discovery {
    Ready {
        name: &'static str,
        bin: PathBuf,
        args: &'static [&'static str],
        auth: Option<&'static str>,
    },
    NeedsAdapter {
        name: &'static str,
        detail: String,
        package: Option<&'static str>,
    },
}

pub fn discover_installed() -> Vec<Discovery> {
    SUPPORTED.iter().filter_map(discover_one).collect()
}

pub fn find_executable(env_key: &str, names: &[&str]) -> Option<PathBuf> {
    if !env_key.is_empty() {
        if let Some(explicit) = std::env::var_os(env_key) {
            return Some(PathBuf::from(explicit));
        }
    }
    search_names(names)
}

fn discover_one(spec: &CliSpec) -> Option<Discovery> {
    let present = search_names(spec.detect).is_some() || (!spec.env_bin.is_empty() && std::env::var_os(spec.env_bin).is_some());
    if !present {
        return None;
    }
    match find_executable(spec.env_bin, spec.acp_bins) {
        Some(bin) if bin.is_file() => Some(Discovery::Ready {
            name: spec.name,
            bin,
            args: spec.acp_args,
            auth: spec.auth,
        }),
        _ => Some(Discovery::NeedsAdapter {
            name: spec.name,
            detail: spec
                .adapter_hint
                .unwrap_or("未找到 ACP 入口")
                .to_owned(),
            package: spec.adapter_npm,
        }),
    }
}

pub fn adapter_npm(name: &str) -> Result<&'static str, String> {
    let spec = SUPPORTED
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("unknown cli {name}"))?;
    spec.adapter_npm
        .ok_or_else(|| format!("{name} 原生支持 ACP，无需安装适配器"))
}

pub async fn install_npm_adapter(package: &str) -> Result<(), String> {
    if !SUPPORTED.iter().any(|s| s.adapter_npm == Some(package)) {
        return Err("未知适配器包".into());
    }
    let npm = find_executable("", &["npm"]).ok_or("未找到 npm")?;
    let out = tokio::process::Command::new(npm)
        .args(["install", "-g", "--no-fund", "--no-audit", package])
        .output()
        .await
        .map_err(|e| format!("启动 npm 失败: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        return Err(if err.is_empty() {
            "npm 安装失败".into()
        } else {
            err.to_owned()
        });
    }
    Ok(())
}

fn search_names(names: &[&str]) -> Option<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".antigravity/antigravity/bin"));
        dirs.push(home.join(".local/share/antigravity/bin"));
        dirs.push(home.join(".local/share/antigravity-acp"));
    }
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/local/bin"));
    for dir in dirs {
        for name in names {
            let candidate = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_names_are_not_discovered() {
        assert!(search_names(&["desk-definitely-missing-cli-xyz"]).is_none());
    }

    #[test]
    fn adapter_npm_is_allowlisted() {
        assert_eq!(
            adapter_npm("codex").unwrap(),
            "@agentclientprotocol/codex-acp"
        );
        assert_eq!(
            adapter_npm("claude").unwrap(),
            "@agentclientprotocol/claude-agent-acp"
        );
        assert!(adapter_npm("gemini").unwrap_err().contains("原生支持"));
        assert!(adapter_npm("nope").is_err());
    }
}
