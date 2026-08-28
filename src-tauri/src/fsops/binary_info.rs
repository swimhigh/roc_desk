use goblin::Object;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// EXE/DLL（PE）、SO（ELF）等不可编辑的可执行文件打开时展示的基本信息 + 依赖库列表
/// （2026-08-28 需求：这类文件之前只能"用系统默认程序打开"，用户想直接看到摘要而不用
/// 真的去运行/反编译它）。只解析头部结构（`goblin::Object::parse` 不加载、不执行代码），
/// 和图片/PDF 预览一样是纯只读展示，不支持编辑保存。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryInfo {
    /// "PE" / "ELF" / "Mach-O"
    pub format: String,
    /// 例如 "x86_64" / "x86 (i386)" / "ARM64"，解析不出已知机器类型时是十六进制原始值。
    pub architecture: String,
    pub bitness: String,
    /// 例如 "可执行文件 (EXE)" / "动态链接库 (DLL)" / "共享库 (SO)"。
    pub file_kind: String,
    /// 入口点的文件内相对地址，展示成十六进制字符串（不同格式的"地址"含义不完全一致，
    /// 展示层不需要关心，只是一个诊断信息）。
    pub entry_point: Option<String>,
    /// PE 独有：链接时间戳，格式化成 "YYYY-MM-DD HH:MM:SS UTC"。
    pub timestamp: Option<String>,
    /// PE 独有：子系统（Windows GUI/CUI 等）。
    pub subsystem: Option<String>,
    /// 依赖的动态库名（PE 的 Import Table DLL 名 / ELF 的 DT_NEEDED），去重后原始顺序。
    pub dependencies: Vec<String>,
    /// 导出符号名——2026-08-28 用户反馈"要能查看所有的导出符号，这对开发人员很重要"，
    /// 不再只截断展示前一小部分；`exports_truncated` 只在极端情况下（超过
    /// `EXPORTS_HARD_CAP`）才会是 true，正常软件的导出表不会碰到这个上限。
    pub exports: Vec<String>,
    pub exports_truncated: bool,
    pub total_exports: usize,
    pub sections: Vec<SectionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionInfo {
    pub name: String,
    pub virtual_size: u64,
    pub raw_size: u64,
}

/// 导出符号数量的兜底安全上限——不是正常展示用的截断阈值（2026-08-28 用户反馈
/// "要能查看所有的导出符号，这对开发人员很重要"，之前 200 条的展示上限已经去掉），
/// 只是防止极端病态文件（伪造的导出表）产生几十万条字符串把 IPC/前端渲染拖垮。
/// 真实软件的导出表远远到不了这个数量级。
const EXPORTS_HARD_CAP: usize = 50_000;

pub fn inspect(bytes: &[u8]) -> Result<BinaryInfo, AppError> {
    match Object::parse(bytes).map_err(|e| AppError::Internal(format!("解析可执行文件失败：{e}")))? {
        Object::PE(pe) => Ok(from_pe(&pe)),
        Object::Elf(elf) => Ok(from_elf(&elf)),
        Object::Mach(mach) => from_mach(mach),
        Object::Archive(_) => Err(AppError::Internal("这是静态库归档文件（ar 格式），不是可执行文件/动态库".into())),
        _ => Err(AppError::Internal("无法识别的可执行文件格式".into())),
    }
}

/// 没有已知可执行文件扩展名的文件（典型是 Linux 下的 ELF 可执行文件——习惯上不带
/// 扩展名，2026-08-28 用户反馈"Linux 的可执行文件默认是以文本编辑器查看的"）打开前，
/// 靠开头几个字节的魔数判断它到底是不是可执行文件/动态库，不是看扩展名。只需要文件
/// 开头一小段，调用方（`commands::fs::fs_peek_is_binary`）只读了 64 字节，代价可以
/// 忽略；这里顺带也识别 PE/Mach-O 魔数，虽然那两种平台上通常都有扩展名，兜底一下
/// 没有坏处。
pub fn looks_like_binary(head: &[u8]) -> bool {
    if head.len() >= 4 && &head[0..4] == b"\x7fELF" {
        return true;
    }
    if head.len() >= 2 && &head[0..2] == b"MZ" {
        return true;
    }
    if head.len() >= 4 {
        const MACH_MAGICS: [u32; 3] = [0xfeedface, 0xfeedfacf, 0xcafebabe];
        let le = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let be = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
        if MACH_MAGICS.contains(&le) || MACH_MAGICS.contains(&be) {
            return true;
        }
    }
    false
}

