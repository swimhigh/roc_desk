use super::session::{run_local_command, CodingTarget};
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
async fn run_git(target: &CodingTarget, cwd: &str, args: &str, ssh_pool: &SshConnectionPool) -> Result<String, AppError> {
    let cmd = format!("git {args}");
    match target {
        CodingTarget::Local => run_local_command(&cmd, cwd).await,
        CodingTarget::Remote { connection_id, .. } => {
            let session = ssh_pool.get_or_connect(*connection_id).await?;
            let quoted_cwd = crate::log::remote::shell_quote(cwd);
            session.exec(&format!("cd {quoted_cwd} && {cmd}")).await
        }
    }
}

/// 探测工作区根目录是否在一个 Git 仓库里——不是仓库（或者目标机器压根没装 git）
/// 时前端应该把"自动提交"开关直接 disable 掉，而不是让用户勾上了才在每次
/// Accept 时才发现根本用不了。
pub async fn is_git_repo(target: &CodingTarget, cwd: &str, ssh_pool: &SshConnectionPool) -> bool {
    match run_git(target, cwd, "rev-parse --is-inside-work-tree", ssh_pool).await {
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
) -> Result<String, AppError> {
    let quoted_path = crate::log::remote::shell_quote(path);
    run_git(target, cwd, &format!("add -- {quoted_path}"), ssh_pool).await?;

    let quoted_message = crate::log::remote::shell_quote(message);
    run_git(target, cwd, &format!("commit -m {quoted_message}"), ssh_pool).await
}
