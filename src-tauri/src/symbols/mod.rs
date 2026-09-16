use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::fsops::{is_excluded_dir, FileOps};

/// 单次索引构建最多扫描这么多个文件——只是防止极端情况下（超大仓库/误把整个磁盘
/// 当工作区打开）扫描无限跑下去，不是常规使用场景会撞到的上限。
const MAX_INDEX_FILES: usize = 20_000;
/// 单文件超过这个大小就跳过不解析——生成的大体积源文件（打包产物、序列化数据）
/// 塞进正则扫描没有意义，还拖慢整体建索引速度。
const MAX_INDEX_FILE_BYTES: u64 = 2 * 1024 * 1024;

const SOURCE_EXTENSIONS: &[&str] = &[
    "c", "h", "cpp", "cc", "cxx", "hpp", "hh", "hxx", "ino", "rs", "py", "go", "js", "jsx", "ts",
    "tsx", "mjs", "cjs",
];

/// C/C++ 常见的控制流/语句关键字，两个用途：一是函数签名正则会把 `if (...)`/
/// `while (...)` 这类语句误当成"名字叫 if 的函数"命中（关键字出现在"名字"位置时
/// 排除）；二是排除 `return foo();`/`throw foo();` 这类语句被误当成定义了一个
/// 叫 `foo` 的函数（关键字出现在"名字前面那段前缀"位置时排除，见 extract_symbols）。
const CONTROL_KEYWORDS: &[&str] = &[
    "if", "for", "while", "switch", "return", "sizeof", "catch", "else", "do", "defined",
    "static_assert", "__attribute__", "throw", "goto", "delete", "case",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolLocation {
    pub path: String,
    /// 1-based 行号，和 Monaco Range 的行号约定一致。
    pub line: u32,
    /// "function"/"type"/"macro"/"typedef" 等，纯展示用，前端目前没细分处理。
    pub kind: String,
}

/// 工作区符号索引（"转到定义/声明"，2026-09-16 需求）。不是真正的语言语义分析
/// （不理解宏展开、重载、条件编译分支），只是逐行正则扫描常见的函数/类型/宏定义
/// 写法——ctags 的思路，但自己写、不依赖外部二进制：一是本机没装 universal-ctags
/// 且不想强迫用户单独安装或者把二进制打进安装包；二是这样能直接复用 `FileOps`
/// trait，本地/SSH/Agent 三种工作区不用区分就天然都支持（只要能读到文件内容），
/// 不需要像真正的 ctags/LSP 那样操心"可执行文件怎么发到远程主机上跑"。
///
/// 精度上的已知取舍：正则匹配不理解语言语义，多行函数签名、宏生成的定义、
/// 复杂的函数指针 typedef 这类写法会漏掉；符号名冲突（重载、不同文件同名类型）
/// 不去重，全部返回给前端，由 Monaco 的"多结果"UI（peek/quick pick）让用户自己选。
#[derive(Default)]
pub struct SymbolIndex {
    table: HashMap<String, Vec<SymbolLocation>>,
    /// path -> 该文件贡献过的符号名列表，重新索引单个文件时用它反查要摘掉哪些旧
    /// 条目，不用整表扫一遍。
    by_file: HashMap<String, Vec<String>>,
}

impl SymbolIndex {
    pub fn lookup(&self, name: &str) -> Vec<SymbolLocation> {
        self.table.get(name).cloned().unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.table.values().map(|v| v.len()).sum()
    }

    fn remove_file(&mut self, path: &str) {
        if let Some(names) = self.by_file.remove(path) {
            for name in names {
                if let Some(locs) = self.table.get_mut(&name) {
                    locs.retain(|l| l.path != path);
                    if locs.is_empty() {
                        self.table.remove(&name);
                    }
                }
            }
        }
    }

    /// 增量重建单个文件的条目——文件保存后调用，不用重跑整个工作区的索引。
    pub fn index_file(&mut self, path: &str, content: &str) {
        self.remove_file(path);
        let symbols = extract_symbols(path, content);
        if symbols.is_empty() {
            return;
        }
        let mut names = Vec::with_capacity(symbols.len());
        for (name, line, kind) in symbols {
            self.table
                .entry(name.clone())
                .or_default()
                .push(SymbolLocation {
                    path: path.to_string(),
                    line,
                    kind: kind.to_string(),
                });
            names.push(name);
        }
        self.by_file.insert(path.to_string(), names);
    }
}

fn is_source_file(name: &str) -> bool {
    name.rsplit_once('.')
        .map(|(_, ext)| SOURCE_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// 遍历整个工作区、逐个源码文件解析符号，构建一份全新的索引（"打开工作区"/手动
/// 触发重建时调用）。复用 `FileOps::list_dir`/`read_file_raw` 这两个和
/// `fsops::search_stream` 同样的基础原语，本地走 std::fs 很快，远程（SSH/Agent）
/// 每个文件是一次独立的往返，工作区文件数很大时会明显慢——这是和
/// `fsops::search_stream` 完全一样的已知取舍，真要提速得让扫描跑在远程主机本身
/// 上，属于后续按需再做的优化，不在这版范围内。
pub async fn build_index(file_ops: &dyn FileOps, root: &str) -> Result<SymbolIndex, AppError> {
    let mut index = SymbolIndex::default();
    let mut stack = vec![root.to_string()];
    let mut scanned = 0usize;

    while let Some(dir) = stack.pop() {
        let entries = match file_ops.list_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries {
            if entry.is_dir {
                if !is_excluded_dir(&entry.name) {
                    stack.push(entry.path.clone());
                }
                continue;
            }
            if !is_source_file(&entry.name) {
                continue;
            }
            if entry.size.map(|s| s > MAX_INDEX_FILE_BYTES).unwrap_or(false) {
                continue;
            }
            let Ok((bytes, _mtime)) = file_ops.read_file_raw(&entry.path).await else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            index.index_file(&entry.path, &text);

            scanned += 1;
            if scanned >= MAX_INDEX_FILES {
                return Ok(index);
            }
        }
    }

    Ok(index)
}

struct LangRules {
    patterns: Vec<(Regex, &'static str)>,
}

macro_rules! lang_rules {
    ($fn_name:ident, $cell:ident, [$(($pat:expr, $kind:expr)),+ $(,)?]) => {
        fn $fn_name() -> &'static LangRules {
            static $cell: OnceLock<LangRules> = OnceLock::new();
            $cell.get_or_init(|| LangRules {
                patterns: vec![$((Regex::new($pat).unwrap(), $kind)),+],
            })
        }
    };
}

lang_rules!(
    c_family_rules,
    C_FAMILY_RULES,
    [
        // 函数定义/声明：`RET_TYPE name(params)`，大括号可能同行，也可能（更常见）
        // 另起一行——C 风格排版习惯，所以正则本身不要求行尾有 `{`。前面这段惰性
        // 前缀 `[\w:<>,*&\s]+?` 吃掉返回类型（可能带 `*`/`&`/命名空间/模板参数），
        // 靠紧跟在名字前的最后一个分隔符（空格/`*`/`&`）切出函数名——这样
        // `void *get_uab_lib()` 这种指针返回类型、`*` 紧贴函数名没有空格的写法
        // 也能命中（早期版本要求前缀每个词后面都跟空白，导致这类写法整行匹配
        // 失败，2026-09-16 用户反馈"简单的转定义都不行"就是这个问题）。
        (
            r"^\s*[\w:<>,*&\s]+?[\s*&]([A-Za-z_]\w*)\s*\(([^;{}]*)\)\s*\{?\s*;?\s*$",
            "function"
        ),
        (r"^\s*(?:typedef\s+)?(?:struct|class|enum|union)\s+([A-Za-z_]\w*)\b", "type"),
        (r"^\s*#\s*define\s+([A-Za-z_]\w*)", "macro"),
        (r"^\s*typedef\b.*[\s*]([A-Za-z_]\w*)\s*;\s*$", "typedef"),
    ]
);

lang_rules!(
    rust_rules,
    RUST_RULES,
    [
        (r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_]\w*)", "function"),
        (r"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+([A-Za-z_]\w*)", "type"),
        (r"^\s*(?:pub(?:\([^)]*\))?\s+)?enum\s+([A-Za-z_]\w*)", "type"),
        (r"^\s*(?:pub(?:\([^)]*\))?\s+)?trait\s+([A-Za-z_]\w*)", "type"),
        (r"^\s*(?:pub(?:\([^)]*\))?\s+)?type\s+([A-Za-z_]\w*)", "typedef"),
        (r"^\s*(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Za-z_][A-Za-z0-9_]*)\s*:", "const"),
        (r"^\s*(?:pub(?:\([^)]*\))?\s+)?static\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*:", "const"),
    ]
);

lang_rules!(
    python_rules,
    PYTHON_RULES,
    [
        (r"^\s*(?:async\s+)?def\s+([A-Za-z_]\w*)\s*\(", "function"),
        (r"^\s*class\s+([A-Za-z_]\w*)\s*[:\(]", "type"),
    ]
);

lang_rules!(
    go_rules,
    GO_RULES,
    [
        (r"^\s*func\s+(?:\([^)]*\)\s*)?([A-Za-z_]\w*)\s*\(", "function"),
        (r"^\s*type\s+([A-Za-z_]\w*)\s+(?:struct|interface)\b", "type"),
    ]
);

lang_rules!(
    js_rules,
    JS_RULES,
    [
        (
            r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s+([A-Za-z_$][\w$]*)\s*\(",
            "function"
        ),
        (r"^\s*(?:export\s+)?(?:default\s+)?class\s+([A-Za-z_$][\w$]*)", "type"),
        (
            r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?\(",
            "function"
        ),
        (r"^\s*(?:export\s+)?interface\s+([A-Za-z_$][\w$]*)", "type"),
        (r"^\s*(?:export\s+)?type\s+([A-Za-z_$][\w$]*)\s*=", "typedef"),
    ]
);

fn rules_for_extension(ext: &str) -> Option<&'static LangRules> {
    match ext {
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "ino" => Some(c_family_rules()),
        "rs" => Some(rust_rules()),
        "py" => Some(python_rules()),
        "go" => Some(go_rules()),
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" => Some(js_rules()),
        _ => None,
    }
}

/// 对单个文件的内容跑一遍对应语言的正则规则表，逐行匹配——一行只按命中的第一条
/// 规则记一次，避免同一行被多条规则重复计入（比如 C 的函数规则和 typedef 规则
/// 理论上可能同时"看起来像"匹配同一行开头）。
fn extract_symbols(path: &str, content: &str) -> Vec<(String, u32, &'static str)> {
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default();
    let Some(rules) = rules_for_extension(&ext) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        for (re, kind) in &rules.patterns {
            let Some(caps) = re.captures(line) else {
                continue;
            };
            let Some(whole) = caps.get(0) else { continue };
            let Some(m) = caps.get(1) else { continue };
            let name = m.as_str();
            if name.is_empty() || CONTROL_KEYWORDS.contains(&name) {
                continue;
            }
            // C 函数规则的前缀现在允许 `*`/`&` 紧贴在名字前（上面的注释解释了为什么），
            // 代价是像 `return foo();`/`throw foo();` 这类语句也满足"前缀 + 分隔符 + 名字 + (...)"
            // 的形状——排除法是看名字前面那一整段前缀，如果它整段就是一个控制流/语句
            // 关键字（不是真的类型），就不当函数定义算。
            let prefix = line[whole.start()..m.start()].trim();
            if CONTROL_KEYWORDS.contains(&prefix) {
                continue;
            }
            out.push((name.to_string(), (idx + 1) as u32, *kind));
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_c_function_with_brace_on_next_line() {
        let content = "static int kgbp_do_signin_via_kuab(KUABCLIHANDLE hHandle)\n{\n    return 0;\n}\n";
        let symbols = extract_symbols("kgmscli_kuabcli.cpp", content);
        assert!(symbols
            .iter()
            .any(|(name, line, kind)| name == "kgbp_do_signin_via_kuab" && *line == 1 && *kind == "function"));
    }

    #[test]
    fn does_not_mistake_control_flow_for_function() {
        let content = "if (get_kmgscli_flg_gm())\n{\n    return 0;\n}\n";
        let symbols = extract_symbols("foo.c", content);
        assert!(symbols.is_empty());
    }

    #[test]
    fn extracts_c_macro_and_struct() {
        let content = "#define MAX_LEN 128\nstruct Foo {\n    int x;\n};\n";
        let symbols = extract_symbols("foo.h", content);
        assert!(symbols.iter().any(|(name, _, kind)| name == "MAX_LEN" && *kind == "macro"));
        assert!(symbols.iter().any(|(name, _, kind)| name == "Foo" && *kind == "type"));
    }

    #[test]
    fn extracts_rust_fn_and_struct() {
        let content = "pub struct Foo;\n\nasync fn do_thing() -> Result<(), Error> {\n    Ok(())\n}\n";
        let symbols = extract_symbols("lib.rs", content);
        assert!(symbols.iter().any(|(name, _, kind)| name == "Foo" && *kind == "type"));
        assert!(symbols.iter().any(|(name, _, kind)| name == "do_thing" && *kind == "function"));
    }

    #[test]
    fn extracts_pointer_return_function_with_star_glued_to_name() {
        // 2026-09-16 用户反馈截图里的真实场景：`*` 紧贴函数名、没有空格分隔，
        // 声明和定义都要能命中，转到定义/声明才找得到。
        let decl = "extern void *get_uab_lib();\n";
        let symbols = extract_symbols("kgmscli_kuabcli.cpp", decl);
        assert!(symbols
            .iter()
            .any(|(name, line, kind)| name == "get_uab_lib" && *line == 1 && *kind == "function"));

        let def = "void *get_uab_lib()\n{\n    return g_lib_handle;\n}\n";
        let symbols = extract_symbols("kgmscli_kuabcli.cpp", def);
        assert!(symbols
            .iter()
            .any(|(name, line, kind)| name == "get_uab_lib" && *line == 1 && *kind == "function"));
    }

    #[test]
    fn does_not_mistake_return_statement_for_function_definition() {
        let content = "int wrapper(void)\n{\n    return get_uab_lib();\n}\n";
        let symbols = extract_symbols("foo.c", content);
        assert!(symbols.iter().any(|(name, _, _)| name == "wrapper"));
        assert!(!symbols.iter().any(|(name, _, _)| name == "get_uab_lib"));
    }

    #[test]
    fn index_file_reindex_replaces_old_entries() {
        let mut index = SymbolIndex::default();
        index.index_file("a.rs", "fn old_name() {}\n");
        assert_eq!(index.lookup("old_name").len(), 1);
        index.index_file("a.rs", "fn new_name() {}\n");
        assert!(index.lookup("old_name").is_empty());
        assert_eq!(index.lookup("new_name").len(), 1);
    }
}