fn truncated_exports(names: Vec<String>) -> (Vec<String>, bool, usize) {
    let total = names.len();
    if total > EXPORTS_HARD_CAP {
        (names.into_iter().take(EXPORTS_HARD_CAP).collect(), true, total)
    } else {
        (names, false, total)
    }
}

fn pe_machine_name(machine: u16) -> String {
    // 只覆盖常见架构；goblin::pe::header 里的常量本身就是这些裸 u16 值，直接匹配
    // 比引入常量名更省心（新架构出现时至少能看到十六进制原始值，不会误报）。
    match machine {
        0x014c => "x86 (i386)".to_string(),
        0x8664 => "x86_64".to_string(),
        0xaa64 => "ARM64".to_string(),
        0x01c0 => "ARM".to_string(),
        0x01c4 => "ARM Thumb-2".to_string(),
        0x0200 => "IA64".to_string(),
        other => format!("未知 (0x{other:04x})"),
    }
}

fn pe_subsystem_name(subsystem: u16) -> String {
    match subsystem {
        1 => "Native".to_string(),
        2 => "Windows GUI".to_string(),
        3 => "Windows 控制台 (CUI)".to_string(),
        5 => "OS/2 CUI".to_string(),
        7 => "POSIX CUI".to_string(),
        9 => "Windows CE GUI".to_string(),
        10 => "EFI Application".to_string(),
        13 => "EFI ROM".to_string(),
        14 => "Xbox".to_string(),
        other => format!("未知 (0x{other:04x})"),
    }
}

fn from_pe(pe: &goblin::pe::PE) -> BinaryInfo {
    let machine = pe.header.coff_header.machine;
    let timestamp = {
        let secs = pe.header.coff_header.time_date_stamp as i64;
        chrono::DateTime::from_timestamp(secs, 0).map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
    };
    let subsystem = pe.header.optional_header.map(|oh| pe_subsystem_name(oh.windows_fields.subsystem));

    let dependencies: Vec<String> = pe.libraries.iter().map(|s| s.to_string()).collect();
    let export_names: Vec<String> = pe.exports.iter().filter_map(|e| e.name.map(|n| n.to_string())).collect();
    let (exports, exports_truncated, total_exports) = truncated_exports(export_names);

    let sections = pe
        .sections
        .iter()
        .map(|s| SectionInfo {
            name: s.name().unwrap_or("").to_string(),
            virtual_size: s.virtual_size as u64,
            raw_size: s.size_of_raw_data as u64,
        })
        .collect();

    BinaryInfo {
        format: "PE".to_string(),
        architecture: pe_machine_name(machine),
        bitness: if pe.is_64 { "64 位".to_string() } else { "32 位".to_string() },
        file_kind: if pe.is_lib { "动态链接库 (DLL)".to_string() } else { "可执行文件 (EXE)".to_string() },
        entry_point: Some(format!("0x{:x}", pe.entry)),
        timestamp,
        subsystem,
        dependencies,
        exports,
        exports_truncated,
        total_exports,
        sections,
    }
}

fn elf_machine_name(machine: u16) -> String {
    use goblin::elf::header;
    match machine {
        header::EM_X86_64 => "x86_64".to_string(),
        header::EM_386 => "x86 (i386)".to_string(),
        header::EM_AARCH64 => "ARM64".to_string(),
        header::EM_ARM => "ARM".to_string(),
        header::EM_RISCV => "RISC-V".to_string(),
        header::EM_MIPS => "MIPS".to_string(),
        header::EM_PPC64 => "PowerPC64".to_string(),
        other => format!("未知 (0x{other:04x})"),
    }
}

