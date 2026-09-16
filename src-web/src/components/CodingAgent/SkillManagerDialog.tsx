import React, { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Trash2, FolderInput, FileArchive } from "lucide-react";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { useCodingStore } from "../../stores/codingStore";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";

interface SkillManagerDialogProps {
  workspaceId: string;
  onClose: () => void;
}

/**
 * 项目 Skills 查看/导入：技能本体是工作区里 `.rock_desk/skills/<name>/SKILL.md`
 * （`coding::skills` 模块），此前只有后端发现+运行时按需加载，没有图形化入口——
 * 用户只能手动在工作区里建目录，2026-09 反馈"看不到导入的地方"补上这个弹窗。
 * "导入"选中的是本地磁盘上一个含 `SKILL.md` 的技能目录、或者一个 `.zip`/
 * `.tar.gz`/`.tgz` 压缩包（2026-09 用户需求：技能包大多是压缩包分发，不该逼
 * 用户自己先手动解压——后端 `skill_import` 会自动解压到临时目录再导入，用完
 * 即删）。RAR 不支持（没有靠谱的纯 Rust 解压库，Skills 包也从不用 RAR 分发）。
 * 整体拷贝进当前工作区（远程/Agent 工作区也一样——后端用 `copy_between` 跨端
 * 拷贝，不要求本地和工作区在同一台机器）。
 */
export const SkillManagerDialog: React.FC<SkillManagerDialogProps> = ({ workspaceId, onClose }) => {
  const { skills, loadSkills, importSkill, deleteSkill } = useCodingStore();
  const [importing, setImporting] = useState(false);
  const push = useToastStore((s) => s.push);

  useEffect(() => {
    loadSkills(workspaceId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  const runImport = async (selected: string | string[] | null) => {
    if (!selected || Array.isArray(selected)) return;
    setImporting(true);
    try {
      const skill = await importSkill(workspaceId, selected);
      push("success", `已导入技能：${skill.name}`);
    } catch (e) {
      push("error", `导入失败：${formatError(e)}`);
    } finally {
      setImporting(false);
    }
  };

  const handleImportFolder = async () => runImport(await open({ directory: true, multiple: false }));

  // 原生文件对话框的 filters 只按最后一段扩展名匹配，写 "gz" 而不是 "tar.gz"
  // 才能同时收到 .tar.gz 和 .tgz；zip 单独一条更直观，两条 filter 在对话框里
  // 都会出现，用户可以切换看。
  const handleImportArchive = async () =>
    runImport(
      await open({
        directory: false,
        multiple: false,
        filters: [
          { name: "Skills 压缩包 (.zip)", extensions: ["zip"] },
          { name: "Skills 压缩包 (.tar.gz/.tgz)", extensions: ["gz", "tgz"] },
        ],
      }),
    );

  const handleDelete = async (name: string) => {
    try {
      await deleteSkill(workspaceId, name);
      push("success", `已删除技能：${name}`);
    } catch (e) {
      push("error", `删除失败：${formatError(e)}`);
    }
  };

  return (
    <ConfirmDialog
      open
      severity="info"
      icon="🧩"
      title="项目 Skills 管理"
      onDismiss={onClose}
      actions={<button className="btn ghost sm" onClick={onClose}>关闭</button>}
    >
      <p style={{ fontSize: 12, color: "var(--text-secondary)", marginBottom: 10 }}>
        技能存放在工作区 <code>.rock_desk/skills/&lt;name&gt;/SKILL.md</code>，AI 助手会自动发现并在需要时按名加载详细步骤执行；
        可以选一个已解压的文件夹，也可以直接选 .zip/.tar.gz/.tgz 压缩包（自动解压导入，不支持 RAR）。远程/Agent 工作区会拷贝到远端。同名技能重新导入会覆盖旧版本。
      </p>
      <div style={{ maxHeight: 220, overflowY: "auto", marginBottom: 12 }}>
        {skills.length === 0 ? (
          <p style={{ fontSize: 13, color: "var(--text-secondary)" }}>当前工作区还没有技能，点下方"导入文件夹/压缩包"添加一个。</p>
        ) : (
          skills.map((skill) => (
            <div key={skill.name} className="file-row" style={{ gridTemplateColumns: "1fr auto" }}>
              <span>
                {skill.name}
                {skill.description && (
                  <span style={{ color: "var(--text-secondary)", fontSize: 11, marginLeft: 6 }}>
                    {skill.description}
                  </span>
                )}
              </span>
              <button className="btn ghost sm" onClick={() => handleDelete(skill.name)} title="删除">
                <Trash2 style={{ width: 14, height: 14 }} />
              </button>
            </div>
          ))
        )}
      </div>

      <div className="form-actions">
        <button className="btn primary sm" onClick={handleImportFolder} disabled={importing}>
          <FolderInput style={{ width: 14, height: 14 }} /> {importing ? "导入中…" : "导入文件夹"}
        </button>
        <button className="btn ghost sm" onClick={handleImportArchive} disabled={importing}>
          <FileArchive style={{ width: 14, height: 14 }} /> {importing ? "导入中…" : "导入压缩包"}
        </button>
      </div>
    </ConfirmDialog>
  );
};
