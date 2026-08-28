import React from "react";
import { Boxes, ExternalLink, Package } from "lucide-react";
import type { JarInfo } from "../../types/bindings";
import { SearchableEntryList } from "./SearchableEntryList";

interface JarInfoPanelProps {
  path: string;
  info: JarInfo | null;
  /** 解析失败时的错误信息（损坏的 ZIP、找不到 manifest 等），仍然给"用系统默认
   * 程序打开"这条退路。 */
  error: string | null;
  onOpenExternally: () => void;
}

const Field: React.FC<{ label: string; value: React.ReactNode }> = ({ label, value }) => (
  <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 120 }}>
    <span style={{ fontSize: 11, color: "var(--text-secondary)", textTransform: "uppercase", letterSpacing: 0.4 }}>{label}</span>
    <span style={{ fontSize: 13, color: "var(--text-primary)", fontFamily: "var(--font-mono)", wordBreak: "break-all" }}>{value}</span>
  </div>
);

/** JAR 包（本质是 ZIP + META-INF/MANIFEST.MF）打开时的展示面板（2026-08-28 需求），
 * 和 `BinaryInfoPanel` 是同一个思路：不再只给"用系统默认程序打开"，而是直接展示
 * manifest 关键属性 + Class-Path 依赖 + 内部条目（`fsops::jar_info`）。 */
export const JarInfoPanel: React.FC<JarInfoPanelProps> = ({ path, info, error, onOpenExternally }) => {
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
        <Package style={{ width: 32, height: 32 }} />
        <div>{fileName} 解析失败{error ? `：${error}` : ""}</div>
        <button className="btn primary sm" onClick={onOpenExternally}>
          用系统默认程序打开
        </button>
      </div>
    );
  }

  const entryPaths = info.entries.map((e) => e.path + (e.is_dir ? "/" : ""));

  return (
    <div style={{ flex: 1, minHeight: 0, overflow: "auto", padding: "var(--space-4)", display: "flex", flexDirection: "column", gap: "var(--space-5)" }}>
      <div style={{ display: "flex", alignItems: "flex-start", gap: "var(--space-3)" }}>
        <Package style={{ width: 28, height: 28, color: "var(--accent)", flexShrink: 0, marginTop: 2 }} />
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          <div style={{ fontSize: 14, fontWeight: 600, color: "var(--text-primary)", wordBreak: "break-all" }}>{fileName}</div>
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>
            JAR 包 · {info.total_entries} 个条目 · {info.class_count} 个 class
          </div>
        </div>
      </div>

      <div style={{ display: "flex", flexWrap: "wrap", gap: "var(--space-5)" }}>
        <Field label="Main-Class" value={info.main_class ?? "无（不是可直接运行的 JAR）"} />
      </div>

      <div>
        <div style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--text-secondary)", marginBottom: "var(--space-2)" }}>
          <Boxes style={{ width: 14, height: 14 }} />
          Class-Path 依赖（{info.class_path.length}）
        </div>
        {info.class_path.length === 0 ? (
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>无——很多 JAR 不用这个属性声明依赖（fat jar 或外部 classpath 管理）</div>
        ) : (
          <div style={{ border: "1px solid var(--border-default)", borderRadius: "var(--radius-md)", overflow: "hidden" }}>
            {info.class_path.map((dep, i) => (
              <div
                key={dep + i}
                style={{
                  padding: "6px var(--space-3)",
                  fontSize: 12,
                  fontFamily: "var(--font-mono)",
                  borderBottom: i === info.class_path.length - 1 ? "none" : "1px solid var(--border-subtle)",
                }}
              >
                {dep}
              </div>
            ))}
          </div>
        )}
      </div>

      {info.manifest.length > 0 && (
        <details>
          <summary style={{ fontSize: 12, color: "var(--text-secondary)", cursor: "pointer", marginBottom: "var(--space-2)" }}>
            Manifest 属性（{info.manifest.length}）
          </summary>
          <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
            <tbody>
              {info.manifest.map((attr, i) => (
                <tr key={attr.key + i} style={{ borderBottom: "1px solid var(--border-subtle)" }}>
                  <td style={{ padding: "4px var(--space-2)", color: "var(--text-secondary)", whiteSpace: "nowrap", verticalAlign: "top" }}>{attr.key}</td>
                  <td style={{ padding: "4px var(--space-2)", fontFamily: "var(--font-mono)", wordBreak: "break-all" }}>{attr.value}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      )}

      {entryPaths.length > 0 && (
        <div>
          <div style={{ fontSize: 12, color: "var(--text-secondary)", marginBottom: "var(--space-2)" }}>包内条目</div>
          <SearchableEntryList
            items={entryPaths}
            hardCapped={info.entries_truncated}
            placeholder="搜索包内条目（类名/路径）…"
            emptyText="无条目"
          />
        </div>
      )}

      <div>
        <button className="btn ghost sm" onClick={onOpenExternally} title="用系统默认程序打开">
          <ExternalLink style={{ width: 14, height: 14 }} /> 用系统程序打开
        </button>
      </div>
    </div>
  );
};
