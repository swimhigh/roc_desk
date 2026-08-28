import React from "react";
import { Boxes, Cpu, ExternalLink } from "lucide-react";
import type { BinaryInfo } from "../../types/bindings";
import { formatBytes } from "../../utils/format";
import { SearchableEntryList } from "./SearchableEntryList";

interface BinaryInfoPanelProps {
  path: string;
  info: BinaryInfo | null;
  /** 解析失败时的错误信息（比如损坏的文件、静态库归档），仍然给"用系统默认程序
   * 打开"这条退路，不让用户卡在一个空白 Tab 上。 */
  error: string | null;
  onOpenExternally: () => void;
}

const Field: React.FC<{ label: string; value: React.ReactNode }> = ({ label, value }) => (
  <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 120 }}>
    <span style={{ fontSize: 11, color: "var(--text-secondary)", textTransform: "uppercase", letterSpacing: 0.4 }}>{label}</span>
    <span style={{ fontSize: 13, color: "var(--text-primary)", fontFamily: "var(--font-mono)" }}>{value}</span>
  </div>
);

/** EXE/DLL/SO 等不可编辑的可执行文件打开时的展示面板（2026-08-28 需求）：不像
 * `UnsupportedBinaryPanel` 那样只给"用系统默认程序打开"一条路——这类文件后端能
 * 解析出架构/入口点/依赖库等信息（`fsops::binary_info`），值得直接展示出来。 */
export const BinaryInfoPanel: React.FC<BinaryInfoPanelProps> = ({ path, info, error, onOpenExternally }) => {
  const fileName = path.split(/[\\/]/).pop() ?? path;

  if (!info) {
    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          height: "100%",
          gap: 12,
          color: "var(--text-secondary)",
          fontSize: 13,
        }}
      >
        <Cpu style={{ width: 32, height: 32 }} />
        <div>{fileName} 解析失败{error ? `：${error}` : ""}</div>
        <button className="btn primary sm" onClick={onOpenExternally}>
          用系统默认程序打开
        </button>
      </div>
    );
  }

  return (
    <div style={{ flex: 1, minHeight: 0, overflow: "auto", padding: "var(--space-4)", display: "flex", flexDirection: "column", gap: "var(--space-5)" }}>
      <div style={{ display: "flex", alignItems: "flex-start", gap: "var(--space-3)" }}>
        <Cpu style={{ width: 28, height: 28, color: "var(--accent)", flexShrink: 0, marginTop: 2 }} />
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          <div style={{ fontSize: 14, fontWeight: 600, color: "var(--text-primary)", wordBreak: "break-all" }}>{fileName}</div>
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>
            {info.format} · {info.file_kind}
          </div>
        </div>
      </div>

      <div style={{ display: "flex", flexWrap: "wrap", gap: "var(--space-5)" }}>
        <Field label="架构" value={info.architecture} />
        <Field label="位数" value={info.bitness} />
        {info.entry_point && <Field label="入口点" value={info.entry_point} />}
        {info.subsystem && <Field label="子系统" value={info.subsystem} />}
        {info.timestamp && <Field label="链接时间" value={info.timestamp} />}
      </div>

      <div>
        <div style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--text-secondary)", marginBottom: "var(--space-2)" }}>
          <Boxes style={{ width: 14, height: 14 }} />
          依赖库（{info.dependencies.length}）
        </div>
        {info.dependencies.length === 0 ? (
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>无</div>
        ) : (
          <div style={{ border: "1px solid var(--border-default)", borderRadius: "var(--radius-md)", overflow: "hidden" }}>
            {info.dependencies.map((dep, i) => (
              <div
                key={dep + i}
                style={{
                  padding: "6px var(--space-3)",
                  fontSize: 12,
                  fontFamily: "var(--font-mono)",
                  borderBottom: i === info.dependencies.length - 1 ? "none" : "1px solid var(--border-subtle)",
                }}
              >
                {dep}
              </div>
            ))}
          </div>
        )}
      </div>

      {info.total_exports > 0 && (
        <div>
          <div style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--text-secondary)", marginBottom: "var(--space-2)" }}>
            <Boxes style={{ width: 14, height: 14 }} />
            导出符号
          </div>
          <SearchableEntryList
            items={info.exports}
            hardCapped={info.exports_truncated}
            placeholder="搜索导出符号…"
            emptyText="无导出符号"
          />
        </div>
      )}

      {info.sections.length > 0 && (
        <details>
          <summary style={{ fontSize: 12, color: "var(--text-secondary)", cursor: "pointer", marginBottom: "var(--space-2)" }}>
            节区（{info.sections.length}）
          </summary>
          <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
            <thead>
              <tr style={{ color: "var(--text-secondary)", borderBottom: "1px solid var(--border-default)" }}>
                <th style={{ textAlign: "left", padding: "4px var(--space-2)", fontWeight: 500 }}>名称</th>
                <th style={{ textAlign: "right", padding: "4px var(--space-2)", fontWeight: 500 }}>虚拟大小</th>
                <th style={{ textAlign: "right", padding: "4px var(--space-2)", fontWeight: 500 }}>文件大小</th>
              </tr>
            </thead>
            <tbody>
              {info.sections.map((s, i) => (
                <tr key={s.name + i} style={{ borderBottom: "1px solid var(--border-subtle)" }}>
                  <td style={{ padding: "4px var(--space-2)", fontFamily: "var(--font-mono)" }}>{s.name}</td>
                  <td style={{ padding: "4px var(--space-2)", textAlign: "right", fontFamily: "var(--font-mono)" }}>{formatBytes(s.virtual_size)}</td>
                  <td style={{ padding: "4px var(--space-2)", textAlign: "right", fontFamily: "var(--font-mono)" }}>{formatBytes(s.raw_size)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      )}

      <div>
        <button className="btn ghost sm" onClick={onOpenExternally} title="用系统默认程序打开">
          <ExternalLink style={{ width: 14, height: 14 }} /> 用系统程序打开
        </button>
      </div>
    </div>
  );
};
