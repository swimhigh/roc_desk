import React, { useState } from "react";
import { Database, Pencil, Play, Trash2 } from "lucide-react";
import { useSqlStore } from "../../stores/sqlStore";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import { sqlService } from "../../services/sqlService";
import type { DataSourceInput, DbKind } from "../../types/bindings";
import { ConfirmDialog } from "../shared/ConfirmDialog";

const DB_KIND_OPTIONS: { value: DbKind; label: string; defaultPort: number }[] = [
  { value: "postgres", label: "PostgreSQL", defaultPort: 5432 },
  { value: "mysql", label: "MySQL", defaultPort: 3306 },
  { value: "tdsql", label: "TDSQL（MySQL 协议）", defaultPort: 15300 },
  { value: "opengauss", label: "openGauss", defaultPort: 5432 },
  { value: "sql_server", label: "SQL Server", defaultPort: 1433 },
  { value: "oracle", label: "Oracle（暂未实现）", defaultPort: 1521 },
];

const emptyForm: DataSourceInput = {
  name: "",
  db_kind: "postgres",
  host: "",
  port: 5432,
  database_name: "",
  default_schema: "",
  username: "",
  password: "",
  environment: "dev",
  group_name: "",
  readonly: true,
  ssl_required: false,
};

/** 数据源列表 + 新建/编辑向导（docs/SQL_DESKTOP_PLAN.md §3.1、§3.2）。为控制
 * 实现规模合并成一个页面，不单独拆多步骤向导弹窗。 */
