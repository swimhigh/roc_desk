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

/// Windows 专属危险命令黑名单（AGENT_DESIGN.md §四.4）：上面那份黑名单是照着
/// `rm -rf /` 这类 POSIX 命令写的模式，直接套在 Windows Agent 目标上完全不会命中——
/// `Remove-Item -Recurse -Force C:\`、`format`、关机/重启、防火墙规则改动这些
/// Windows 特有的破坏性操作需要单独一份模式表，和上面那份并列生效（不是替换）。
static WINDOWS_BLACKLIST: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r#"(?i)remove-item\s+.*-recurse\b.*c:\\?['"]?(\s|$)"#).unwrap(),
        Regex::new(r"(?i)\bformat\s+[a-z]:").unwrap(),
        Regex::new(r"(?i)\bdel\s+/[sS]\s+/[qQ]\b.*\\\s*$").unwrap(),
        Regex::new(r"(?i)\brd\s+/[sS]\s+/[qQ]\b.*\\\s*$").unwrap(),
        Regex::new(r"(?i)\bvssadmin\s+delete\b").unwrap(),
        Regex::new(r"(?i)\breg\s+delete\s+hklm\b").unwrap(),
        Regex::new(r"(?i)\bbcdedit\b").unwrap(),
        Regex::new(r"(?i)\bstop-computer\b|\brestart-computer\b|\bshutdown\s+/[rs]\b").unwrap(),
        Regex::new(r"(?i)\bdiskpart\b").unwrap(),
        Regex::new(r"(?i)netsh\s+advfirewall\s+set\s+allprofiles\s+state\s+off").unwrap(),
    ]
});

/// 只读白名单（DESIGN.md §3.8.2.1）：用户可选择让这些命令自动放行，减少高频确认打断。
/// 只匹配命令的第一个词，不代表"这条命令一定安全"（比如 `git status && rm -rf /`
/// 会被 `&&`/`;` 拆开逐条检查——见 `is_whitelisted` 的实现）。
static READONLY_PREFIXES: &[&str] = &["ls", "cat", "grep", "git status", "git log", "git diff", "pwd", "whoami", "echo", "head", "tail", "find", "which", "ps"];

/// `is_windows_target` 为 true（`CodingTarget::Agent`）时额外叠加
/// `WINDOWS_BLACKLIST`——两份模式表并集生效，不是二选一：Windows 目标上用户仍然
/// 可能敲出 `rm -rf /` 风格的命令（比如装了 Git Bash），照样应该拦截。
pub fn is_blacklisted(command: &str, is_windows_target: bool) -> bool {
    BLACKLIST.iter().any(|re| re.is_match(command)) || (is_windows_target && WINDOWS_BLACKLIST.iter().any(|re| re.is_match(command)))
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
        assert!(is_blacklisted("rm -rf /", false));
        assert!(is_blacklisted("sudo rm -fr /", false));
    }

    #[test]
    fn does_not_block_normal_rm() {
        assert!(!is_blacklisted("rm -rf ./build", false));
        assert!(!is_blacklisted("rm -rf /home/user/tmp", false));
    }

    #[test]
    fn blocks_fork_bomb() {
        assert!(is_blacklisted(":(){ :|:& };:", false));
    }

    #[test]
    fn blocks_windows_danger_commands_only_for_agent_target() {
        assert!(is_blacklisted("format c:", true));
        assert!(is_blacklisted("vssadmin delete shadows /all", true));
        assert!(is_blacklisted("Restart-Computer -Force", true));
        // 同样的命令在非 Windows 目标（本地/SSH）上不应该被这份专属列表拦截——
        // 那边压根不会真的跑出这些 Windows 命令，但即便跑了也不是这份表要管的范围。
        assert!(!is_blacklisted("format c:", false));
    }

    #[test]
    fn windows_target_still_blocks_posix_blacklist() {
        assert!(is_blacklisted("rm -rf /", true));
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
