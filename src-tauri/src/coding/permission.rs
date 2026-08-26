use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::repo::permission_rules_repo::PermissionRulesRepo;
use crate::error::AppError;

/// 权限规则的判定结果（对齐 OpenCode 的 `allow`/`ask`/`deny` 三态，REQUIREMENTS.md
/// §3.7）。这一层只接管 `run_command`/`webfetch`/`mcp` 三个工具维度，不接管
/// `write_file`/`edit_file`——那两个已经有 Diff-Accept 流程做门禁，等价于"始终 ask"，
/// 不需要再叠一层规则。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Ask => "ask",
            Decision::Deny => "deny",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "allow" => Decision::Allow,
            "deny" => Decision::Deny,
            _ => Decision::Ask,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub id: Uuid,
    /// 固定取值之一：`"run_command"` / `"webfetch"` / `"mcp"`。
    pub tool: String,
    /// 按工具维度含义不同：`run_command` 匹配命令文本本身；`webfetch` 匹配 URL；
    /// `mcp` 匹配 `"<server>:<tool>"`（比如 `filesystem:*` 放行某个服务器的
    /// 全部工具、`filesystem:read_file` 只放行单个工具）。都支持 `*`/`?` 通配符。
    pub pattern: String,
    pub decision: Decision,
    pub enabled: bool,
    pub created_at: String,
}

/// 把 `*`（任意长度）/`?`（单字符）通配符模式转成等价正则做匹配，不引入
/// glob crate——只需要这两个通配符，逐字符转译足够。
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let mut regex_src = String::from("(?s)^");
    for ch in pattern.chars() {
        match ch {
            '*' => regex_src.push_str(".*"),
            '?' => regex_src.push('.'),
            _ => regex_src.push_str(&regex::escape(&ch.to_string())),
        }
    }
    regex_src.push('$');
    regex::Regex::new(&regex_src).map(|re| re.is_match(text)).unwrap_or(false)
}

/// 一次 `send_message` 生命周期内有效的规则快照——每条消息处理前从数据库现取一份
/// （见 `CodingSession::send_message`），规则改动（增删）后下一条消息立即生效，
/// 不需要额外的缓存失效通知机制。
pub struct PermissionEngine {
    rules: Vec<PermissionRule>,
}

impl PermissionEngine {
    pub fn load(repo: &PermissionRulesRepo) -> Result<Self, AppError> {
        Ok(Self { rules: repo.list()? })
    }

    /// 后创建的规则优先命中（`rules` 已经按 `created_at` 升序，这里反向查找），
    /// 让用户新加的规则总能覆盖旧规则，不需要额外的优先级字段。没有任何规则匹配
    /// 时返回 `None`，调用方落回各工具已有的默认策略（白名单/强制确认等）。
    pub fn decide(&self, tool: &str, text: &str) -> Option<Decision> {
        self.rules
            .iter()
            .rev()
            .find(|rule| rule.enabled && rule.tool == tool && wildcard_match(&rule.pattern, text))
            .map(|rule| rule.decision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matches_prefix_pattern() {
        assert!(wildcard_match("git *", "git status"));
        assert!(wildcard_match("git *", "git push origin main"));
        assert!(!wildcard_match("git *", "npm install"));
    }

    #[test]
    fn wildcard_matches_exact_and_question_mark() {
        assert!(wildcard_match("ls", "ls"));
        assert!(!wildcard_match("ls", "ls -la"));
        assert!(wildcard_match("rm f??.txt", "rm foo.txt"));
    }

    fn rule(tool: &str, pattern: &str, decision: Decision, created_at: &str) -> PermissionRule {
        PermissionRule { id: Uuid::new_v4(), tool: tool.into(), pattern: pattern.into(), decision, enabled: true, created_at: created_at.into() }
    }

    #[test]
    fn later_rule_wins_over_earlier_conflicting_rule() {
        let engine = PermissionEngine {
            rules: vec![
                rule("run_command", "git *", Decision::Allow, "2026-01-01"),
                rule("run_command", "git push *", Decision::Ask, "2026-01-02"),
            ],
        };
        assert_eq!(engine.decide("run_command", "git push origin main"), Some(Decision::Ask));
        assert_eq!(engine.decide("run_command", "git status"), Some(Decision::Allow));
    }

    #[test]
    fn no_match_returns_none() {
        let engine = PermissionEngine { rules: vec![rule("run_command", "git *", Decision::Allow, "2026-01-01")] };
        assert_eq!(engine.decide("run_command", "npm install"), None);
    }

    #[test]
    fn disabled_rule_is_ignored() {
        let mut disabled = rule("run_command", "git *", Decision::Allow, "2026-01-01");
        disabled.enabled = false;
        let engine = PermissionEngine { rules: vec![disabled] };
        assert_eq!(engine.decide("run_command", "git status"), None);
    }
}
