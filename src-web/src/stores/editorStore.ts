import { create } from "zustand";
import { fsService } from "../services/fsService";

export interface EditorBuffer {
  path: string;
  content: string;
  mtime: number;
  dirty: boolean;
  isPreview: boolean; // 单击=预览态，双击=固定态（UI_DESIGN.md §3.3）
  encoding: string; // 探测到（或用户强制指定）的编码，状态栏展示用（参考 VS Code）
}

export interface SaveConflict {
  path: string;
  currentMtime: number;
  currentPreview: string;
}

interface EditorState {
  buffers: Record<string, EditorBuffer>;
  order: string[];
  activePath: string | null;
  conflict: SaveConflict | null;

  /** 单击文件：复用/替换预览标签，而不是无限开新标签 */
  openPreview: (workspaceId: string, path: string) => Promise<void>;
  /** 双击文件：转为固定标签 */
  pin: (path: string) => void;
  setActive: (path: string) => void;
  updateContent: (path: string, content: string) => void;
  save: (workspaceId: string, path: string) => Promise<void>;
  resolveConflict: (
    workspaceId: string,
    resolution: "overwrite" | "discard",
  ) => Promise<void>;
  close: (path: string) => void;
  /** "Reopen with Encoding"（参考 VS Code）：丢弃当前内容，强制按指定编码重新读取。*/
  reopenWithEncoding: (workspaceId: string, path: string, encodingLabel: string) => Promise<void>;
  /** "Save with Encoding"（参考 VS Code）：按指定编码写盘，而不是固定 UTF-8。*/
  saveWithEncoding: (workspaceId: string, path: string, encodingLabel: string) => Promise<void>;
  reset: () => void;
}

export const useEditorStore = create<EditorState>((set, get) => ({
  buffers: {},
  order: [],
  activePath: null,
  conflict: null,

  openPreview: async (workspaceId, path) => {
    const existing = get().buffers[path];
    if (existing) {
      set({ activePath: path });
      return;
    }

    const { text, mtime, encoding } = await fsService.readFile(workspaceId, path);

    // 单击直接开一个常驻标签，不做"预览态复用/替换上一个标签"那一套——早期版本
    // 参照 VS Code 单击=预览/双击=固定的语义，但对不熟悉这个手势的用户来说，
    // 单击点开另一个文件却把原来那个标签"顶掉"，读出来就是"没法同时开多个文件"
    // （真实反馈，2026-08-18）。多文件同时编辑是更基础的诉求，优先满足它。
    set((s) => {
      const buffers = { ...s.buffers, [path]: { path, content: text, mtime, dirty: false, isPreview: false, encoding } };
      const order = s.order.includes(path) ? s.order : [...s.order, path];
      return { buffers, order, activePath: path };
    });
  },

  pin: (path) => {
    set((s) => {
      const buf = s.buffers[path];
      if (!buf) return s;
      return { buffers: { ...s.buffers, [path]: { ...buf, isPreview: false } } };
    });
  },

  setActive: (path) => set({ activePath: path }),

  updateContent: (path, content) => {
    set((s) => {
      const buf = s.buffers[path];
      if (!buf) return s;
      return { buffers: { ...s.buffers, [path]: { ...buf, content, dirty: true } } };
    });
  },

  save: async (workspaceId, path) => {
    const buf = get().buffers[path];
    if (!buf) return;

    const outcome = await fsService.writeFile(workspaceId, path, buf.content, buf.mtime);

    if (outcome.type === "Conflict") {
      set({
        conflict: {
          path,
          currentMtime: outcome.current_mtime,
          currentPreview: outcome.current_preview,
        },
      });
      return;
    }

    set((s) => ({
      buffers: { ...s.buffers, [path]: { ...buf, mtime: outcome.mtime, dirty: false } },
    }));
  },

  resolveConflict: async (workspaceId, resolution) => {
    const conflict = get().conflict;
    if (!conflict) return;
    set({ conflict: null });

    if (resolution === "discard") {
      // 放弃本地修改，重新读取远端/磁盘当前内容
      const { text, mtime, encoding } = await fsService.readFile(workspaceId, conflict.path);
      set((s) => ({
        buffers: {
          ...s.buffers,
          [conflict.path]: { path: conflict.path, content: text, mtime, dirty: false, isPreview: false, encoding },
        },
      }));
      return;
    }

    // 仍要覆盖：不带 expectedMtime 强制写入
    const buf = get().buffers[conflict.path];
    if (!buf) return;
    const outcome = await fsService.writeFile(workspaceId, conflict.path, buf.content, null);
    if (outcome.type === "Written") {
      set((s) => ({
        buffers: {
          ...s.buffers,
          [conflict.path]: { ...buf, mtime: outcome.mtime, dirty: false },
        },
      }));
    }
  },

  reopenWithEncoding: async (workspaceId, path, encodingLabel) => {
    const { text, mtime, encoding } = await fsService.readFileWithEncoding(workspaceId, path, encodingLabel);
    set((s) => {
      const buf = s.buffers[path];
      if (!buf) return s;
      return { buffers: { ...s.buffers, [path]: { ...buf, content: text, mtime, dirty: false, encoding } } };
    });
  },

  saveWithEncoding: async (workspaceId, path, encodingLabel) => {
    const buf = get().buffers[path];
    if (!buf) return;
    const outcome = await fsService.writeFileWithEncoding(workspaceId, path, buf.content, encodingLabel, buf.mtime);
    if (outcome.type === "Conflict") {
      set({ conflict: { path, currentMtime: outcome.current_mtime, currentPreview: outcome.current_preview } });
      return;
    }
    set((s) => ({
      buffers: { ...s.buffers, [path]: { ...buf, mtime: outcome.mtime, dirty: false, encoding: encodingLabel } },
    }));
  },

  close: (path) => {
    set((s) => {
      const buffers = { ...s.buffers };
      delete buffers[path];
      const order = s.order.filter((p) => p !== path);
      const activePath = s.activePath === path ? order[order.length - 1] ?? null : s.activePath;
      return { buffers, order, activePath };
    });
  },

  reset: () => set({ buffers: {}, order: [], activePath: null, conflict: null }),
}));
