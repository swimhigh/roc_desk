use serde::{Deserialize, Serialize};

/// 远程主机资源使用率探针（远程工具模式 SSH 会话状态栏，用户 2026-08-25 需求：
/// "SSH会话的下面请增加SSH机器 资源使用率相关的信息，定时刷新"，参考 FinalShell
/// 每个会话底部的状态栏）。
///
/// 只返回**原始累计计数器**（CPU jiffies、网卡累计字节数），不在后端算 CPU%/网速——
/// 这两个值都需要"两次采样的差"才有意义，把上一次采样存前端（每个状态栏组件自己
/// 拿一个 ref 记住上次的 `HostStats`）比在后端为每个 profile_id 维护一份"上次采样"
/// 状态简单得多：不用管会话断开重连、多个前端同时轮询等状态同步问题，纯函数式。
///
/// 只探测 Linux/Unix 远程主机——和仓库里其它远程能力（`shell_quote`、日志搜索的
/// tail 实现）的假设一致，本来就没考虑过 Windows 远程 shell。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostStats {
    pub hostname: String,
    pub uptime_seconds: f64,
    /// /proc/stat 第一行（聚合所有核心）：user+nice+system+idle+iowait+irq+softirq+steal 之和
    pub cpu_total: u64,
    /// 同一行的 idle+iowait
    pub cpu_idle: u64,
    pub mem_total_kb: u64,
    pub mem_available_kb: u64,
    /// 所有非 lo 网卡累计接收/发送字节数之和
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub disks: Vec<DiskUsage>,
    /// 采样时刻（毫秒 epoch），前端用两次采样的时间差算网速
    pub sampled_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskUsage {
    pub mount: String,
    pub total_kb: u64,
    pub used_kb: u64,
    /// 直接取 `df` 自己算好的 Capacity 列，不用 used/total 现算——避免和 df 自己
    /// 显示的数字对不上（df 的百分比分母是"非 root 预留"的可用空间，不是单纯 total）。
    pub used_percent: u8,
}

/// 探针脚本：一次 `exec` 拿全部指标，不为每个指标单独开 Channel（SSH 每开一个
/// Channel 都有握手开销，定时轮询场景下次数会很密集）。每个 `echo "X:$(...)"`
/// 都用双引号包住命令替换——不加引号的话 shell 的分词会把内部换行也吃掉，
/// 多行输出会被拼成一行空格分隔，`df`/`net/dev` 那种一行一条记录的格式就解析不出来了。
pub const PROBE_SCRIPT: &str = r#"echo "H:$(hostname)"; echo "U:$(cat /proc/uptime)"; echo "C:$(head -1 /proc/stat)"; echo "M:$(grep -E 'MemTotal|MemAvailable' /proc/meminfo)"; echo "N:$(grep -v ' lo:' /proc/net/dev | tail -n +3)"; echo "D:$(df -Pk | tail -n +2)""#;