fn from_elf(elf: &goblin::elf::Elf) -> BinaryInfo {
    use goblin::elf::header;

    let file_kind = match elf.header.e_type {
        header::ET_EXEC => "可执行文件".to_string(),
        // ET_DYN 同时覆盖共享库和 PIE 可执行文件——有解释器（.interp 段）的是可独立
        // 运行的 PIE 可执行文件，没有的才是纯共享库，不能只看 e_type。
        header::ET_DYN if elf.interpreter.is_some() => "可执行文件 (PIE)".to_string(),
        header::ET_DYN => "共享库 (SO)".to_string(),
        header::ET_REL => "目标文件（未链接）".to_string(),
        header::ET_CORE => "Core Dump".to_string(),
        other => format!("未知类型 ({other})"),
    };

    let dependencies: Vec<String> = elf.libraries.iter().map(|s| s.to_string()).collect();

    // 动态符号表里既有导入也有导出，只挑"这个文件自己定义、且全局/弱可见"的当作导出——
    // 否则会把这个库自己依赖的外部符号也混进"导出"列表里，误导用户。
    let export_names: Vec<String> = elf
        .dynsyms
        .iter()
        .filter(|sym| !sym.is_import() && sym.st_value != 0 && (sym.is_function() || sym.st_bind() == goblin::elf::sym::STB_GLOBAL))
        .filter_map(|sym| elf.dynstrtab.get_at(sym.st_name).map(|s| s.to_string()))
        .collect();
    let (exports, exports_truncated, total_exports) = truncated_exports(export_names);

    let sections = elf
        .section_headers
        .iter()
        .filter_map(|sh| {
            elf.shdr_strtab.get_at(sh.sh_name).map(|name| SectionInfo {
                name: name.to_string(),
                virtual_size: sh.sh_size,
                raw_size: sh.sh_size,
            })
        })
        .filter(|s| !s.name.is_empty())
        .collect();

    BinaryInfo {
        format: "ELF".to_string(),
        architecture: elf_machine_name(elf.header.e_machine),
        bitness: if elf.is_64 { "64 位".to_string() } else { "32 位".to_string() },
        file_kind,
        entry_point: Some(format!("0x{:x}", elf.entry)),
        timestamp: None,
        subsystem: None,
        dependencies,
        exports,
        exports_truncated,
        total_exports,
        sections,
    }
}

fn mach_machine_name(cputype: u32) -> String {
    // goblin::mach::constants 里的 CPU_TYPE_* 常量本身就是这些裸值。
    match cputype {
        0x0100_0007 => "x86_64".to_string(),
        0x0000_0007 => "x86 (i386)".to_string(),
        0x0100_000c => "ARM64".to_string(),
        0x0000_000c => "ARM".to_string(),
        other => format!("未知 (0x{other:08x})"),
    }
}

fn from_mach(mach: goblin::mach::Mach) -> Result<BinaryInfo, AppError> {
    // Fat/universal binary（多架构合一）取第一个可解析的架构展示——这类文件在
    // roc_desk 的使用场景（远程 Linux 主机/本地 Windows 工作区）里很少见，够用即可，
    // 不需要把每个架构都摊开展示。
    let single = match mach {
        goblin::mach::Mach::Binary(m) => m,
        goblin::mach::Mach::Fat(fat) => {
            let arch = fat
                .into_iter()
                .find_map(|a| a.ok())
                .ok_or_else(|| AppError::Internal("无法解析 Fat/Universal Mach-O 文件".into()))?;
            match arch {
                goblin::mach::SingleArch::MachO(m) => m,
                goblin::mach::SingleArch::Archive(_) => {
                    return Err(AppError::Internal("Fat 文件里是静态库归档，不是可执行文件/动态库".into()))
                }
            }
        }
    };

    let dependencies: Vec<String> = single.libs.iter().filter(|l| **l != "self").map(|s| s.to_string()).collect();
    let export_names: Vec<String> = single.exports().ok().map(|es| es.into_iter().map(|e| e.name).collect()).unwrap_or_default();
    let (exports, exports_truncated, total_exports) = truncated_exports(export_names);

    let sections: Vec<SectionInfo> = single
        .segments
        .sections()
        .flatten()
        .filter_map(|r| r.ok())
        .map(|(section, _)| SectionInfo {
            name: section.name().unwrap_or("").to_string(),
            virtual_size: section.size,
            raw_size: section.size,
        })
        .collect();

    Ok(BinaryInfo {
        format: "Mach-O".to_string(),
        architecture: mach_machine_name(single.header.cputype),
        bitness: if single.is_64 { "64 位".to_string() } else { "32 位".to_string() },
        file_kind: if single.header.filetype == goblin::mach::header::MH_DYLIB {
            "动态库 (dylib)".to_string()
        } else {
            "可执行文件".to_string()
        },
        entry_point: single.entry.checked_sub(0).map(|e| format!("0x{e:x}")),
        timestamp: None,
        subsystem: None,
        dependencies,
        exports,
        exports_truncated,
        total_exports,
        sections,
    })
}
