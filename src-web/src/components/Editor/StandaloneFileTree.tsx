import React from "react";
import { Folder, FolderOpen, File as FileIcon } from "lucide-react";
import { localFsService } from "../../services/localFsService";
import type { FileEntry } from "../../types/bindings";

export const StandaloneFileTree: React.FC<{ onOpenFile?: (path: string) => void }> = ({ onOpenFile }) => {
  const [roots, setRoots] = React.useState<string[]>([]);
  const [children, setChildren] = React.useState<Record<string, FileEntry[]>>({});
  const [expanded, setExpanded] = React.useState<Set<string>>(new Set());
  const [selected, setSelected] = React.useState<string | null>(null);
  React.useEffect(() => { localFsService.listDrives().then(setRoots).catch(() => {}); }, []);
  const load = async (path: string) => {
    if (!children[path]) { const entries = await localFsService.listDir(path); setChildren((s) => ({ ...s, [path]: entries })); }
  };
  const reveal = async (path: string) => {
    const norm = path.replace(/\\/g, "/");
    const parts = norm.split("/").filter(Boolean);
    let cur = /^[A-Za-z]:$/.test(parts[0] ?? "") ? `${parts.shift()!}/` : "/";
    await load(cur);
    for (const part of parts) { cur = `${cur.replace(/\/$/, "")}/${part}`; setExpanded((s) => new Set(s).add(cur)); await load(cur); }
    setSelected(path);
  };
  React.useEffect(() => { const h = (e: Event) => { const p = (e as CustomEvent<{path?: string}>).detail?.path; if (p) void reveal(p); }; window.addEventListener("roc:reveal-standalone", h); return () => window.removeEventListener("roc:reveal-standalone", h); }, [children]);
  const render = (entries: FileEntry[], depth: number) => entries.map((e) => <React.Fragment key={e.path}><div className={`tree-item ${selected === e.path ? "active" : ""}`} style={{ paddingLeft: 8 + depth * 14 }} onClick={() => { setSelected(e.path); if (e.is_dir) { setExpanded((s) => { const n = new Set(s); n.has(e.path) ? n.delete(e.path) : n.add(e.path); return n; }); void load(e.path); } else onOpenFile?.(e.path); }}>{e.is_dir ? (expanded.has(e.path) ? <FolderOpen className="tree-icon is-dir" /> : <Folder className="tree-icon is-dir" />) : <FileIcon className="tree-icon" />}<span className="tree-name">{e.name}</span></div>{e.is_dir && expanded.has(e.path) && children[e.path] ? render(children[e.path], depth + 1) : null}</React.Fragment>);
  return <div className="project-tree" style={{ width: 240, overflow: "auto" }}>{roots.map((r) => <React.Fragment key={r}><div className="tree-item" style={{ paddingLeft: 8 }} onClick={() => { setExpanded((s) => new Set(s).add(r)); void load(r); }}><Folder className="tree-icon is-dir" /><span className="tree-name">{r}</span></div>{expanded.has(r) && children[r] ? render(children[r], 1) : null}</React.Fragment>)}</div>;
};
