use std::sync::LazyLock;

use regex::Regex;

/// 破坏性命令黑名单（DESIGN.md §3.8.2.1）：命中即硬拦截，不提供"仍要执行"的绕过——
/// 只允许用户去终端模块手动敲。模式尽量宽松匹配变体（多空格、`sudo` 前缀等），
/// 漏报比误报危害更大，但也不追求覆盖所有可能的破坏性命令——这是纵深防御的一层，
/// 不是唯一防线（终端本身仍然不受限）。
static BLACKLIST: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"rm\s+(-\w*r\w*f\w*|-\w*f\w*r\w*)\s+/(\s|$)").unwrap(),
        Regex::new(r"rm\s+(-\w*r\w*f\w*|-\w*f\w*r\w*)\s+/\*").unwrap(),
        Regex::new(r"\bmkfs(\.\w+)?\b").unwrap(),
        Regex::new(r"\bdd\s+.*of=/dev/").unwrap(),
        Regex::new(r":\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:").unwrap(), // fork bomb
        Regex::new(r">\s*/etc/passwd\b").unwrap(),
        Regex::new(r">\s*/etc/shadow\b").unwrap(),
        Regex::new(r"\bshutdown\b|\breboot\b|\bhalt\b").unwrap(),
        Regex::new(r"chmod\s+-R\s+000\s+/(\s|$)").unwrap(),
    ]
});

/// 只读白名单（DESIGN.md §3.8.2.1）：用户可选择让这些命令自动放行，减少高频确认打断。
/// 只匹配命令的第一个词，不代表"这条命令一定安全"（比如 `git status && rm -rf /`
/// 会被 `&&`/`;` 拆开逐条检查——见 `is_whitelisted` 的实现）。
static READONLY_PREFIXES: &[&str] = &["ls", "cat", "grep", "git status", "git log", "git diff", "pwd", "whoami", "echo", "head", "tail", "find", "which", "ps"];

pub fn is_blacklisted(command: &str) -> bool {
    BLACKLIST.iter().any(|re| re.is_match(command))
}

/// 白名单判断按 `&&`/`;`/`|` 拆分每个子命令，要求全部命中只读前缀才放行——
/// 防止 `git status && rm -rf /important` 这类拼接绕过。
pub fn is_whitelisted(command: &str) -> bool {
    command
        .split(&['&', ';', '|'][..])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .all(|part| READONLY_PREFIXES.iter().any(|prefix| part == *prefix || part.starts_with(&format!("{prefix} "))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_rm_rf_root() {
        assert!(is_blacklisted("rm -rf /"));
        assert!(is_blacklisted("sudo rm -fr /"));
    }

    #[test]
    fn does_not_block_normal_rm() {
        assert!(!is_blacklisted("rm -rf ./build"));
        assert!(!is_blacklisted("rm -rf /home/user/tmp"));
    }

    #[test]
    fn blocks_fork_bomb() {
        assert!(is_blacklisted(":(){ :|:& };:"));
    }

    #[test]
    fn whitelist_allows_plain_readonly() {
        assert!(is_whitelisted("git status"));
        assert!(is_whitelisted("ls -la /var/log"));
    }

    #[test]
    fn whitelist_rejects_chained_bypass() {
        assert!(!is_whitelisted("git status && rm -rf /important"));
    }
}
