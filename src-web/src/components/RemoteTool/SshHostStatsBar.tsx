import React, { useEffect, useRef, useState } from "react";
import { Cpu, MemoryStick, ArrowUp, ArrowDown, Clock, HardDrive } from "lucide-react";
import { sshService } from "../../services/sshService";
import { formatError } from "../../utils/error";
import type { HostStats } from "../../types/bindings";

const POLL_INTERVAL_MS = 3000;

function formatBytesRate(bytesPerSec: number): string {
  if (bytesPerSec < 1024) return `${bytesPerSec.toFixed(0)}B/s`;
  if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(1)}KB/s`;
  return `${(bytesPerSec / 1024 / 1024).toFixed(2)}MB/s`;
}

// 之前这里少算了一级单位：`kb / 1024` 算出来其实是 MB，却直接标成了 GB，
// TB 的分界线也错着用了 1024*1024（=1GB 对应的 KB 数，本该是 1TB 对应的
// KB 数），29GB 内存就会被显示成 28.47TB（用户 2026-08-25 反馈截图里的原样）。
function formatKb(kb: number): string {
  const mb = kb / 1024;
  if (mb < 1024) return `${mb.toFixed(0)}MB`;
  const gb = mb / 1024;
  if (gb < 1024) return `${gb.toFixed(1)}GB`;
  return `${(gb / 1024).toFixed(2)}TB`;
}

function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  if (days > 0) return `${days} 天`;
  const hours = Math.floor(seconds / 3600);
  if (hours > 0) return `${hours} 小时`;
  return `${Math.floor(seconds / 60)} 分钟`;
}

interface Rates {
  cpuPercent: number;
  rxRate: number;
  txRate: number;
}

interface SshHostStatsBarProps {
  profileId: string;
  /** 标签不可见时暂停轮询（多个 SSH 标签同时开着，没必要都在后台定时执行探针命令）。*/
  active: boolean;
}

/**
 * SSH 会话下方的资源使用率状态栏（用户 2026-08-25 需求："SSH会话的下面请增加SSH机器
 * 资源使用率相关的信息，定时刷新"，参考 FinalShell 每个会话底部的状态栏）。
 *
 * CPU%/网速需要两次采样的差才有意义——`prevRef` 记住上一次的原始计数器采样，本组件
 * 自己算差，后端 `ssh_host_stats` 只管返回原始值（见 ssh/monitor.rs 顶部注释）。
 */
export const SshHostStatsBar: React.FC<SshHostStatsBarProps> = ({ profileId, active }) => {
  const [stats, setStats] = useState<HostStats | null>(null);
  const [rates, setRates] = useState<Rates | null>(null);
  const [error, setError] = useState<string | null>(null);
  const prevRef = useRef<HostStats | null>(null);

  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    let timer: number | undefined;

    const poll = async () => {
      try {
        const s = await sshService.hostStats(profileId);
        if (cancelled) return;
        setError(null);
        const prev = prevRef.current;
        if (prev) {
          const dTotal = s.cpu_total - prev.cpu_total;
          const dIdle = s.cpu_idle - prev.cpu_idle;
          const dtSeconds = Math.max(0.5, (s.sampled_at_ms - prev.sampled_at_ms) / 1000);
          setRates({
            cpuPercent: dTotal > 0 ? Math.max(0, Math.min(100, 100 * (1 - dIdle / dTotal))) : 0,
            rxRate: Math.max(0, (s.net_rx_bytes - prev.net_rx_bytes) / dtSeconds),
            txRate: Math.max(0, (s.net_tx_bytes - prev.net_tx_bytes) / dtSeconds),
          });
        }
        prevRef.current = s;
        setStats(s);
      } catch (e) {
        if (!cancelled) setError(formatError(e));
      } finally {
        if (!cancelled) timer = window.setTimeout(poll, POLL_INTERVAL_MS);
      }
    };
    void poll();

    return () => {
      cancelled = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [profileId, active]);

  if (error) {
    return <div className="host-stats-bar" style={{ color: "var(--text-disabled)" }}>资源使用率不可用：{error}</div>;
  }
  if (!stats) {
    return <div className="host-stats-bar" style={{ color: "var(--text-disabled)" }}>正在获取资源使用率…</div>;
  }

  const memUsedKb = stats.mem_total_kb - stats.mem_available_kb;
  const memPercent = stats.mem_total_kb > 0 ? (memUsedKb / stats.mem_total_kb) * 100 : 0;

  return (
    <div className="host-stats-bar">
      <div className={`host-stats-item ${rates && rates.cpuPercent >= 90 ? "danger" : rates && rates.cpuPercent >= 70 ? "warn" : ""}`}>
        <Cpu /> CPU <strong>{rates ? `${rates.cpuPercent.toFixed(0)}%` : "—"}</strong>
      </div>
      <div className={`host-stats-item ${memPercent >= 90 ? "danger" : memPercent >= 75 ? "warn" : ""}`}>
        <MemoryStick /> 内存 <strong>{formatKb(memUsedKb)} / {formatKb(stats.mem_total_kb)}</strong>
      </div>
      <div className="host-stats-item">
        <ArrowUp /> <strong>{rates ? formatBytesRate(rates.txRate) : "—"}</strong>
      </div>
      <div className="host-stats-item">
        <ArrowDown /> <strong>{rates ? formatBytesRate(rates.rxRate) : "—"}</strong>
      </div>
      <div className="host-stats-item">
        <Clock /> <strong>{formatUptime(stats.uptime_seconds)}</strong>
      </div>
      {stats.disks.length > 0 && (
        <div className="host-stats-disks">
          <HardDrive style={{ width: 12, height: 12, flexShrink: 0 }} />
          {stats.disks.map((d) => (
            <span key={d.mount} className={`host-stats-disk ${d.used_percent >= 90 ? "danger" : d.used_percent >= 75 ? "warn" : ""}`}>
              <span className="mount">{d.mount}</span>
              <strong>{d.used_percent}%</strong>
            </span>
          ))}
        </div>
      )}
    </div>
  );
};