export const DataSourceManager: React.FC = () => {
  const dataSources = useSqlStore((s) => s.dataSources);
  const saveDataSource = useSqlStore((s) => s.saveDataSource);
  const deleteDataSource = useSqlStore((s) => s.deleteDataSource);
  const selectDataSource = useSqlStore((s) => s.selectDataSource);
  const push = useToastStore((s) => s.push);

  const [form, setForm] = useState<DataSourceInput>(emptyForm);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  const set = <K extends keyof DataSourceInput>(key: K, v: DataSourceInput[K]) =>
    setForm((cur) => ({ ...cur, [key]: v }));

  const startEdit = (id: string) => {
    const d = dataSources.find((x) => x.id === id);
    if (!d) return;
    setEditingId(id);
    setForm({
      name: d.name,
      db_kind: d.db_kind,
      host: d.host,
      port: d.port,
      database_name: d.database_name,
      default_schema: d.default_schema,
      username: d.username,
      password: "",
      environment: d.environment,
      group_name: d.group_name,
      readonly: d.readonly,
      ssl_required: d.ssl_required,
    });
  };

  const resetForm = () => {
    setEditingId(null);
    setForm(emptyForm);
  };

  const canSave = Boolean(form.name.trim() && form.host.trim() && form.db_kind !== "oracle" && (form.port == null || (Number.isInteger(form.port) && form.port > 0 && form.port <= 65535)));

  const handleTest = async () => {
    setTesting(true);
    try {
      const info = await sqlService.testConnection(editingId, form);
      push("success", `连接成功：${info.version}（延迟 ${info.latency_ms}ms）`);
    } catch (e) {
      push("error", `连接失败：${formatError(e)}`);
    } finally {
      setTesting(false);
    }
  };

  const handleSave = async (open = true) => {
    setSaving(true);
    try {
      const saved = await saveDataSource(editingId, form);
      push("success", editingId ? "已保存修改" : "已新增数据源");
      resetForm();
      if (open) await selectDataSource(saved.id);
    } catch (e) {
      push("error", `保存失败：${formatError(e)}`);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteDataSource(id);
      if (editingId === id) resetForm();
    } catch (e) {
      push("error", `删除失败：${formatError(e)}`);
    }
  };

  return (
    <div style={{ display: "flex", width: "100%", height: "100%", overflow: "hidden" }}>
      <div style={{ width: 320, flexShrink: 0, borderRight: "1px solid var(--border-default)", overflowY: "auto", padding: 12 }}>
        <div style={{ fontSize: 12, color: "var(--text-secondary)", marginBottom: 8 }}>数据源</div>
        {dataSources.length === 0 ? (
          <div className="empty-state" style={{ padding: 16 }}>暂无数据源，右侧新建一个</div>
        ) : (
          dataSources.map((d) => (
            <div key={d.id} className="file-row" style={{ gridTemplateColumns: "1fr auto auto auto", alignItems: "center" }}>
              <span
                onClick={() => void selectDataSource(d.id)}
                title={`${d.name}（${d.db_kind}@${d.host}${d.environment === "prod" ? " · 生产" : ""}）`}
                style={{ cursor: "pointer", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", minWidth: 0 }}
              >
                {d.name}
                <span style={{ color: "var(--text-secondary)", fontSize: 11, marginLeft: 6 }}>
                  {d.db_kind}@{d.host}
                  {d.environment === "prod" ? " · 生产" : ""}
                </span>
              </span>
              <button className="btn ghost sm" onClick={() => void selectDataSource(d.id)} title="打开工作区">
                <Play style={{ width: 14, height: 14 }} />
              </button>
              <button className="btn ghost sm" onClick={() => startEdit(d.id)} title="编辑">
                <Pencil style={{ width: 14, height: 14 }} />
              </button>
              <button className="btn ghost sm" onClick={() => setDeleteTarget(d.id)} title="删除">
                <Trash2 style={{ width: 14, height: 14 }} />
              </button>
            </div>
          ))
        )}
      </div>
      <ConfirmDialog open={!!deleteTarget} severity="danger" title="删除数据源？" onDismiss={() => setDeleteTarget(null)} actions={<><button className="btn ghost sm" onClick={() => setDeleteTarget(null)}>取消</button><button className="btn danger sm" onClick={() => { const id = deleteTarget; setDeleteTarget(null); if (id) void handleDelete(id); }}>删除</button></>}>
        {(() => { const d = dataSources.find((x) => x.id === deleteTarget); return d ? <p>将删除“{d.name}”（{d.host}，{d.environment === "prod" ? "生产" : d.environment}）的本地连接配置和凭据引用。查询草稿不会删除。</p> : null; })()}
      </ConfirmDialog>

      <div style={{ flex: 1, overflowY: "auto", padding: 24, maxWidth: 480 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 16 }}>
          <Database />
          <h2 style={{ margin: 0, fontSize: 16 }}>{editingId ? "编辑数据源" : "新建数据源"}</h2>
        </div>
        <div className="form">
          <div className="form-row">
            <label className="form-label">数据库类型</label>
            <select
              className="form-select"
              value={form.db_kind}
              onChange={(e) => {
                const kind = e.target.value as DbKind;
                const preset = DB_KIND_OPTIONS.find((o) => o.value === kind);
                setForm((cur) => ({ ...cur, db_kind: kind, port: preset?.defaultPort ?? cur.port }));
              }}
            >
              {DB_KIND_OPTIONS.map((o) => (
                <option key={o.value} value={o.value} disabled={o.value === "oracle"}>
                  {o.label}
                </option>
              ))}
            </select>
          </div>
          <div className="form-row">
            <label className="form-label">名称</label>
            <input className="form-input" value={form.name} onChange={(e) => set("name", e.target.value)} placeholder="如：测试环境订单库" />
          </div>
          <div className="form-row" style={{ flexDirection: "row", gap: 8 }}>
            <div style={{ flex: 1 }}>
              <label className="form-label">主机</label>
              <input className="form-input" value={form.host} onChange={(e) => set("host", e.target.value)} placeholder="127.0.0.1" />
            </div>
            <div style={{ width: 100 }}>
              <label className="form-label">端口</label>
              <input
                className="form-input"
                type="number"
                value={form.port ?? ""}
                onChange={(e) => set("port", e.target.value ? Number(e.target.value) : null)}
              />
            </div>
          </div>
          <div className="form-row">
            <label className="form-label">数据库/服务名</label>
            <input
              className="form-input"
              value={form.database_name ?? ""}
              onChange={(e) => set("database_name", e.target.value)}
            />
          </div>
          <div className="form-row">
            <label className="form-label">用户名</label>
            <input className="form-input" value={form.username ?? ""} onChange={(e) => set("username", e.target.value)} />
          </div>
          <div className="form-row">
            <label className="form-label">密码</label>
            <input
              className="form-input"
              type="password"
              value={form.password ?? ""}
              onChange={(e) => set("password", e.target.value)}
              placeholder={editingId ? "留空则沿用已保存的密码" : ""}
            />
          </div>
          <div className="form-row" style={{ flexDirection: "row", gap: 8 }}>
            <div style={{ flex: 1 }}>
              <label className="form-label">环境标签</label>
              <select className="form-select" value={form.environment} onChange={(e) => set("environment", e.target.value)}>
                <option value="dev">开发</option>
                <option value="test">测试</option>
                <option value="prod">生产</option>
              </select>
            </div>
          </div>
          <div className="form-row" style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
            <input type="checkbox" checked={form.readonly} onChange={(e) => set("readonly", e.target.checked)} />
            <label className="form-label" style={{ margin: 0 }}>
              只读模式（拒绝 INSERT/UPDATE/DELETE/DDL）
            </label>
          </div>
          <div className="form-row" style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
            <input type="checkbox" checked={form.ssl_required} onChange={(e) => set("ssl_required", e.target.checked)} />
            <label className="form-label" style={{ margin: 0 }}>
              强制 TLS
            </label>
          </div>
          <div className="form-actions">
            <button className="btn ghost sm" disabled={saving || testing || !canSave} onClick={() => void handleSave(false)}>仅保存</button>
            {editingId && (
              <button className="btn ghost sm" onClick={resetForm} disabled={saving}>
                取消编辑
              </button>
            )}
            <button className="btn ghost sm" onClick={() => void handleTest()} disabled={testing || !canSave}>
              {testing ? "测试中…" : "测试连接"}
            </button>
            <button className="btn primary sm" onClick={() => void handleSave()} disabled={saving || !canSave}>
              {saving ? "保存中…" : editingId ? "保存并打开" : "+ 新建并打开"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
