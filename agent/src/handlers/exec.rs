//! 一次性命令执行（AGENT_DESIGN.md §2.1/§四.4）：`command + args` 数组直接对应
//! Windows `CreateProcess`/`std::process::Command` 的参数数组语义，不走"拼一行
//! shell 字符串再转义"这条容易出错的路——这是相对 SSH `exec()` 的关键差异
//! （SSH 那边只能传一整行给远端 shell 解析，这里天然避免了引号/转义规则的问题）。

use std::time::Duration;

use roc_desk_protocol::ErrorCode;

pub async fn exec(command: &str, args: &[String], cwd: &str, timeout_secs: u32) -> Result<(Option<i32>, String), (ErrorCode, String)> {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args);
    if !cwd.is_empty() {
        cmd.current_dir(cwd);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    // 超时被打断时，`timeout()` 会丢弃还持有 `Child` 所有权的 future——`kill_on_drop`
    // 让 tokio 在那一刻尽力把子进程杀掉，不留孤儿进程挂在远程主机上。
    cmd.kill_on_drop(true);

    let child = cmd.spawn().map_err(|e| (ErrorCode::Internal, format!("启动进程失败: {e}")))?;
    let timeout = Duration::from_secs(if timeout_secs == 0 { 120 } else { timeout_secs as u64 });

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            Ok((output.status.code(), text))
        }
        Ok(Err(e)) => Err((ErrorCode::Internal, format!("执行命令失败: {e}"))),
        Err(_) => Err((ErrorCode::Internal, format!("命令执行超时（{}s）", timeout.as_secs()))),
    }
}
