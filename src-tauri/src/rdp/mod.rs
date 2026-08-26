//! RDP 远程桌面（远程工具模式，DESIGN.md §3.9）。
//!
//! **不自己实现 RDP 协议**——这是用户两天里反复调整过方向的结果，每一步都是真
//! 踩出来的坑，不是拍脑袋换的：
//!   1. 纯 Rust 的 `rustls` 只支持现代 TLS1.2/1.3 + 前向保密套件，遇到 SChannel
//!      策略配置得比较老/受限的 Windows 服务器（能被 mstsc.exe/MobaXterm 正常连上）
//!      时，ClientHello 里没有一个套件对方能接受，直接被 TCP RST 掉。
//!   2. 换 `native-tls` 绕开①之后，又踩到当时用的协议库 `IronRDP` 本身没实现"并发
//!      会话数超限时展示 Windows 会话选择/踢人界面"这个能力——查过源码，服务器发的
//!      是专门的 `ServerSetErrorInfo` 协议包，`ironrdp-session` 收到就直接断线，
//!      压根没有走到"渲染成画面给用户选"这条路径，这是库没做，不是配置问题。
//!   3. 改成以 ActiveX 方式承载 Windows 自带的 `mstscax.dll` 控件——手抄 COM 接口
//!      （逐槽位比对过类型库，无误）、免注册 COM 激活上下文、两阶段就地激活、
//!      `OnFrameWindowActivate`/`OnDocWindowActivate`、`IOleControlSite`……逐项对照
//!      Devolutions MsRdpEx（一份真实在用的原生 C++ 容器，同一控件）的源码把能做的
//!      全做了，`Connect()` 依然同步返回 `S_FALSE` 并立刻自我断开，且这个行为
//!      100% 发生在触碰网络之前（用不存在的服务器地址测试，结果和真实服务器完全
//!      一样）。花了一天多用直接调用内部代码的探针排除了 vtable 错位/coclass
//!      选错/容器接口缺失/环境属性/密码设置接口这些可能性，仍然没找到根因。
//!   4. 退回拉起系统自带 `mstsc.exe`、`SetParent` 嵌入——这条内嵌外部进程窗口的
//!      路径本身在早期测试中渲染过证书警告对话框（说明机制可行），但换到真正的
//!      RDP 桌面画面后，真机联调复现的现象是：内嵌窗口的句柄/位置/尺寸/可见性
//!      全部正常，但内容是纯黑。查证是跨进程 `SetParent` 之后硬件加速渲染窗口的
//!      通病——现代 mstsc.exe 用 DirectX/DXGI 画桌面，窗口被"过继"给别的进程时，
//!      窗口本身这些"窗口管理器属性"（DWM/USER32 层面）不受影响，但 GPU 合成表面
//!      没有正确重新挂到新宿主上。试过"尺寸抖动强制 `WM_SIZE`"+`RedrawWindow`
//!      强制重绘这两个通用手法，均无效——这类问题的正规解法（Windows"不允许硬件
//!      加速解码"策略）要写 `HKLM` 且要管理员权限改全机策略，对我们的场景太重，
//!      而且从现象看这更可能是 DWM 对子窗口和顶层窗口合成 DirectX 内容方式的
//!      结构性差异，不是"没触发重绘"这种时序问题，砸时间在这条路上继续试大概率
//!      是白费。
//!
//! ## 最终方案：换成 FreeRDP 的 `wfreerdp.exe`
//! 用户原话（参考 Remmina 这类基于 FreeRDP 内核的开源远程管理器后）："参考下这个
//! 代码换成FREERDP试下吧"。选它不只是"换一个能连的客户端"，是有具体技术理由的：
//! `wfreerdp.exe` 是 FreeRDP 官方已标记为 deprecated 的旧式 Windows 客户端（替代品
//! 是新的 SDL 客户端），渲染管线比现代 mstsc.exe 简单、老，大概率不是 DXGI 合成
//! 表面那一套，从根上不会撞见③里诊断出的那类跨进程子窗口合成失败——这是这一轮
//! 换客户端和之前"再猜一个开关试试"性质不同的地方。
//!
//! 二进制来自 Chocolatey 的 `freerdp.portable` 包（`vendor/wfreerdp.exe`，SHA256
//! 校验过，可追溯到 FreeRDP 官方 CI 按 GitHub 正式发布 tag 打的构建），随应用一起
//! 分发。密码/用户名直接通过 `/u:`/`/p:` 命令行参数传给 `wfreerdp.exe`（FreeRDP
//! 不像 mstsc.exe 那样对接 Windows 凭据管理器，不需要 `cmdkey` 那套预置/清理流程，
//! 代价是密码会短暂以明文形式出现在这个进程的命令行里，本机其它有权限的进程/
//! 管理员能看到——这是接受的已知取舍，多数同类工具对 FreeRDP 命令行客户端都是
//! 这么处理密码的）。窗口查找/内嵌/持续监视这套基础设施（下面几节）和客户端换
//! 哪一个无关，原样复用。
//!
//! ## 真正要修的 bug：只在方案④之前就诊断出来、一直没修
//! mstsc.exe（以及大概率 wfreerdp.exe 也一样）一个进程生命周期里会先后弹出好几个
//! 独立的顶层窗口（证书警告 → 可能的会话选择 → 真正桌面画面），最早的实现只在
//! 连接发起那一刻 `EnumWindows` 探测一次，抓到第一个窗口嵌进来后就不再追踪——
//! 真机实测复现过：证书警告能看到，但对话框消失后画面窗口没跟着嵌进来，等于卡在
//! 半途。这次从设计上直接修掉这个问题：不是只连接时找一次，而是专门起一个后台
//! 线程，整个会话生命周期内持续轮询这个 PID 当前的顶层窗口，一旦和已内嵌的不
//! 一样就重新摘边框/重新 `SetParent`/重新定位——挑"当前该嵌哪个窗口"不按"有没有
//! 标题"（真机联调发现真正渲染桌面的那个窗口标题可能是空的，按标题过滤会永远
//! 找不到它），改成按"这个 PID 名下面积最大的可见窗口"，桌面渲染窗口天然比证书
//! 警告/连接状态这类小对话框大，不依赖标题内容。
//!
//! ## 内嵌手法的最后一次调整：从"转子窗口"改成"顶层属主窗口"
//! 换成 `wfreerdp.exe` 之后真机复现的现象和上面④里 mstsc.exe 的黑屏一模一样——
//! 先怀疑是 `wfreerdp.exe` 这份二进制用 `WITH_CLIENT_SDL2=ON` 编译、SDL2 默认走
//! Direct3D/OpenGL 硬件加速，试过强制 `SDL_RENDER_DRIVER=software`+
//! `SDL_VIDEODRIVER=windows` 关掉硬件加速，真机测试**仍然黑屏**。两次用完全不同的
//! 渲染技术（mstsc.exe 的 DXGI、wfreerdp.exe 的 SDL2 且已强制软件渲染）复现出
//! 同一个"窗口属性全部正常、内容纯黑"的症状，说明问题根本不在具体渲染后端，而在
//! `SetParent`+`WS_CHILD` 这个内嵌手法本身——这类窗口从创建起就是独立顶层窗口，
//! 转成子窗口后 DWM 不会正确把它们的合成表面重新挂到新宿主上，这是结构性限制，
//! 换任何渲染后端都躲不掉。
//!
//! 真正的修法是不再转子窗口：`attach_owned_window` 保留窗口的顶层身份，只用
//! `SetWindowLongPtrW(GWLP_HWNDPARENT, ...)` 把 roc_desk 主窗口设成它的"属主"
//! （不是 `SetParent` 的"父子"关系），再靠 `position_window` 持续把它的屏幕坐标
//! 摆到面板对应的位置——DWM 按对待任何其它普通顶层窗口完全相同的路径合成它，
//! 画面必然正常。属主关系带来几个免费的好处：Windows 保证属主窗口的 z-order
//! 始终在属主之上、属主最小化时跟着隐藏、默认不出现在 Alt+Tab/任务栏（额外设置
//! `WS_EX_TOOLWINDOW` 加固这一点）——不需要我们手动维护。
//!
//! 代价是真实的、如实记录：这个窗口是货真价实的独立顶层窗口，会浮在 roc_desk
//! 整个界面之上（不只是面板那一块区域）。如果 RDP 标签页可见的同时前端弹出下拉
//! 菜单/对话框这类浮层 UI，会被这个窗口盖住——这是"用真窗口贴一块区域"这类做法
//! 共有的"空域"（airspace）问题，本轮只处理了"切标签页"这一种场景（`hide`/`show`），
//! 没有为每种浮层单独接线；真机确认这个方案能出画面之后再按实际影响到的场景加。
//!
//! ## 代价（如实记录，不是没想到）
//!   - 只在 Windows 上能用（`[target.'cfg(windows)'.dependencies]`），但 roc_desk
//!     本来就是 Windows 专属客户端，这不是新增的限制。
//!   - 密码经命令行参数传递的取舍见上面"最终方案"一节。
//!   - `wfreerdp.exe` 本身已被 FreeRDP 官方标记为 deprecated——目前功能完整可用，
//!     官方哪天真正下架了需要跟进换成 SDL 客户端。
//!   - 换成属主窗口方案之后暴露过一个新问题：`WS_CHILD` 子窗口会在父窗口销毁时被
//!     Windows 自动级联销毁（这顺带掩盖了"退出时没人管 RDP 子进程"这个缺口，
//!     wfreerdp 进程的窗口被系统强制关掉，它自己的消息循环也就跟着退出了）；
//!     属主窗口没有这层自动级联，真机测试证实退出 roc_desk 后 wfreerdp.exe 会
//!     变成孤儿进程留在桌面上。修法是 `lib.rs` 的 `run()` 挂了
//!     `RunEvent::ExitRequested` 回调，正常退出时显式调用一次 `disconnect_all`
//!     （杀子进程连带其窗口）。应用被强制杀掉（不是走正常关闭流程，比如任务管理器
//!     强制结束进程）的话这个回调不会触发，孤儿窗口仍然可能留下——这是没法在
//!     应用自己代码里彻底堵上的场景，只能算已知的 v1 缺口。

