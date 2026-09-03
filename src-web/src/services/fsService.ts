import { invoke } from "@tauri-apps/api/core";
import type { BinaryInfo, FileContent, FileEntry, JarInfo, ReplaceSummary, SearchMode, SearchOptions, WriteOutcome } from "../types/bindings";

export const fsService = {
  listDir(workspaceId: string, path: string): Promise<FileEntry[]> {
    return invoke("fs_list_dir", { workspaceId, path });
  },

  readFile(workspaceId: string, path: string): Promise<FileContent> {
    return invoke("fs_read_file", { workspaceId, path });
  },

  /** 图片/PDF/Word/Excel 预览：返回 base64（不含 data: 前缀），前端自己按扩展名分流。*/
  readBinaryPreview(workspaceId: string, path: string): Promise<string> {
    return invoke("fs_read_binary_preview", { workspaceId, path });
  },

  /** "用系统默认程序打开"——本地工作区开原路径，远程工作区后端会先下载到本地临时目录。*/
  openExternally(workspaceId: string, path: string): Promise<void> {
    return invoke("fs_open_externally", { workspaceId, path });
  },

  /** 旧版二进制 Office 文档（.doc/.xls/.ppt/.pptx）用本机 LibreOffice 临时转成 PDF
   * 预览，返回 base64（不含 data: 前缀）——没装 LibreOffice 时会 reject，调用方需要
   * 兜底展示成"转换失败 + 用系统程序打开"（2026-08-28 用户建议）。*/
  convertLegacyOfficeToPdf(workspaceId: string, path: string): Promise<string> {
    return invoke("fs_convert_legacy_office_to_pdf", { workspaceId, path });
  },

  /** EXE/DLL/SO 等可执行文件的基本信息 + 依赖库列表（2026-08-28 需求）。*/
  inspectBinary(workspaceId: string, path: string): Promise<BinaryInfo> {
    return invoke("fs_inspect_binary", { workspaceId, path });
  },

  /** 没有已知可执行文件扩展名的文件，打开前嗅探开头几个字节判断是不是 ELF/PE/
   * Mach-O（2026-08-28 用户反馈：Linux 下的可执行文件习惯上不带扩展名，之前一律
   * 被当文本打开）。只读一小段，不是整篇。*/
  peekIsBinary(workspaceId: string, path: string): Promise<boolean> {
    return invoke("fs_peek_is_binary", { workspaceId, path });
  },

  /** JAR 包的基本信息（manifest/Main-Class/Class-Path）+ 内部条目列表（2026-08-28 需求）。*/
  inspectJar(workspaceId: string, path: string): Promise<JarInfo> {
    return invoke("fs_inspect_jar", { workspaceId, path });
  },

  writeFile(
    workspaceId: string,
    path: string,
    content: string,
    expectedMtime: number | null,
  ): Promise<WriteOutcome> {
    return invoke("fs_write_file", {
      workspaceId,
      path,
      content,
      expectedMtime,
    });
  },

  /** "Reopen with Encoding"（参考 VS Code）：忽略自动探测，强制按指定编码重新读取。*/
  readFileWithEncoding(workspaceId: string, path: string, encodingLabel: string): Promise<FileContent> {
    return invoke("fs_read_file_with_encoding", { workspaceId, path, encodingLabel });
  },

  /** "Save with Encoding"（参考 VS Code）：按指定编码写盘，而不是固定 UTF-8。*/
  writeFileWithEncoding(
    workspaceId: string,
    path: string,
    content: string,
    encodingLabel: string,
    expectedMtime: number | null,
  ): Promise<WriteOutcome> {
    return invoke("fs_write_file_with_encoding", { workspaceId, path, content, encodingLabel, expectedMtime });
  },

  supportedEncodings(): Promise<string[]> {
    return invoke("fs_supported_encodings");
  },

  deleteFile(workspaceId: string, path: string, isDir: boolean): Promise<void> {
    return invoke("fs_delete", { workspaceId, path, isDir });
  },

  rename(workspaceId: string, from: string, to: string): Promise<void> {
    return invoke("fs_rename", { workspaceId, from, to });
  },

  /** 剪切+粘贴的移动就是换个目标路径调 rename；复制目前只支持文件，见后端 `FileOps::copy` 注释。*/
  copy(workspaceId: string, from: string, to: string, isDir: boolean): Promise<void> {
    return invoke("fs_copy", { workspaceId, from, to, isDir });
  },

  /** Explorer 右键"新建文件夹"。"新建文件"不需要单独命令，直接调 `writeFile(..., "", null)`。*/
  createDir(workspaceId: string, path: string): Promise<void> {
    return invoke("fs_create_dir", { workspaceId, path });
  },

  /** 左侧目录树的全文搜索（参考 VS Code 搜索面板）：不直接返回结果，结果通过
   * `search:file-result`/`search:done`/`search:error` 事件流式推送（2026-08-18
   * 需求："能否一个一个目录搜，搜到一部分先展示一部分"），这里只是触发一次搜索。
   * `scopePath` 为空时搜整个工作区，传了就只搜这个子目录（右键"在此文件夹中搜索"）。*/
  searchStream(
    workspaceId: string,
    requestId: string,
    scopePath: string | null,
    query: string,
    mode: SearchMode,
    options: SearchOptions,
  ): Promise<void> {
    return invoke("fs_search_stream", { workspaceId, requestId, scopePath, query, mode, options });
  },

  /** 手动停止正在跑的搜索（2026-08-29 需求："搜索功能不能停止，需要有停止功能"）。
   * `requestId` 必须是那次搜索发起时用的同一个，避免误停用户之后又发起的新搜索。*/
  cancelSearch(requestId: string): Promise<void> {
    return invoke("fs_search_cancel", { requestId });
  },

  /** 查找并替换全部——paths 是搜索结果里的文件路径，不重新在后端搜一遍。 */
  replace(
    workspaceId: string,
    paths: string[],
    query: string,
    replacement: string,
    options: SearchOptions,
  ): Promise<ReplaceSummary> {
    return invoke("fs_replace", { workspaceId, paths, query, replacement, options });
  },
};

