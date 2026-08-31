use super::session::{run_local_command, CodingTarget};
use crate::agent::AgentConnectionPool;
use crate::error::AppError;
use crate::ssh::SshConnectionPool;

/// AI 编程助手的 Git 集成（DESIGN.md §3.8.2 "Git 自动提交变更"）。
///
/// 只做"每次 Accept 一条变更就提交一次"这一件事，不做分支管理、不做冲突处理、
/// 不做推送——那些是需要用户主动决策的操作，自动化的边界应该停在"帮你把已经
/// 显式确认过的改动记录进历史"，再往前一步（比如自动 push）风险和收益就不成正比了。
///
/// 远程执行每次 `exec` 都是新开一个 Channel，没有像交互式 Shell 那样的持久 cwd，
/// 所以每条命令都要自己拼 `cd <目录> && git ...`；本地则直接复用
/// `run_local_command` 的 `current_dir` 设置。
/// `args` 是未加引号的原始 git 子命令词——Local/SSH 分支里还是拼一行字符串交给
/// `sh -c`/远端 shell（`crate::log::remote::shell_quote` 按 POSIX 规则转义，和
/// 改动前完全一致的行为，不在这次改动里动它）；Agent 分支直接把 `args` 作为
/// `CreateProcess` 的参数数组传给 `git.exe`，不经过任何 shell 解析，天然没有
/// POSIX 转义规则套在 Windows 目标上失配的问题（AGENT_DESIGN.md §一）。
async fn run_git(target: &CodingTarget, cwd: &str, args: &[&str], ssh_pool: &SshConnectionPool, agent_pool: &AgentConnectionPool) -> Result<String, AppError> {
    match target {
        CodingTarget::Local => {
            let quoted = args.iter().map(|a| crate::log::remote::shell_quote(a)).collect::<Vec<_>>().join(" ");
            run_local_command(&format!("git {quoted}"), cwd).await
        }
        CodingTarget::Remote { connection_id, .. } => {
            let session = ssh_pool.get_or_connect(*connection_id).await?;
            let quoted_cwd = crate::log::remote::shell_quote(cwd);
            let quoted = args.iter().map(|a| crate::log::remote::shell_quote(a)).collect::<Vec<_>>().join(" ");
            session.exec(&format!("cd {quoted_cwd} && git {quoted}")).await
        }
        // Agent 的 `Exec` 请求原生带 `cwd` 字段，不需要像 SSH 那样自己拼
        // `cd <目录> &&`——这正是 AGENT_DESIGN.md §四.4 强调的"命令执行原语原生
        // 按 Windows 语义设计"的一处具体体现。
        CodingTarget::Agent { connection_id, .. } => {
            let session = agent_pool.get_or_connect(*connection_id).await?;
            let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            session.exec_argv("git", &argv, cwd).await
        }
    }
}

/// 探测工作区根目录是否在一个 Git 仓库里——不是仓库（或者目标机器压根没装 git）
/// 时前端应该把"自动提交"开关直接 disable 掉，而不是让用户勾上了才在每次
/// Accept 时才发现根本用不了。
pub async fn is_git_repo(target: &CodingTarget, cwd: &str, ssh_pool: &SshConnectionPool, agent_pool: &AgentConnectionPool) -> bool {
    match run_git(target, cwd, &["rev-parse", "--is-inside-work-tree"], ssh_pool, agent_pool).await {
        Ok(out) => out.trim() == "true",
        Err(_) => false,
    }
}

/// 提交单个文件的改动，返回 `git commit` 的原始输出（成功提交、"nothing to
/// commit"、还是因为没配置 `user.name`/`user.email` 之类的原因失败，都在这段
/// 文本里）。
///
/// 这里不解析这段输出去判断"是否成功"——`SshSession::exec`/`run_local_command`
/// 本身都不透出命令的退出码（只拿到 stdout+stderr 拼在一起的文本），在这个地基上
/// 硬做"成功/失败"的字符串匹配只会是"经常猜对、偶尔猜错还不容易发现"，比如没配置
/// git 身份信息时的报错文本不含"nothing to commit"，会被误判成"提交成功了"。
/// 与其自作聪明，不如把原始输出原样交给调用方展示，用户自己一眼就能看懂 git 的
/// 输出——这是给开发者用的工具，不需要replace 掉他们自己判断的能力。
pub async fn commit_file(
    target: &CodingTarget,
    cwd: &str,
    path: &str,
    message: &str,
    ssh_pool: &SshConnectionPool,
    agent_pool: &AgentConnectionPool,
) -> Result<String, AppError> {
    run_git(target, cwd, &["add", "--", path], ssh_pool, agent_pool).await?;
    run_git(target, cwd, &["commit", "-m", message], ssh_pool, agent_pool).await
}