use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::{AppHandle, Manager};
use uuid::Uuid;
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, GWL_EXSTYLE, GWL_STYLE, GWLP_HWNDPARENT, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CAPTION, WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME, WS_VISIBLE,
};

use crate::connection::{ConnectionManager, ConnectionProfile, Protocol};
use crate::error::AppError;

/// 不弹出控制台窗口（`wfreerdp.exe` 是控制台子系统程序，日志走 stdout/stderr——
/// 我们的 GUI 进程没有控制台，Windows 默认会给它新分配一个可见的控制台窗口，
/// 那样用户会在嵌入的 RDP 窗口旁边看到一个多余的黑框）。
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PanelBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpStatusState {
    Connecting,
    Connected,
    Disconnected,
    Error,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RdpStatus {
    pub state: RdpStatusState,
    pub reason: Option<i32>,
    /// 内嵌窗口的实时体检信息——嵌入的是外部进程窗口，我们拿不到 RDP 协议层面的
    /// 连接状态，只能诚实报告"这个会话眼下到底嵌的是哪个窗口"，不假装知道更多。
    pub diagnostics: String,
}

/// RDP 专属的少量额外字段，存在 `ConnectionProfile.options` 里（前端 `ConnectionForm`
/// 的 RDP 分支写进去的：domain/width/height）。
#[derive(Debug, Deserialize, Default)]
struct RdpProfileOptions {
    domain: Option<String>,
    width: Option<u16>,
    height: Option<u16>,
}

struct EmbeddedSession {
    child: Child,
    /// 当前被内嵌的窗口句柄，`0` 表示还没找到任何窗口。会随后台监视线程的发现
    /// 不断更新，不是连接那一刻定死的——这是本模块顶部说明的"持续监视"机制的落点。
    /// `HWND` 内部是裸指针不是 `Send`，存成 `isize` 绕开限制，只当不透明句柄用。
    hwnd: Arc<AtomicIsize>,
    /// 面板最后一次已知的屏幕坐标——后台线程发现新窗口时要用这个把它摆到正确位置，
    /// 它自己不知道当前应该摆在哪（那是前端通过 `set_bounds`/`show` 告诉我们的）。
    last_bounds: Arc<Mutex<Option<PanelBounds>>>,
    /// 通知监视线程该退出了。
    stop: Arc<AtomicBool>,
    #[allow(dead_code)]
    watcher: JoinHandle<()>,
}

fn hwnd_from_isize(v: isize) -> HWND {
    HWND(v as *mut core::ffi::c_void)
}

/// 内嵌 wfreerdp 窗口的会话登记表（远程工具模式，DESIGN.md §3.9）。和 SSH 的
/// `SshConnectionPool` 不是一回事——这里没有"多路复用同一条连接"的概念，每次
/// "打开 RDP"都是一个独立的 wfreerdp.exe 进程，key 是 session_id 不是 profile_id。
pub struct RdpSessionManager {
    sessions: Mutex<HashMap<Uuid, EmbeddedSession>>,
    connection_manager: Arc<ConnectionManager>,
}

impl RdpSessionManager {
    pub fn new(connection_manager: Arc<ConnectionManager>) -> Self {
        Self { sessions: Mutex::new(HashMap::new()), connection_manager }
    }

    pub async fn connect(&self, app_handle: &AppHandle, profile_id: Uuid, bounds: PanelBounds) -> Result<Uuid, AppError> {
        let profile: ConnectionProfile = self
            .connection_manager
            .get(profile_id)?
            .ok_or_else(|| AppError::NotFound(format!("connection not found: {profile_id}")))?;
        if profile.protocol != Protocol::Rdp {
            return Err(AppError::Conflict(format!("connection {profile_id} is not an RDP profile")));
        }
        let secret = self
            .connection_manager
            .resolve_secret(&profile)
            .await?
            .ok_or_else(|| AppError::Auth("缺少 RDP 密码".into()))?;

        let opts: RdpProfileOptions = profile
            .options
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let width = opts.width.unwrap_or(1280);
        let height = opts.height.unwrap_or(800);
        let server = if profile.port == 3389 { profile.host.clone() } else { format!("{}:{}", profile.host, profile.port) };
        let user_arg = match opts.domain.as_deref() {
            Some(domain) if !domain.is_empty() => format!("{domain}\\{}", profile.username),
            _ => profile.username.clone(),
        };

        let wfreerdp = find_wfreerdp_exe()?;

        // 起进程、等第一个窗口出现都是同步阻塞调用，扔到 tokio 的阻塞线程池，
        // 不占用 async 运行时的 worker 线程（等窗口最多要等 15 秒）。`HWND` 内部是
        // 裸指针不是 `Send`，过不了 `spawn_blocking` 返回值的边界——闭包内部转成
        // `isize` 带出来，外面再用 `hwnd_from_isize` 转回去。
        let (child, hwnd_raw, pid) = tokio::task::spawn_blocking(move || -> Result<(Child, isize, u32), AppError> {
            let mut child = Command::new(&wfreerdp)
                .arg(format!("/v:{server}"))
                .arg(format!("/u:{user_arg}"))
                .arg(format!("/p:{secret}"))
                .arg(format!("/size:{width}x{height}"))
                // 我们的 GUI 进程没有控制台可以回答交互式确认，跳过证书信任提示、
                // 直接接受——不然会卡在一个没人能应答的确认请求上，等价于之前
                // ActiveX 方案里"没有事件接收者应答 OnReceivedTSPublicKey"那个坑。
                .arg("/cert:ignore")
                // 真机联调直接在命令行跑 wfreerdp（不经过我们的应用）逐台复现过两台
                // 测试服务器的真实失败原因，不是猜的：
                //   - 10.203.0.111：默认安全协商成功但 TLS 握手本身失败
                //     （`BIO_do_handshake failed`）——和这个会话最开始 `rustls` 踩的坑
                //     同一类问题，现代加密库的默认安全策略比服务器的 SChannel 配置更
                //     严格，直接拒绝协商。
                //   - 10.203.0.113：连 TLS 握手都没到，安全协商阶段服务器直接回绝
                //     SSL/TLS（`SSL_NOT_ALLOWED_BY_SERVER`）——`/tls:seclevel:0` 对
                //     这种情况完全没用，因为根本没走到 TLS 那一步。
                // 两台服务器表现不同，但 `/sec:rdp`（强制走最老、几乎所有 RDP 服务器
                // 都支持的经典 RDP 安全层，不用 TLS）对两台都测通了——不用分别处理，
                // 一个参数覆盖两种失败模式。代价：这条连接不再有 TLS 加密，是接受的
                // 取舍（这些都是内网测试服务器，`/cert:ignore` 已经是同一个信任模型）。
                .arg("/sec:rdp")
                // 这份 `wfreerdp.exe`（Chocolatey `freerdp.portable` 包）是用
                // `WITH_CLIENT_SDL2=ON` 编译的——启动日志里明确打过这行 build 选项，
                // 之前一直没意识到这条信息的分量：意味着它的窗口渲染走的是 SDL2
                // （默认用 Direct3D/OpenGL 之类硬件加速后端），不是想当然以为的老式
                // GDI。换了客户端还是"窗口属性正常、内容纯黑"这个和 mstsc.exe 一模
                // 一样的症状，根因很可能也是同一类：跨进程 `SetParent` 之后硬件加速
                // 表面没有正确挂到新宿主上。SDL2 提供标准环境变量强制走纯软件渲染，
                // 绕开这整条硬件合成路径——`SDL_RENDER_DRIVER=software` 关掉硬件
                // 加速渲染器，`SDL_VIDEODRIVER=windows` 保证用的是原生 Win32 窗口
                // （不是 SDL 自己的其它后端）。
                .env("SDL_RENDER_DRIVER", "software")
                .env("SDL_VIDEODRIVER", "windows")
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| AppError::Internal(format!("启动 wfreerdp.exe 失败：{e}")))?;

            let pid = child.id();
            match find_window_for_process(pid, Duration::from_secs(15)) {
                Some(hwnd) => Ok((child, hwnd.0 as isize, pid)),
                None => {
                    let _ = child.kill();
                    Err(AppError::Connection("等待 RDP 客户端窗口出现超时（15 秒）".into()))
                }
            }
        })
        .await
        .map_err(|e| AppError::Internal(format!("后台任务异常：{e}")))??;
        let hwnd = hwnd_from_isize(hwnd_raw);

        let parent = main_window_hwnd(app_handle)?;
        let scale = window_scale_factor(app_handle)?;
        attach_owned_window(hwnd, parent)?;
        position_window(hwnd, parent, scale, bounds)?;

        let hwnd_state = Arc::new(AtomicIsize::new(hwnd_raw));
        let last_bounds = Arc::new(Mutex::new(Some(bounds)));
        let stop = Arc::new(AtomicBool::new(false));

        // 持续监视这个 PID 的顶层窗口——见本文件顶部"真正要修的 bug"一节，不是连接
        // 时找一次就撒手不管。
        let parent_raw = parent.0 as isize;
        let watcher = std::thread::spawn({
            let app_handle = app_handle.clone();
            let hwnd_state = hwnd_state.clone();
            let last_bounds = last_bounds.clone();
            let stop = stop.clone();
            move || watch_and_reembed(pid, app_handle, hwnd_from_isize(parent_raw), hwnd_state, last_bounds, stop)
        });

        let session_id = Uuid::new_v4();
        self.sessions.lock().unwrap().insert(session_id, EmbeddedSession { child, hwnd: hwnd_state, last_bounds, stop, watcher });
        Ok(session_id)
    }

    pub fn set_bounds(&self, app_handle: &AppHandle, session_id: Uuid, bounds: PanelBounds) -> Result<(), AppError> {
        let parent = main_window_hwnd(app_handle)?;
        let scale = window_scale_factor(app_handle)?;
        let sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get(&session_id) {
            *session.last_bounds.lock().unwrap() = Some(bounds);
            let hwnd = session.hwnd.load(Ordering::Relaxed);
            if hwnd != 0 {
                position_window(hwnd_from_isize(hwnd), parent, scale, bounds)?;
            }
        }
        Ok(())
    }

    pub fn hide(&self, session_id: Uuid) -> Result<(), AppError> {
        let sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get(&session_id) {
            let hwnd = session.hwnd.load(Ordering::Relaxed);
            if hwnd != 0 {
                unsafe {
                    let _ = ShowWindow(hwnd_from_isize(hwnd), SW_HIDE);
                }
            }
        }
        Ok(())
    }

    pub fn show(&self, app_handle: &AppHandle, session_id: Uuid, bounds: PanelBounds) -> Result<(), AppError> {
        let parent = main_window_hwnd(app_handle)?;
        let scale = window_scale_factor(app_handle)?;
        let sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get(&session_id) {
            *session.last_bounds.lock().unwrap() = Some(bounds);
            let hwnd = session.hwnd.load(Ordering::Relaxed);
            if hwnd != 0 {
                let h = hwnd_from_isize(hwnd);
                position_window(h, parent, scale, bounds)?;
                unsafe {
                    let _ = ShowWindow(h, SW_SHOW);
                }
            }
        }
        Ok(())
    }

    pub fn status(&self, session_id: Uuid) -> Result<RdpStatus, AppError> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions.get(&session_id).ok_or_else(|| AppError::NotFound("RDP 会话不存在".into()))?;
        let hwnd = session.hwnd.load(Ordering::Relaxed);
        let (state, diagnostics) = if hwnd == 0 {
            (RdpStatusState::Connecting, "尚未找到 RDP 客户端窗口".to_string())
        } else {
            let h = hwnd_from_isize(hwnd);
            let visible = unsafe { IsWindowVisible(h) }.as_bool();
            let mut rect = RECT::default();
            let rect_text = if unsafe { GetWindowRect(h, &mut rect) }.is_ok() {
                format!("{}x{}", rect.right - rect.left, rect.bottom - rect.top)
            } else {
                "?".to_string()
            };
            (RdpStatusState::Connected, format!("嵌入窗口 hwnd=0x{hwnd:X} 可见={visible} 尺寸={rect_text}"))
        };
        Ok(RdpStatus { state, reason: None, diagnostics })
    }

    pub fn disconnect(&self, session_id: Uuid) -> Result<(), AppError> {
        let session = self.sessions.lock().unwrap().remove(&session_id);
        if let Some(mut session) = session {
            session.stop.store(true, Ordering::Relaxed);
            let _ = session.child.kill();
            let _ = session.child.wait();
            // 不 join 监视线程，避免阻塞——它下一次轮询周期内会自己看到 stop 标记
            // 退出，反正进程已经被杀了，线程活多 400ms 也不会有任何副作用。
        }
        Ok(())
    }

    /// 关闭所有还开着的 RDP 会话（应用退出/工作区切换时调用），避免留下孤儿
    /// wfreerdp 进程——和 `browser::close_all` 是同一个收尾职责。
    pub fn disconnect_all(&self) -> Result<(), AppError> {
        let mut sessions = self.sessions.lock().unwrap();
        for (_, mut session) in sessions.drain() {
            session.stop.store(true, Ordering::Relaxed);
            let _ = session.child.kill();
            let _ = session.child.wait();
        }
        Ok(())
    }
}