/** "游离文件"（不属于任何工作区，靠拖拽/Ctrl+O/系统文件关联直接打开的单个本地文件）
 * 的读写——和 `fsService` 一一对应，只是不需要 `workspaceId`，命令名对应后端
 * `commands/local_fs.rs` 里的 `local_*` 系列（不经过工作区边界校验，见该文件注释）。*/
export const localFileService = {
  readFile(path: string): Promise<FileContent> {
    return invoke("local_read_file", { path });
  },

  readBinaryPreview(path: string): Promise<string> {
    return invoke("local_read_binary_preview", { path });
  },

  openExternally(path: string): Promise<void> {
    return invoke("local_open_externally", { path });
  },

  convertLegacyOfficeToPdf(path: string): Promise<string> {
    return invoke("local_convert_legacy_office_to_pdf", { path });
  },

  inspectBinary(path: string): Promise<BinaryInfo> {
    return invoke("local_inspect_binary", { path });
  },

  peekIsBinary(path: string): Promise<boolean> {
    return invoke("local_peek_is_binary", { path });
  },

  inspectJar(path: string): Promise<JarInfo> {
    return invoke("local_inspect_jar", { path });
  },

  writeFile(path: string, content: string, expectedMtime: number | null): Promise<WriteOutcome> {
    return invoke("local_write_file", { path, content, expectedMtime });
  },

  readFileWithEncoding(path: string, encodingLabel: string): Promise<FileContent> {
    return invoke("local_read_file_with_encoding", { path, encodingLabel });
  },

  writeFileWithEncoding(
    path: string,
    content: string,
    encodingLabel: string,
    expectedMtime: number | null,
  ): Promise<WriteOutcome> {
    return invoke("local_write_file_with_encoding", { path, content, encodingLabel, expectedMtime });
  },
};
