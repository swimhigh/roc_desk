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

// 通用日志文件语法高亮（2026-08-18 用户原话："able .log等日志文件能否自动识别列信息，
// 做更多的颜色展示"）——之前 `.log` 固定映射到 plaintext，理由是"结构化高亮属于日志
// 搜索模块的职责"，但那是把"搜索"和"在编辑器里直接看一眼哪几行是 ERROR"混为一谈了，
// 后者纯粹是编辑器可用性问题。不追求解析出某个特定系统的日志格式（不同项目/组件的
// 日志字段千差万别），只识别几类几乎所有日志都有的通用元素：时间戳、日志级别关键字、
// 方括号包起来的标签（PID/TID/模块名/来源文件位置）、引号字符串、key: value 里的 key、
// 数字。级别关键字按语义分了四种颜色（错误/警告/信息/调试），和日志搜索面板里
// `.log-level-tag` 徽标用的是同一套配色（见下面 registerLogTheme 里的十六进制值），
// 保持"同一个概念在应用里到处都是同一个颜色"的一致性。
monaco.languages.register({ id: "log", filenamePatterns: ["*.log"] });
monaco.languages.setMonarchTokensProvider("log", {
  tokenizer: {
    root: [
      // 时间戳（可能带或不带外层方括号）：2026-03-08 19:01:14.559280 / 2026-03-08T19:01:14Z
      [/\[?\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:?\d{2})?\]?/, "log.timestamp"],
      // syslog 风格：Aug 18 10:23:45
      [/\b(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\s+\d{1,2}\s+\d{2}:\d{2}:\d{2}\b/, "log.timestamp"],

      // 方括号包着的级别（"[ WARN]" 这类，必须写在下面的通用方括号规则之前，否则会被
      // 那条规则原样吃掉整个 "[ WARN]" 当成普通标签，级别颜色就套不上了）
      [/\[\s*(FATAL|CRITICAL|PANIC)\s*\]/, "log.level.error"],
      [/\[\s*ERRORS?\s*\]/, "log.level.error"],
      [/\[\s*WARN(ING)?\s*\]/, "log.level.warn"],
      [/\[\s*INFO\s*\]/, "log.level.info"],
      [/\[\s*(DEBUG|TRACE)\s*\]/, "log.level.debug"],
      // 不带方括号、独立出现的级别单词（"level=ERROR"/"ERROR:" 这类格式）
      [/\b(FATAL|CRITICAL|PANIC)\b/, "log.level.error"],
      [/\bERRORS?\b/, "log.level.error"],
      [/\bWARN(ING)?\b/, "log.level.warn"],
      [/\bINFO\b/, "log.level.info"],
      [/\b(DEBUG|TRACE)\b/, "log.level.debug"],

      [/"([^"\\]|\\.)*"/, "string"],
      [/'([^'\\]|\\.)*'/, "string"],

      // key: value / key=value 里的 key——粗粒度的"列"识别，不追求覆盖所有格式
      [/[A-Za-z_][\w.-]*(?=\s*[:=])/, "attribute.name"],

      [/\b\d+(\.\d+)?\b/, "number"],

      // 剩下没被上面规则消费掉的方括号内容：PID#TID、模块名、来源文件:行号 等标签
      [/\[[^\]\n]*\]/, "log.tag"],
    ],
  },
});

/** 深色/浅色两套自定义主题，在 Monaco 内置的 vs-dark/vs 基础上叠加日志 token 的
 * 配色规则（`inherit: true` 保留其它语言原有的高亮不受影响）。颜色取自
 * `styles/theme.css` 里 `--danger`/`--warning`/`--accent`/`--text-secondary` 的
 * 十六进制值——Monaco 主题规则吃不了 CSS 变量，只能手抄一份字面量，两边如果以后
 * 改了配色需要记得一起改（数量不多，没有做成自动同步的必要）。 */
function registerLogTheme(
  name: string,
  base: "vs-dark" | "vs",
  colors: { timestamp: string; error: string; warn: string; info: string; debug: string; tag: string },
) {
  monaco.editor.defineTheme(name, {
    base,
    inherit: true,
    rules: [
      { token: "log.timestamp", foreground: colors.timestamp },
      { token: "log.level.error", foreground: colors.error, fontStyle: "bold" },
      { token: "log.level.warn", foreground: colors.warn, fontStyle: "bold" },
      { token: "log.level.info", foreground: colors.info },
      { token: "log.level.debug", foreground: colors.debug },
      { token: "log.tag", foreground: colors.tag },
    ],
    colors: {},
  });
}
registerLogTheme("roc-dark", "vs-dark", {
  timestamp: "9a9ca1",
  error: "e5534b",
  warn: "d9a441",
  info: "58a6ff",
  debug: "9a9ca1",
  tag: "4f8cff",
});
registerLogTheme("roc-light", "vs", {
  timestamp: "6b6d72",
  error: "cc3b33",
  warn: "b8860b",
  info: "3568e0",
  debug: "6b6d72",
  tag: "3568e0",
});