/// 找 `wfreerdp.exe`：便携版发布时它和 `roc_desk.exe` 放在同一个目录（见
/// `scripts/build-portable.ps1`），生产环境优先找这里；`cargo run`/`cargo tauri dev`
/// 场景下可执行文件在 `target/debug` 之类目录、旁边不会有这个文件，退回到编译期
/// 就确定的仓库内 `vendor/` 目录（只在 dev 构建里有意义，便携版不依赖这条路径）。
fn find_wfreerdp_exe() -> Result<PathBuf, AppError> {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()));
    if let Some(dir) = &exe_dir {
        let candidate = dir.join("wfreerdp.exe");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    let dev_candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("vendor").join("wfreerdp.exe");
    if dev_candidate.is_file() {
        return Ok(dev_candidate);
    }
    Err(AppError::Internal(
        "找不到 wfreerdp.exe（应该和 roc_desk.exe 放在同一目录，或者在开发环境的仓库 vendor/ 目录下）".into(),
    ))
}

fn main_window_hwnd(app_handle: &AppHandle) -> Result<HWND, AppError> {
    app_handle
        .get_window("main")
        .ok_or_else(|| AppError::Internal("找不到主窗口".into()))?
        .hwnd()
        .map_err(|e| AppError::Internal(format!("获取主窗口句柄失败：{e}")))
}

