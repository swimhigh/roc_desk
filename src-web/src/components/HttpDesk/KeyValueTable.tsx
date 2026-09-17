import React from "react";
import { Trash2 } from "lucide-react";
import type { KeyValueItem } from "../../types/bindings";

interface KeyValueTableProps {
  items: KeyValueItem[];
  onChange: (items: KeyValueItem[]) => void;
  keyPlaceholder?: string;
  valuePlaceholder?: string;
}

/** Params/Headers/表单字段共用的键值对编辑表格——最后一行永远是空行，一旦用户
 * 往里面填内容就自动补一行新的空行，不需要手动点"添加"按钮
 * （docs/HTTP_DESKTOP_PLAN.md §3.3 里 Postman/Apifox 这类工具的通用交互）。 */
export const KeyValueTable: React.FC<KeyValueTableProps> = ({ items, onChange, keyPlaceholder = "Key", valuePlaceholder = "Value" }) => {
  const rows = items.length === 0 || items[items.length - 1].key !== "" ? [...items, { key: "", value: "", enabled: true }] : items;

  const update = (index: number, patch: Partial<KeyValueItem>) => {
    const next = rows.map((r, i) => (i === index ? { ...r, ...patch } : r));
    onChange(next.filter((r, i) => !(i === next.length - 1 && r.key === "" && r.value === "")));
  };

  const remove = (index: number) => {
    onChange(rows.filter((_, i) => i !== index).filter((r) => r.key !== "" || r.value !== ""));
  };

  return (
    <table style={{ width: "100%", borderCollapse: "collapse" }}>
      <tbody>
        {rows.map((row, i) => (
          <tr key={i}>
            <td style={{ width: 24 }}>
              <input
                type="checkbox"
                checked={row.enabled}
                onChange={(e) => update(i, { enabled: e.target.checked })}
                disabled={row.key === "" && row.value === ""}
              />
            </td>
            <td style={{ padding: "2px 4px" }}>
              <input
                className="form-input"
                style={{ width: "100%" }}
                value={row.key}
                placeholder={keyPlaceholder}
                onChange={(e) => update(i, { key: e.target.value })}
              />
            </td>
            <td style={{ padding: "2px 4px" }}>
              <input
                className="form-input"
                style={{ width: "100%" }}
                value={row.value}
                placeholder={valuePlaceholder}
                onChange={(e) => update(i, { value: e.target.value })}
              />
            </td>
            <td style={{ width: 28 }}>
              {(row.key !== "" || row.value !== "") && (
                <button className="btn ghost sm" onClick={() => remove(i)} title="删除这一行">
                  <Trash2 size={12} />
                </button>
              )}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
};