/// 解析 [`PROBE_SCRIPT`] 的输出。按"行首是不是一个已知 section 前缀"分段——
/// 每个 section 除了第一行带 `X:` 前缀，后续行是该 section 的延续（比如 `D:` 后面
/// 跟着好几行 df 记录），不是每行都重新判断属于谁。
pub fn parse_probe_output(output: &str, sampled_at_ms: u64) -> HostStats {
    let mut hostname = String::new();
    let mut uptime_seconds = 0.0;
    let mut cpu_total = 0u64;
    let mut cpu_idle = 0u64;
    let mut mem_total_kb = 0u64;
    let mut mem_available_kb = 0u64;
    let mut net_rx_bytes = 0u64;
    let mut net_tx_bytes = 0u64;
    let mut disks = Vec::new();

    #[derive(PartialEq, Clone, Copy)]
    enum Section {
        None,
        Uptime,
        Cpu,
        Mem,
        Net,
        Disk,
    }
    let mut section = Section::None;

    for raw_line in output.lines() {
        let line = raw_line.trim_end();
        if line.is_empty() {
            continue;
        }
        let (marker, rest) = if line.len() >= 2 && line.as_bytes()[1] == b':' {
            (line.as_bytes()[0], &line[2..])
        } else {
            (0u8, line)
        };

        match marker {
            b'H' => {
                hostname = rest.trim().to_string();
                section = Section::None;
                continue;
            }
            b'U' => {
                section = Section::Uptime;
            }
            b'C' => {
                section = Section::Cpu;
            }
            b'M' => {
                section = Section::Mem;
            }
            b'N' => {
                section = Section::Net;
            }
            b'D' => {
                section = Section::Disk;
            }
            _ => {}
        }

        match section {
            Section::Uptime => {
                if let Some(first) = rest.split_whitespace().next() {
                    uptime_seconds = first.parse().unwrap_or(0.0);
                }
            }
            Section::Cpu => {
                // "cpu  user nice system idle iowait irq softirq steal ..."
                let fields: Vec<u64> = rest
                    .split_whitespace()
                    .skip(1)
                    .filter_map(|f| f.parse().ok())
                    .collect();
                if fields.len() >= 5 {
                    cpu_total = fields.iter().sum();
                    cpu_idle = fields[3] + fields[4];
                }
            }
            Section::Mem => {
                let mut parts = rest.split_whitespace();
                let label = parts.next().unwrap_or("");
                let value: u64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                if label.starts_with("MemTotal") {
                    mem_total_kb = value;
                } else if label.starts_with("MemAvailable") {
                    mem_available_kb = value;
                }
            }
            Section::Net => {
                // "iface: rx_bytes rx_packets ... tx_bytes tx_packets ..."（16 个数值字段）
                let after_colon = rest.split(':').nth(1).unwrap_or(rest);
                let fields: Vec<u64> = after_colon
                    .split_whitespace()
                    .filter_map(|f| f.parse().ok())
                    .collect();
                if fields.len() >= 9 {
                    net_rx_bytes += fields[0];
                    net_tx_bytes += fields[8];
                }
            }
            Section::Disk => {
                // "Filesystem 1024-blocks Used Available Capacity Mounted-on"
                let fields: Vec<&str> = rest.split_whitespace().collect();
                if fields.len() >= 6 {
                    let total_kb: u64 = fields[1].parse().unwrap_or(0);
                    let used_kb: u64 = fields[2].parse().unwrap_or(0);
                    let used_percent: u8 = fields[4].trim_end_matches('%').parse().unwrap_or(0);
                    let mount = fields[5..].join(" ");
                    // 虚拟文件系统（tmpfs/devtmpfs/overlay 的 docker 层等）不是用户关心的
                    // "磁盘"，噪音大于信息量，和 FinalShell 的默认呈现习惯一致地过滤掉。
                    if total_kb > 0 && !mount.starts_with("/dev") && !mount.starts_with("/sys") && !mount.starts_with("/proc") && !mount.starts_with("/run") {
                        disks.push(DiskUsage { mount, total_kb, used_kb, used_percent });
                    }
                }
            }
            Section::None => {}
        }
    }

    HostStats {
        hostname,
        uptime_seconds,
        cpu_total,
        cpu_idle,
        mem_total_kb,
        mem_available_kb,
        net_rx_bytes,
        net_tx_bytes,
        disks,
        sampled_at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_probe_output() {
        let output = "H:kl46\n\
U:1296000.50 987654.32\n\
C:cpu  123 4 56 7890 12 0 3 0 0 0\n\
M:MemTotal:       533456780 kB\n\
MemAvailable:   401234567 kB\n\
N:eth0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0\n\
docker0: 500 5 0 0 0 0 0 0 600 6 0 0 0 0 0 0\n\
D:/dev/sda1      532676608 122654208 382067712  25% /\n\
tmpfs             1024000         0   1024000   0% /dev/shm\n\
/dev/sda2       1048064   356432    638016  36% /boot\n";
        let stats = parse_probe_output(output, 1_000);
        assert_eq!(stats.hostname, "kl46");
        assert!((stats.uptime_seconds - 1296000.50).abs() < 0.001);
        assert_eq!(stats.cpu_total, 123 + 4 + 56 + 7890 + 12 + 0 + 3 + 0);
        assert_eq!(stats.cpu_idle, 7890 + 12);
        assert_eq!(stats.mem_total_kb, 533456780);
        assert_eq!(stats.mem_available_kb, 401234567);
        assert_eq!(stats.net_rx_bytes, 1000 + 500);
        assert_eq!(stats.net_tx_bytes, 2000 + 600);
        assert_eq!(stats.disks.len(), 2);
        assert_eq!(stats.disks[0].mount, "/");
        assert_eq!(stats.disks[0].used_percent, 25);
        assert_eq!(stats.disks[1].mount, "/boot");
    }
}