fn window_scale_factor(app_handle: &AppHandle) -> Result<f64, AppError> {
    app_handle
        .get_window("main")
        .ok_or_else(|| AppError::Internal("找不到主窗口".into()))?
        .scale_factor()
        .map_err(|e| AppError::Internal(format!("获取窗口缩放比例失败：{e}")))
}

/// 摘掉标题栏/边框/系统菜单/最大最小化按钮，把 `owner` 设成 `child` 的属主窗口——
/// 用 `GWLP_HWNDPARENT`，不是 `SetParent`，刻意不转成 `WS_CHILD` 子窗口（见本文件
/// 顶部"内嵌手法的最后一次调整"一节：转子窗口在真机上复现过内容纯黑，换任何渲染
/// 后端都躲不掉，根因是 DWM 对子窗口的合成方式，不是配置问题）。保留顶层窗口身份
/// 让 DWM 按对待普通顶层窗口完全相同的路径合成它。`WS_EX_TOOLWINDOW` 是双保险，
/// 属主窗口本来默认就不出现在 Alt+Tab/任务栏，这里再显式加固一下。
fn attach_owned_window(child: HWND, owner: HWND) -> Result<(), AppError> {
    unsafe {
        let style = WINDOW_STYLE(GetWindowLongPtrW(child, GWL_STYLE) as u32);
        let stripped = style & !(WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX);
        SetWindowLongPtrW(child, GWL_STYLE, (stripped | WS_VISIBLE).0 as isize);

        let ex_style = WINDOW_EX_STYLE(GetWindowLongPtrW(child, GWL_EXSTYLE) as u32);
        SetWindowLongPtrW(child, GWL_EXSTYLE, ((ex_style | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW).0 as isize);

        SetWindowLongPtrW(child, GWLP_HWNDPARENT, owner.0 as isize);

        // 只是让上面两次样式改动生效（`SWP_FRAMECHANGED`），不移动/不缩放/不改
        // z-order——真正的位置由 `position_window` 负责。
        let _ = SetWindowPos(child, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    }
    Ok(())
}

/// `bounds` 是前端 `getBoundingClientRect()` 拿到的逻辑像素（CSS px，相对 roc_desk
/// 主窗口客户区），Win32 的 `SetWindowPos` 要物理像素——按窗口当前 DPI 缩放比例
/// 换算一次。`child` 现在是独立顶层窗口（不再是子窗口），`SetWindowPos` 的坐标是
/// 屏幕坐标，不再是"相对父窗口客户区"，所以要先用 `ClientToScreen` 把主窗口客户区
/// 原点换算成屏幕坐标，再加上面板的相对偏移。
fn position_window(child: HWND, owner: HWND, scale: f64, bounds: PanelBounds) -> Result<(), AppError> {
    let mut origin = POINT::default();
    unsafe {
        ClientToScreen(owner, &mut origin)
            .ok()
            .map_err(|e| AppError::Internal(format!("ClientToScreen 失败：{e}")))?;
    }
    let x = origin.x + (bounds.x * scale).round() as i32;
    let y = origin.y + (bounds.y * scale).round() as i32;
    let width = (bounds.width * scale).round().max(0.0) as i32;
    let height = (bounds.height * scale).round().max(0.0) as i32;
    unsafe {
        SetWindowPos(child, None, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE)
            .map_err(|e| AppError::Internal(format!("SetWindowPos 失败：{e}")))?;
    }
    Ok(())
}

/// 后台监视线程：整个会话生命周期内持续轮询这个 PID 当前的顶层窗口，一旦和已内嵌
/// 的不一样就重新摘边框/`SetParent`/定位——见本文件顶部"真正要修的 bug"一节。
fn watch_and_reembed(
    pid: u32,
    app_handle: AppHandle,
    parent: HWND,
    hwnd_state: Arc<AtomicIsize>,
    last_bounds: Arc<Mutex<Option<PanelBounds>>>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(400));
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Some(found) = find_window_for_process(pid, Duration::ZERO) else { continue };
        let found_raw = found.0 as isize;
        if found_raw == hwnd_state.load(Ordering::Relaxed) {
            continue;
        }
        if attach_owned_window(found, parent).is_err() {
            continue;
        }
        if let (Ok(scale), Some(bounds)) = (window_scale_factor(&app_handle), *last_bounds.lock().unwrap()) {
            let _ = position_window(found, parent, scale, bounds);
        }
        hwnd_state.store(found_raw, Ordering::Relaxed);
    }
}

