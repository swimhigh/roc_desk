//! 免注册 COM（Registration-Free COM）：不依赖 `mstscax.dll` 在系统注册表里已经有
//! `HKCR\CLSID\{...}` 条目——真实测试机上就踩到过这个坑：`mstscax.dll` 文件本身在
//! `System32`/`SysWOW64` 里都有，但两边注册表 `CLSID` 键都是空的（`regsvr32` 从来
//! 没跑过），`CoCreateInstance` 直接报 `0x80040154`（`REGDB_E_CLASSNOTREG`）。让每个
//! 用户在能用 RDP 功能前先以管理员身份手动 `regsvr32 mstscax.dll` 是不可接受的
//! 使用门槛，而且这是全局系统级改动，不是 roc_desk 该做的事。
//!
//! MobaXterm 是单文件绿色版、不要求装 mstscax 或者其它管理员权限前置步骤就能用
//! RDP——参照的就是 Windows 从 Vista 起提供的"隔离/免注册 COM"机制：把原本该写进
//! `HKCR\CLSID` 的 `clsid → dll → threadingModel` 映射改放进一份"清单"（manifest）
//! XML 里，运行时用 `CreateActCtxW`/`ActivateActCtx` 把这份清单临时压进"激活上下文"
//! 栈顶，`CoCreateInstance` 在正常注册表查找之前会先查当前激活上下文——查到了就直接
//! 用清单里写的 DLL 创建对象，完全不需要管理员权限，也不依赖系统上这个 DLL 有没有
//! 被注册过。
//!
//! **真机联调踩了两轮才落地的细节**：
//!   1. 第一版只给了 `lpSource`（清单文件路径，写在 `%TEMP%` 下），`CoCreateInstance`
//!      从 `REGDB_E_CLASSNOTREG` 变成 `0x8007007E`（`ERROR_MOD_NOT_FOUND`）——说明
//!      激活上下文确实生效、查到了 clsid 映射，但 SxS/fusion 加载器解析
//!      `<file name="mstscax.dll">` 时，是把"清单文件所在的目录"当探测根目录去找
//!      这个文件名的，不会像普通 `LoadLibrary` 那样退回标准 DLL 搜索路径（System32
//!      在内）——这是"隔离部署"机制刻意的设计（避免 DLL 搜索路径劫持），不是 bug。
//!   2. 第二版尝试用 `ACTCTX_FLAG_ASSEMBLY_DIRECTORY_VALID` + `lpAssemblyDirectory`
//!      把探测根目录显式指向系统目录，同样的错误码原样复现——说明这个字段实际上
//!      不影响 `<file>` 元素的探测路径（很可能只对 `<dependency>` 引用的其它程序集
//!      生效），是凭对 SxS 机制的印象而非验证过的行为下的判断，判断错了。
//!
//! 最终改成最基础、最没有歧义的私有程序集部署方式：把系统那份 `mstscax.dll` **复制
//! 一份**到清单文件所在的同一个目录（`%LOCALAPPDATA%\roc_desk\mstscax_actctx\`），
//! `<file name="mstscax.dll">` 就在探测根目录（清单自己所在的目录）里直接找得到，
//! 不依赖任何"退回搜索路径"或"显式指定探测目录"这类还需要验证的行为——这是各种
//! 免注册 COM 教程里最基本的用法（DLL 和清单放一块），必定成立。只在目标文件不存在
//! 时才复制（幂等，不是每次连接都重新拷一遍几 MB 的文件），代价是如果系统上的
//! `mstscax.dll` 被 Windows Update 换了新版本，roc_desk 这边缓存的副本要等这个目录
//! 被清掉才会跟着更新——可以接受，这个控件的公开接口这些年没有变过。

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::{Error, Result, PCWSTR};
use windows::Win32::Foundation::{E_FAIL, HANDLE};
use windows::Win32::System::ApplicationInstallationAndServicing::{ActivateActCtx, CreateActCtxW, DeactivateActCtx, ReleaseActCtx, ACTCTXW};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="RocDesk.MsTscAx" version="1.0.0.0" processorArchitecture="amd64" />
  <file name="mstscax.dll">
    <comClass clsid="{1FB464C8-09BB-4017-A2F5-EB742F04392F}" threadingModel="Apartment" />
  </file>
</assembly>
"#;

/// 激活上下文的 RAII 守卫——`Drop` 时按创建的反顺序 `DeactivateActCtx`→`ReleaseActCtx`，
/// 保证哪怕中间 `CoCreateInstance` 失败提前返回，激活上下文也不会泄漏在栈上或内存里。
pub struct ActivationGuard {
    handle: HANDLE,
    cookie: usize,
}

impl ActivationGuard {
    pub fn activate() -> Result<Self> {
        let dir = staging_dir();
        std::fs::create_dir_all(&dir).map_err(io_err("创建免注册 COM 暂存目录失败"))?;

        let dll_path = dir.join("mstscax.dll");
        if !dll_path.exists() {
            std::fs::copy(system_mstscax_path(), &dll_path).map_err(io_err("拷贝系统 mstscax.dll 到暂存目录失败"))?;
        }

        let manifest_path = dir.join("roc_desk_mstscax.manifest");
        std::fs::write(&manifest_path, MANIFEST).map_err(io_err("写入激活上下文清单文件失败"))?;

        let mut path_wide: Vec<u16> = manifest_path.as_os_str().encode_wide().collect();
        path_wide.push(0);

        let actctx = ACTCTXW {
            cbSize: core::mem::size_of::<ACTCTXW>() as u32,
            dwFlags: 0,
            lpSource: PCWSTR(path_wide.as_ptr()),
            wProcessorArchitecture: 0,
            wLangId: 0,
            lpAssemblyDirectory: PCWSTR::null(),
            lpResourceName: PCWSTR::null(),
            lpApplicationName: PCWSTR::null(),
            hModule: Default::default(),
        };

        let handle = unsafe { CreateActCtxW(&actctx)? };
        let mut cookie: usize = 0;
        if let Err(e) = unsafe { ActivateActCtx(Some(handle), &mut cookie as *mut usize) } {
            unsafe { ReleaseActCtx(handle) };
            return Err(e);
        }
        Ok(Self { handle, cookie })
    }
}

impl Drop for ActivationGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = DeactivateActCtx(0, self.cookie);
            ReleaseActCtx(self.handle);
        }
    }
}

fn io_err(context: &'static str) -> impl FnOnce(std::io::Error) -> Error {
    move |e| Error::new(E_FAIL, format!("{context}：{e}"))
}

fn staging_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("roc_desk").join("mstscax_actctx")
}

fn system_mstscax_path() -> PathBuf {
    let mut buf = vec![0u16; 260];
    let len = unsafe { GetSystemDirectoryW(Some(&mut buf)) };
    buf.truncate(len as usize);
    let system_dir = String::from_utf16_lossy(&buf);
    Path::new(&system_dir).join("mstscax.dll")
}
