// Monaco Editor 离线自举配置（DESIGN.md §2.2 选定的编辑器内核）。
//
// `@monaco-editor/react` 默认从 CDN（jsdelivr）拉取 Monaco 资源，这对一个要离线运行的
// 桌面应用不可接受；这里改为用本地打包的 `monaco-editor` 包，worker 通过 Vite 的
// `?worker` 后缀原生支持（不需要额外的 vite 插件）。main.tsx 里在渲染 App 之前
// import 这个文件一次即可，之后所有 <Editor> 组件都会用本地资源。
import { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor";

import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import jsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker";
import cssWorker from "monaco-editor/esm/vs/language/css/css.worker?worker";
import htmlWorker from "monaco-editor/esm/vs/language/html/html.worker?worker";
import tsWorker from "monaco-editor/esm/vs/language/typescript/ts.worker?worker";

self.MonacoEnvironment = {
  getWorker(_workerId: string, label: string) {
    switch (label) {
      case "json":
        return new jsonWorker();
      case "css":
      case "scss":
      case "less":
        return new cssWorker();
      case "html":
      case "handlebars":
      case "razor":
        return new htmlWorker();
      case "typescript":
      case "javascript":
        return new tsWorker();
      default:
        return new editorWorker();
    }
  },
};

loader.config({ monaco });