struct EnumState {
    target_pid: u32,
    /// 目前见过的最大窗口（句柄, 面积）——见 `enum_windows_callback` 为什么按面积
    /// 挑而不是按"有没有标题"挑。
    best: Option<(HWND, i64)>,
}

unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = unsafe { &mut *(lparam.0 as *mut EnumState) };
    let mut pid: u32 = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32));
    }
    if pid != state.target_pid {
        return BOOL(1); // 继续枚举
    }
    // 按"这个 PID 名下面积最大的窗口"挑，不按"有没有标题"挑——最早的实现是按标题
    // 过滤的（证书警告/连接状态这类小对话框通常有标题，内部隐藏辅助窗口通常没有），
    // 但那是只在连接发起那一刻查一次的写法。现在要在整个会话生命周期里持续判断
    // "当前该嵌哪个窗口"，如果真正渲染桌面的那个窗口标题是空的（真机联调证实过
    // 确实会是空的），按标题过滤会永远找不到它，卡在之前嵌入的那个小窗口上——
    // 表现正是真机复现过的"看到在连，但最终是一片空白"。桌面渲染窗口天然比证书
    // 警告/连接状态这类小对话框大，用面积挑是更可靠的信号，不依赖标题内容。
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return BOOL(1);
    }
    let area = (rect.right - rect.left) as i64 * (rect.bottom - rect.top) as i64;
    if area <= 0 {
        return BOOL(1);
    }
    if state.best.map(|(_, best_area)| area > best_area).unwrap_or(true) {
        state.best = Some((hwnd, area));
    }
    BOOL(1) // 继续枚举——要扫完全部才能确定"最大"，不能找到一个就停
}

/// 找这个 PID 当前的顶层窗口。`timeout` 为零时只做一次 `EnumWindows` 扫描就返回
/// （给 `watch_and_reembed` 每个轮询周期用），非零时按 200ms 间隔重试直到超时
/// （给 `connect()` 等待第一个窗口出现用）。
fn find_window_for_process(pid: u32, timeout: Duration) -> Option<HWND> {
    let deadline = Instant::now() + timeout;
    loop {
        let mut state = EnumState { target_pid: pid, best: None };
        let lparam = LPARAM(&mut state as *mut EnumState as isize);
        unsafe {
            let _ = EnumWindows(Some(enum_windows_callback), lparam);
        }
        if let Some((hwnd, _)) = state.best {
            return Some(hwnd);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
