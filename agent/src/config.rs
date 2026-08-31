use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `agent.toml`：与 exe 同目录（AGENT_DESIGN.md §六.2，对齐 REQUIREMENTS.md §3.9
/// "可变内容不打包进 exe、放同级目录"的既有约定）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { listen_addr: "0.0.0.0".to_string(), port: 7879 }
    }
}

/// `allowed_roots` 为空表示不限制（等价当前 SSH 场景下"能连上就能读写这个账户
/// 权限范围内的任何文件"），配置了则所有路径请求必须落在其中一项之内
/// （AGENT_DESIGN.md §五）。`token_hash` 由 `pair` 子命令写入，不存明文令牌。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    pub allowed_roots: Vec<String>,
    pub token_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LimitsConfig {
    pub max_concurrent_connections: usize,
    pub exec_timeout_secs: u32,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self { max_concurrent_connections: 8, exec_timeout_secs: 120 }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self { server: ServerConfig::default(), security: SecurityConfig::default(), limits: LimitsConfig::default() }
    }
}

/// exe 所在目录——所有可变内容（`agent.toml`/`cert.pem`/`key.pem`/审计日志）都落在
/// 这里同级或子目录，不用系统 AppData，便于整个部署目录整体拷贝/迁移。
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_path() -> PathBuf {
    exe_dir().join("agent.toml")
}

impl AgentConfig {
    pub fn load_or_default(path: &Path) -> std::io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        toml::from_str(&text).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        std::fs::write(path, text)
    }

    /// `allowed_roots` 校验用的规范化形式（统一分隔符、去掉末尾斜杠、小写化——
    /// Windows 路径大小写不敏感）。
    pub fn allowed_roots_normalized(&self) -> Vec<String> {
        self.security.allowed_roots.iter().map(|r| normalize_for_compare(r)).collect()
    }
}

pub fn normalize_for_compare(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}
