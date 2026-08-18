use std::time::Duration;

/// 指数退避重连策略（DESIGN.md §3.2.3）：1s → 2s → 4s → 8s，封顶 30s。
///
/// 目前只提供退避序列生成，尚未接入 `SshConnectionPool` 的自动重连触发——
/// 那需要在 `session.rs` 的 Channel 读取循环检测到 EOF/Close 时回调到这里，
/// 是 §十-开放风险 里"多工作区并行状态"定案后再接的下一步。
pub struct Backoff {
    attempt: u32,
    max_attempts: u32,
}

impl Backoff {
    pub fn new(max_attempts: u32) -> Self {
        Self { attempt: 0, max_attempts }
    }

    /// 返回下一次重试前应等待的时长；`None` 表示已达最大重试次数。
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.attempt >= self.max_attempts {
            return None;
        }
        let secs = 1u64 << self.attempt.min(4); // 1,2,4,8,16 → 封顶在下面 clamp 到 30
        self.attempt += 1;
        Some(Duration::from_secs(secs.min(30)))
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_sequence_caps_at_30s_and_respects_max_attempts() {
        let mut b = Backoff::new(3);
        assert_eq!(b.next_delay(), Some(Duration::from_secs(1)));
        assert_eq!(b.next_delay(), Some(Duration::from_secs(2)));
        assert_eq!(b.next_delay(), Some(Duration::from_secs(4)));
        assert_eq!(b.next_delay(), None);
    }
}
