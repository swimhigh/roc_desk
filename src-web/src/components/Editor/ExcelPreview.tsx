import React, { useState } from "react";
import type { ExcelSheet } from "../../utils/previewFile";

interface ExcelPreviewProps {
  sheets: ExcelSheet[];
}

/** Excel 只读预览（2026-08-28 用户反馈）：多个 sheet 用标签切换，单元格原样展示
 * `xlsx` 解析出的值，不做数字格式化/公式求值这类还原度更高但工作量大得多的事——
 * "能看到数据"和 Excel 本身比起来终究是弱化版预览，不追求完全保真。 */
export const ExcelPreview: React.FC<ExcelPreviewProps> = ({ sheets }) => {
  const [active, setActive] = useState(0);
  const sheet = sheets[active];

  if (!sheet) {
    return (
      <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>这个 Excel 文件没有可显示的工作表。</div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {sheets.length > 1 && (
        <div style={{ display: "flex", gap: 2, padding: "4px 8px", borderBottom: "1px solid var(--border-default)", flexShrink: 0, overflowX: "auto" }}>
          {sheets.map((s, i) => (
            <button
              key={s.name}
              className={`btn ghost sm ${i === active ? "active" : ""}`}
              onClick={() => setActive(i)}
              style={{ flexShrink: 0 }}
            >
              {s.name}
            </button>
          ))}
        </div>
      )}
      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        <table className="excel-preview-table">
          <tbody>
            {sheet.rows.map((row, r) => (
              <tr key={r}>
                {row.map((cell, c) => (
                  <td key={c}>{cell}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
};
