use std::io::{Cursor, Read};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// JAR 包（本质是 ZIP + `META-INF/MANIFEST.MF`）打开时展示的基本信息 + 依赖库列表
/// + 内部条目（2026-08-28 需求）。和 `binary_info::BinaryInfo` 是同一个"不可编辑的
/// 打包格式，打开时展示结构化信息而不是当文本硬打开"的思路，只是 JAR 的"依赖库"
/// 概念不一样——PE/ELF 是运行时链接的动态库，JAR 是 manifest 里 `Class-Path`
/// 声明的其它 jar（这个字段完全是可选的，很多 JAR 没有）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JarInfo {
    pub total_entries: usize,
    pub class_count: usize,
    /// manifest 的 `Main-Class` 属性——有这个值通常意味着这是个可以 `java -jar` 直接
    /// 运行的可执行 JAR。
    pub main_class: Option<String>,
    /// manifest 的 `Class-Path` 属性按空白拆分后的依赖 jar 列表（很多 JAR 没有这个
    /// 属性——现代 Java 项目更常用 fat jar 或外部的 classpath 文件管理依赖，这里能
    /// 展示的只是 manifest 自己声明的那部分）。
    pub class_path: Vec<String>,
    /// manifest 全部键值对，按文件里出现的原始顺序（供展示"其它 manifest 属性"）。
    pub manifest: Vec<ManifestAttribute>,
    /// 条目名（含目录分隔符 `/`），最多 `ENTRIES_HARD_CAP` 条，用途和
    /// `BinaryInfo::exports` 的截断说明一致——只是安全上限，不是正常展示截断。
    pub entries: Vec<JarEntryInfo>,
    pub entries_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestAttribute {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JarEntryInfo {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub compressed_size: u64,
}

/// 和 `binary_info::EXPORTS_HARD_CAP` 同样的取舍：fat jar/Spring Boot 打包产物可能有
/// 几千甚至上万个条目，这不是正常展示截断，只是防止病态文件把 IPC/前端渲染拖垮的
/// 安全上限。
const ENTRIES_HARD_CAP: usize = 50_000;

pub fn inspect(bytes: &[u8]) -> Result<JarInfo, AppError> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| AppError::Internal(format!("解析 JAR 包失败：{e}")))?;

    let total_entries = archive.len();
    let mut class_count = 0usize;
    let mut entries = Vec::with_capacity(total_entries.min(ENTRIES_HARD_CAP));
    let mut manifest_text: Option<String> = None;

    for i in 0..total_entries {
        let mut file = match archive.by_index(i) {
            Ok(f) => f,
            Err(_) => continue, // 单个条目损坏不影响其它条目的展示
        };
        let path = file.name().to_string();
        let is_dir = file.is_dir();
        if !is_dir && path.ends_with(".class") {
            class_count += 1;
        }
        if path.eq_ignore_ascii_case("META-INF/MANIFEST.MF") {
            let mut text = String::new();
            if file.read_to_string(&mut text).is_ok() {
                manifest_text = Some(text);
            }
        }
        if entries.len() < ENTRIES_HARD_CAP {
            entries.push(JarEntryInfo { path, is_dir, size: file.size(), compressed_size: file.compressed_size() });
        }
    }

    let manifest = manifest_text.as_deref().map(parse_manifest).unwrap_or_default();
    let main_class = find_attribute(&manifest, "Main-Class");
    let class_path = find_attribute(&manifest, "Class-Path")
        .map(|v| v.split_whitespace().map(|s| s.to_string()).collect())
        .unwrap_or_default();

    Ok(JarInfo {
        total_entries,
        class_count,
        main_class,
        class_path,
        manifest: manifest.into_iter().map(|(key, value)| ManifestAttribute { key, value }).collect(),
        entries_truncated: total_entries > ENTRIES_HARD_CAP,
        entries,
    })
}

fn find_attribute(manifest: &[(String, String)], key: &str) -> Option<String> {
    manifest.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone())
}

/// JAR manifest 格式（JAR 规范，不是 INI）：`Key: Value` 一行一条属性，值超过 72
/// 字节会在下一行用单个空格开头续行——续行不是新属性，要拼接回上一条的 value，
/// 不处理这个换行规则的话，长的 `Class-Path`/`Implementation-Vendor-Id` 之类的属性
/// 会被错误截断。
///
/// 只解析"主区块"（文件开头到第一个空行）：空行之后是逐条目区块，给单个 class/
/// 资源文件签名或打标记用的（比如按条目出现的 `Name`/`Sealed`），属于那个条目自己，
/// 不是 JAR 整体的信息——真实构建产物（比如 Derby）经常有几十个这种重复的
/// `Name`/`Sealed` 对，混进"基本信息"里展示只会是噪音，`Main-Class`/`Class-Path`
/// 这些真正关心的属性也总是在主区块里，不会漏。
fn parse_manifest(text: &str) -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        if let Some(continuation) = line.strip_prefix(' ') {
            if let Some(last) = result.last_mut() {
                last.1.push_str(continuation);
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            result.push((key.trim().to_string(), value.trim_start().to_string()));
        }
    }
    result
}
