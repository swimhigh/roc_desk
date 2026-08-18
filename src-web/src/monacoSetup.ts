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

// Monaco 自带的 basic-languages 列表里没有 makefile（确认过 node_modules 里的语言目录，
// 只有 shell/dockerfile 等，makefile 不在其中），只能自己注册一个 Monarch tokenizer——
// 不追求覆盖 GNU Make 全部语法细节（比如 `$(shell ...)` 之类函数调用内部的精细分词），
// 覆盖到"变量/目标/指令/注释/Recipe 命令行"这几个最常见、最需要用颜色区分的元素就够用。
monaco.languages.register({
  id: "makefile",
  filenamePatterns: ["Makefile", "makefile", "GNUmakefile", "*.mk", "*.mak"],
});
monaco.languages.setMonarchTokensProvider("makefile", {
  tokenizer: {
    root: [
      // Recipe 命令行：Tab 缩进，整行按字符串色处理（不做二次分词，足够和普通语句区分开）
      [/^\t.*$/, "string"],
      [/#.*$/, "comment"],
      // 自动变量 $@ $< $^ $? $* 等
      [/\$[@<^?*+%]/, "variable.predefined"],
      // 变量引用 $(VAR) / ${VAR}，不处理嵌套函数调用内部结构
      [/\$[({][^)}]*[)}]/, "variable"],
      [/^\s*\.(PHONY|SUFFIXES|DEFAULT|PRECIOUS|INTERMEDIATE|SECONDARY|DELETE_ON_ERROR|IGNORE|EXPORT_ALL_VARIABLES|NOTPARALLEL|ONESHELL)\b/, "keyword"],
      [/^\s*(ifeq|ifneq|ifdef|ifndef|else|endif|define|endef|include|-include|sinclude|export|unexport|override|vpath|undefine)\b/, "keyword"],
      // 变量赋值：VAR = / := / ::= / ?= / +=
      [/^[\w.][\w.-]*(?=[ \t]*:{0,2}[+?]?=)/, "variable.name"],
      [/::?=|[+?]=|=/, "operator"],
      // 目标规则：target(s): prerequisites（排除 := 这一路赋值写法）
      [/^[^\s#:][^:#]*(?=:(?!=))/, "type.identifier"],
      [/:/, "delimiter"],
    ],
  },
});
