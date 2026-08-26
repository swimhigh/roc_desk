//! RDP 远程桌面（远程工具模式，DESIGN.md §3.9）。
//!
//! **不自己实现 RDP 协议**——这是用户 2026-08-25 明确要求的方向调整，背后是真实
//! 踩出来的几个坑，一路收窄到现在这个方案：
//!   1. 纯 Rust 的 `rustls` 只支持现代 TLS1.2/1.3 + 前向保密套件，遇到 SChannel
//!      策略配置得比较老/受限的 Windows 服务器（能被 mstsc.exe/MobaXterm 正常连上）
//!      时，ClientHello 里没有一个套件对方能接受，直接被 TCP RST 掉。
//!   2. 换 `native-tls` 绕开①之后，又踩到当时用的协议库 `IronRDP` 本身没实现"并发
//!      会话数超限时展示 Windows 会话选择/踢人界面"这个能力——查过源码，服务器发的
//!      是专门的 `ServerSetErrorInfo` 协议包，`ironrdp-session` 收到就直接断线，
//!      压根没有走到"渲染成画面给用户选"这条路径，这是库没做，不是配置问题。
//!   3. 退而求其次拉起系统自带 `mstsc.exe`、`SetParent` 把它的窗口嵌进主窗口，这条
//!      路能力上是够的（会话选择/证书警告都是 mstsc 自己在画），但 mstsc.exe 一个
//!      进程生命周期里会先后创建好几个顶层窗口（证书警告对话框 → 可能的会话选择
//!      对话框 → 真正的桌面画面窗口），我们只在连接发起那一刻 `EnumWindows` 探测
//!      了一次，抓到第一个窗口嵌进来之后就不再追踪——用户实测：证书警告能看到，
//!      但对话框消失后画面窗口没跟着嵌进来，等于卡在半途。
//!
//! 用户原话："能改成和MobaXterm一样就改成和它一样的搞法"、"直接上 ActiveX 完整
//! 方案"——查证过 MobaXterm 自己就是走这条路：不重新实现协议，直接以 ActiveX 方式
//! 承载 Windows 自带的 RDP 客户端控件（`mstscax.dll` 的 `MsTscAx`/`IMsRdpClient`），
//! 这正是 mstsc.exe 内部用来渲染桌面的同一个控件，只是 mstsc.exe 把它套在自己开的
//! 顶层窗口里，而我们把它套进 roc_desk 自己的窗口——控件从创建到断开只有这一个
//! 窗口，不会像方案③那样中途切换成另一个顶层窗口，从根上不会再复现"嵌了证书对话框
//! 之后就卡住"的问题。
//!
//! ## 实现要点
//! ActiveX 控件是"就地激活"（in-place activation）模型，不是简单 `CoCreateInstance`
//! 完就能用——控件需要一个实现了 `IOleClientSite`/`IOleInPlaceSite`/`IOleInPlaceFrame`
//! 的"容器"对象跟它协商宿主窗口、摆放矩形，参见 [`ole_container`] 模块；控件本身的
//! COM 接口（`IMsTscAx`/`IMsRdpClient`/`IMsTscNonScriptable`）不在 `windows` crate
//! 的预生成绑定里（那份绑定只覆盖 Win32 核心 API），是照着 MSVC `#import` 生成的
//! 权威类型库头文件手抄的，参见 [`mstscax`] 模块开头的说明。
//!
//! 控件不直接挂在 roc_desk 主窗口下，中间隔了一层我们自己创建的**宿主窗口**
//! （`create_host_window`）：控件永远铺满宿主的客户区、坐标恒为 (0,0,w,h)，要挪
//! 位置就挪宿主。这是标准 ActiveX 宿主做法（ATL 的 `CAxWindow` 同理），一次解决
//! 三件事——坐标系不再有歧义（不用把"整个界面里的绝对位置"这种它不该关心的信息
//! 传给控件）、显示/隐藏/Z 序由我们自己的窗口说了算、控件窗口和它父窗口同在一个
//! STA 线程上使消息路由更规矩。
//!
//! COM 的这一套接口要求在同一个 STA（单线程单元）线程上创建、调用、跑消息循环，
//! 所以每个 RDP 会话背后是一个独立的 `std::thread`：`CoInitializeEx` → 创建控件 →
//! `DoVerb` 就地激活 → 设置 Server/UserName/ClearTextPassword 等属性 → `Connect()`
//! → 进入 `GetMessage`/`DispatchMessage` 消息循环，直到收到我们发的 `WM_QUIT`（对应
//! `disconnect()`）才退出、释放 COM 对象。控件激活后会创建自己的真实子窗口（父窗口
//! 是我们传给 `DoVerb` 的 roc_desk 主窗口），这个 HWND 之后可以像任何子窗口一样从
//! 任意线程调用 `SetWindowPos`/`ShowWindow`（Windows 保证跨线程投递到窗口所在线程
//! 处理，这是标准用法，不需要都切回 STA 线程）——bounds/show/hide 因此可以保持同步
//! 快速调用，不必每次都跟 STA 线程打交道。
//!
//! ## 第 4 个坑：`CoCreateInstance` 报 `REGDB_E_CLASSNOTREG`
//! 真机联调时踩到的——`mstscax.dll` 文件本身在 `System32`/`SysWOW64` 都在，但注册表
//! 里 `HKCR\CLSID\{1FB464C8-...}` 两边（64/32 位）都是空的，从来没人跑过
//! `regsvr32`。让用户在能用 RDP 前先以管理员身份手动注册这个系统 DLL 是不可接受的
//! 前置门槛，而且是全局系统级改动。MobaXterm 是单文件绿色版、不需要这种步骤就能用
//! RDP，参照的是 Windows 的"免注册 COM"（Registration-Free COM）机制：运行时用一份
//! 清单（manifest）XML 临时声明 `clsid → dll → threadingModel` 映射、`CreateActCtxW`/
//! `ActivateActCtx` 把它压进激活上下文栈顶，`CoCreateInstance` 会先查激活上下文再查
//! 注册表——查到了就直接按清单创建，完全不碰注册表，不需要管理员权限，也不管这台机器
//! 上这个 DLL 有没有被注册过。实现见 [`actctx`] 模块。
//!
//! ## 连接状态：轮询代替真事件下沉
//! 没有实现 `IMsTscAxEvents` 事件下沉（DISPID 需要要么硬猜要么运行时 `ITypeInfo`
//! 反射，风险/工作量都不小）——改用轮询：`run_message_loop` 每 250ms 调一次
//! `get_Connected()`/`get_ExtendedDisconnectReason()`，结果存进 `RdpSession` 上的
//! `AtomicU8`/`AtomicI32`，前端 `rdp_status` 命令读取、`RdpView.tsx` 定时拉取显示。
//! 不是真正的事件推送（有 250ms 的检测延迟），但足够让前端从"盲目乐观地认为
//! connected"变成能看到真实的 connecting/connected/error 状态和断线原因码。
//!
//! 没做事件下沉还有一个更隐蔽的后果：控件遇到需要用户确认身份（比如自签名证书）
//! 的服务器时，正常会触发 `OnReceivedTSPublicKey` 事件问容器"要不要继续"——没有
//! 事件接收者，控件很可能就静默卡在这一步，界面表现为"连接调用全部成功、不报错，
//! 但一直黑屏"。这台测试服务器（10.203.0.111）已知会触发这个流程，所以在
//! `activate_control` 里把 `IMsRdpClientAdvancedSettings2::AuthenticationLevel`
//! 设成 0（"直接连接，不做身份校验"），从根上避开这条没有接收者的死路——这个属性
//! 挂在一个巨大到不值得手抄 vtable 的 dual interface 上，改用 `IDispatch` 晚绑定
//! （`GetIDsOfNames`+`Invoke`）设置，见 `disable_authentication_warning`。
//!
//! ## 其它已知缺口（如实记录）
//!   - 只在 Windows 上能用（`[target.'cfg(windows)'.dependencies]`），但 roc_desk
//!     本来就是 Windows 专属客户端，这不是新增的限制。
//!   - 应用被强制杀掉（不是走正常关闭流程）的话，STA 线程和它持有的 COM 对象会
//!     随进程一起消失，这本来就是期望行为，不需要特别处理。

mod actctx;
mod mstscax;
mod ole_container;

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicI32, AtomicU8, Ordering};
use std::thread::JoinHandle;

use serde::Deserialize;
use tauri::{AppHandle, Manager};
use uuid::Uuid;
use windows::core::{Interface, BSTR, GUID, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, DISPATCH_PROPERTYPUT, DISPPARAMS,
};
use windows::Win32::System::Ole::{IOleClientSite, IOleObject, OleSetContainedObject, DISPID_PROPERTYPUT, OLEIVERB_INPLACEACTIVATE};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Variant::{VARIANT, VARIANT_0_0, VARIANT_0_0_0, VT_UI4};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowRect, IsWindow, IsWindowVisible,
    PostThreadMessageW, RegisterClassW, SetWindowPos, ShowWindow, TranslateMessage, HWND_TOP, MSG, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SW_HIDE, SW_SHOW, WINDOW_EX_STYLE, WM_QUIT, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_VISIBLE,
};

use crate::connection::{ConnectionManager, ConnectionProfile, Protocol};
use crate::error::AppError;
use mstscax::{IMsRdpClient, IMsTscNonScriptable, CLSID_MSTSCAX};
use ole_container::RdpOleContainer;

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PanelBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpStatusState { Connecting, Connected, Disconnected, Error }

#[derive(Debug, Clone, serde::Serialize)]
pub struct RdpStatus {
    pub state: RdpStatusState,
    pub reason: Option<i32>,
    /// 原生窗口的实时体检结果（句柄是否有效/是否可见/实际屏幕矩形/控件自报的
    /// Connected 值）——黑屏这个现象本身分不清是"根本没连上"还是"连上了但窗口被
    /// 摆错地方/被盖住了"，把这些事实直接摊开给前端显示，才能不靠猜定位问题。
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

/// STA 线程握手结果：控件创建/激活/连接全部发起成功后，把控件自己的窗口句柄和
/// 线程 ID 带回主线程——前者用来后续摆放位置/显示隐藏，后者用来发 `WM_QUIT` 触发
/// 消息循环退出。
struct StaHandshake {
    thread_id: u32,
    /// 我们自己创建的宿主窗口——摆位置/显示隐藏都是操作它，控件永远铺满它的客户区。
    host_hwnd: isize,
    /// ActiveX 控件自己创建的窗口（宿主窗口的子窗口），只用于体检诊断。
    control_hwnd: isize,
}

struct RdpSession {
    thread_id: u32,
    /// 我们自己创建的宿主窗口（roc_desk 主窗口的子窗口）——所有摆位置/显示隐藏都
    /// 操作它，ActiveX 控件铺满它的客户区。`HWND` 内部是裸指针不是 `Send`，存成
    /// `isize` 绕开限制，只当不透明句柄用。
    hwnd: isize,
    /// 控件自己的窗口句柄，只在体检诊断里用到。
    control_hwnd: isize,
    /// STA 线程的 `JoinHandle`——`disconnect()` 发完 `WM_QUIT` 后不等它退出
    /// （线程会自己清理 COM 对象、退出），所以这个字段目前只用来在丢弃 `RdpSession`
    /// 时避免编译器警告"从未使用"；真要等待可以在这基础上加 `.join()`。
    #[allow(dead_code)]
    join: JoinHandle<()>,
    state: Arc<AtomicU8>,
    reason: Arc<AtomicI32>,
    /// 控件 `get_Connected()` 的原始返回值（0=未连接，1=已连接，2=连接中），
    /// 直接透出来做诊断——比我们归纳过的 `state` 更接近控件自己的说法。
    connected_raw: Arc<AtomicI32>,
}

/// STA 线程往外汇报状态用的一组原子变量——打包成一个结构体只是为了别让
/// `run_sta_session` 的参数列表继续膨胀（已经有 8 个参数了）。
struct SessionProbes {
    state: Arc<AtomicU8>,
    reason: Arc<AtomicI32>,
    connected_raw: Arc<AtomicI32>,
}

fn hwnd_from_isize(v: isize) -> HWND {
    HWND(v as *mut core::ffi::c_void)
}

/// RDP 会话登记表（远程工具模式，DESIGN.md §3.9）。和 SSH 的 `SshConnectionPool`
/// 不是一回事——这里没有"多路复用同一条连接"的概念，每次"打开 RDP"都是一个独立的
/// STA 线程 + 一个独立的 ActiveX 控件实例，key 是 session_id 不是 profile_id。
pub struct RdpSessionManager {
    sessions: Mutex<HashMap<Uuid, RdpSession>>,
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
        let domain = opts.domain.unwrap_or_default();
        let width = opts.width.unwrap_or(1280);
        let height = opts.height.unwrap_or(800);
        // MsTscAx 的 `Server` 属性和 mstsc.exe 的 `/v:` 参数共用底层引擎，接受同样的
        // `host` 或 `host:port` 写法（3389 是默认端口，不用带）。
        let server = if profile.port == 3389 { profile.host.clone() } else { format!("{}:{}", profile.host, profile.port) };
        let username = profile.username.clone();

        let parent = main_window_hwnd(app_handle)?;
        // `HWND` 内部是裸指针，不是 `Send`，过不了 `thread::spawn` 的闭包边界——
        // 转成 `isize` 带过去，进线程里再用 `hwnd_from_isize` 转回来，就是个不透明
        // 句柄的搬运，不涉及跨线程访问它指向的内存。
        let parent_raw = parent.0 as isize;
        let scale = window_scale_factor(app_handle)?;
        let rect = bounds_to_rect(scale, bounds);

        let state = Arc::new(AtomicU8::new(0));
        let reason = Arc::new(AtomicI32::new(0));
        let connected_raw = Arc::new(AtomicI32::new(-1));
        let (tx, rx) = std::sync::mpsc::channel::<Result<StaHandshake, AppError>>();
        let probes = SessionProbes {
            state: state.clone(),
            reason: reason.clone(),
            connected_raw: connected_raw.clone(),
        };
        let join = std::thread::spawn(move || {
            run_sta_session(hwnd_from_isize(parent_raw), rect, server, domain, username, secret, width, height, probes, tx);
        });

        // STA 线程的创建/激活/连接全是同步阻塞的 COM 调用，`rx.recv()` 本身也是阻塞
        // 等待——扔到 tokio 阻塞线程池，不占用 async 运行时的 worker 线程。
        let handshake = tokio::task::spawn_blocking(move || rx.recv())
            .await
            .map_err(|e| AppError::Internal(format!("后台任务异常：{e}")))?
            .map_err(|_| AppError::Internal("RDP 会话线程未完成握手就退出了".into()))??;

        let session_id = Uuid::new_v4();
        self.sessions.lock().unwrap().insert(
            session_id,
            RdpSession {
                thread_id: handshake.thread_id,
                hwnd: handshake.host_hwnd,
                control_hwnd: handshake.control_hwnd,
                join,
                state,
                reason,
                connected_raw,
            },
        );
        Ok(session_id)
    }

    pub fn set_bounds(&self, app_handle: &AppHandle, session_id: Uuid, bounds: PanelBounds) -> Result<(), AppError> {
        let scale = window_scale_factor(app_handle)?;
        let sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get(&session_id) {
            position_window(hwnd_from_isize(session.hwnd), hwnd_from_isize(session.control_hwnd), scale, bounds)?;
        }
        Ok(())
    }

    pub fn hide(&self, session_id: Uuid) -> Result<(), AppError> {
        let sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get(&session_id) {
            unsafe {
                let _ = ShowWindow(hwnd_from_isize(session.hwnd), SW_HIDE);
            }
        }
        Ok(())
    }

    pub fn show(&self, app_handle: &AppHandle, session_id: Uuid, bounds: PanelBounds) -> Result<(), AppError> {
        let scale = window_scale_factor(app_handle)?;
        let sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get(&session_id) {
            let hwnd = hwnd_from_isize(session.hwnd);
            let control = hwnd_from_isize(session.control_hwnd);
            position_window(hwnd, control, scale, bounds)?;
            unsafe {
                let _ = ShowWindow(control, SW_SHOW);
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
        }
        Ok(())
    }

    pub fn disconnect(&self, session_id: Uuid) -> Result<(), AppError> {
        let session = self.sessions.lock().unwrap().remove(&session_id);
        if let Some(session) = session {
            post_quit(session.thread_id);
        }
        Ok(())
    }

    pub fn status(&self, session_id: Uuid) -> Result<RdpStatus, AppError> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions.get(&session_id).ok_or_else(|| AppError::NotFound("RDP 会话不存在".into()))?;
        let state = match session.state.load(Ordering::Relaxed) {
            1 => RdpStatusState::Connected,
            2 => RdpStatusState::Disconnected,
            3 => RdpStatusState::Error,
            _ => RdpStatusState::Connecting,
        };
        let reason = match session.reason.load(Ordering::Relaxed) { 0 => None, value => Some(value) };
        let diagnostics = describe_windows(session.hwnd, session.control_hwnd, session.connected_raw.load(Ordering::Relaxed));
        Ok(RdpStatus { state, reason, diagnostics })
    }

    /// 关闭所有还开着的 RDP 会话（应用退出/工作区切换时调用）——和 `browser::close_all`
    /// 是同一个收尾职责。
    pub fn disconnect_all(&self) -> Result<(), AppError> {
        let mut sessions = self.sessions.lock().unwrap();
        for (_, session) in sessions.drain() {
            post_quit(session.thread_id);
        }
        Ok(())
    }
}

fn post_quit(thread_id: u32) {
    unsafe {
        let _ = PostThreadMessageW(thread_id, WM_QUIT, windows::Win32::Foundation::WPARAM(0), windows::Win32::Foundation::LPARAM(0));
    }
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

/// `bounds` 是前端 `getBoundingClientRect()` 拿到的逻辑像素（CSS px），Win32 要
/// 物理像素——按窗口当前 DPI 缩放比例换算一次，Tauri 不会替原生 Win32 调用自动
/// 转换，得自己乘一遍。
fn bounds_to_rect(scale: f64, bounds: PanelBounds) -> RECT {
    let x = (bounds.x * scale).round() as i32;
    let y = (bounds.y * scale).round() as i32;
    let width = (bounds.width * scale).round().max(0.0) as i32;
    let height = (bounds.height * scale).round().max(0.0) as i32;
    RECT { left: x, top: y, right: x + width, bottom: y + height }
}

/// 摆放宿主窗口，并让控件铺满宿主的客户区。控件是宿主的子窗口，宿主移动/缩放时
/// 控件不会自动跟着变大小，所以这两步必须成对做。
fn position_window(host: HWND, control: HWND, scale: f64, bounds: PanelBounds) -> Result<(), AppError> {
    let rect = bounds_to_rect(scale, bounds);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    unsafe {
        SetWindowPos(host, Some(HWND_TOP), rect.left, rect.top, width, height, SWP_NOACTIVATE | SWP_FRAMECHANGED)
            .map_err(|e| AppError::Internal(format!("SetWindowPos（宿主窗口）失败：{e}")))?;
        if IsWindow(Some(control)).as_bool() {
            SetWindowPos(control, Some(HWND_TOP), 0, 0, width, height, SWP_NOACTIVATE)
                .map_err(|e| AppError::Internal(format!("SetWindowPos（控件窗口）失败：{e}")))?;
        }
    }
    Ok(())
}

/// 把两个原生窗口的实时状态拍平成一行文本给前端显示——黑屏时这是区分"根本没连上"
/// 和"连上了但窗口没摆对/被盖住"的唯一依据，不写日志文件是因为绿色版用户不会去翻
/// 日志，直接显示在失败提示里才有用。
fn describe_windows(host: isize, control: isize, connected_raw: i32) -> String {
    let host_hwnd = hwnd_from_isize(host);
    let control_hwnd = hwnd_from_isize(control);
    unsafe {
        let mut host_rect = RECT::default();
        let host_ok = IsWindow(Some(host_hwnd)).as_bool();
        let host_visible = host_ok && IsWindowVisible(host_hwnd).as_bool();
        if host_ok {
            let _ = GetWindowRect(host_hwnd, &mut host_rect);
        }
        let mut control_rect = RECT::default();
        let control_ok = IsWindow(Some(control_hwnd)).as_bool();
        let control_visible = control_ok && IsWindowVisible(control_hwnd).as_bool();
        if control_ok {
            let _ = GetWindowRect(control_hwnd, &mut control_rect);
        }
        let connected_text = match connected_raw {
            -1 => "尚未探测".to_string(),
            0 => "0=未连接".to_string(),
            1 => "1=已连接".to_string(),
            2 => "2=连接中".to_string(),
            other => format!("{other}=未知"),
        };
        format!(
            "宿主窗口 hwnd=0x{host:X} 有效={host_ok} 可见={host_visible} 屏幕矩形=({},{})-({},{}); \
             控件窗口 hwnd=0x{control:X} 有效={control_ok} 可见={control_visible} 屏幕矩形=({},{})-({},{}); \
             控件自报 Connected={connected_text}",
            host_rect.left,
            host_rect.top,
            host_rect.right,
            host_rect.bottom,
            control_rect.left,
            control_rect.top,
            control_rect.right,
            control_rect.bottom,
        )
    }
}

/// 创建承载 ActiveX 控件的宿主窗口。
///
/// 之前是直接把控件挂到 roc_desk 主窗口下、并且把"控件该摆在哪"这个矩形用主窗口
/// 客户区坐标传给它——能跑，但把两件事混在了一起：控件既要负责自己的内部布局，又
/// 要正确落在一个它并不了解的坐标系里的某个位置。标准的 ActiveX 宿主做法（ATL 的
/// `CAxWindow`、MFC 的控件容器都是如此）是先建一个自己的普通窗口当"画框"，控件
/// 永远铺满画框的客户区、坐标恒为 (0,0,w,h)，需要挪位置时挪画框——控件完全不需要
/// 知道自己在整个界面里的绝对位置。这样做同时解决三件事：坐标系不再有歧义；显示/
/// 隐藏/Z 序都由我们自己这个窗口说了算，不依赖控件配合；控件窗口的父窗口和它自己
/// 在同一个 STA 线程上，消息路由更规矩。
fn create_host_window(parent: HWND, rect: RECT) -> Result<HWND, AppError> {
    const CLASS_NAME: PCWSTR = windows::core::w!("RocDeskRdpHost");
    static REGISTER: std::sync::Once = std::sync::Once::new();
    let mut register_error: Option<String> = None;
    REGISTER.call_once(|| unsafe {
        let instance = match GetModuleHandleW(None) {
            Ok(h) => h,
            Err(e) => {
                register_error = Some(format!("GetModuleHandleW 失败：{e}"));
                return;
            }
        };
        let class = WNDCLASSW {
            lpfnWndProc: Some(host_wnd_proc),
            hInstance: instance.into(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            register_error = Some(format!("RegisterClassW 失败：{}", windows::core::Error::from_win32()));
        }
    });
    if let Some(e) = register_error {
        return Err(AppError::Internal(format!("注册 RDP 宿主窗口类失败：{e}")));
    }

    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS_NAME,
            PCWSTR::null(),
            // `WS_CLIPCHILDREN`：宿主自己不画任何东西，全部交给铺在上面的控件，
            // 避免宿主的背景擦除和控件绘制打架产生闪烁。
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
            rect.left,
            rect.top,
            (rect.right - rect.left).max(1),
            (rect.bottom - rect.top).max(1),
            Some(parent),
            None,
            None,
            None,
        )
        .map_err(|e| AppError::Internal(format!("创建 RDP 宿主窗口失败：{e}")))
    }
}

unsafe extern "system" fn host_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// 在独立 STA 线程上跑完的整个 ActiveX 生命周期：初始化 COM → 创建/激活控件 →
/// 设置连接参数 → 发起连接 → 把结果（控件窗口句柄或错误）回传给 `connect()` →
/// 成功的话继续跑消息循环直到收到 `WM_QUIT`。
#[allow(clippy::too_many_arguments)]
fn run_sta_session(
    parent: HWND,
    rect: RECT,
    server: String,
    domain: String,
    username: String,
    password: String,
    width: u16,
    height: u16,
    probes: SessionProbes,
    tx: Sender<Result<StaHandshake, AppError>>,
) {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            let _ = tx.send(Err(AppError::Internal(format!("CoInitializeEx 失败：{hr:?}"))));
            return;
        }

        let thread_id = GetCurrentThreadId();
        match activate_control(parent, rect, &server, &domain, &username, &password, width, height) {
            Ok((host_hwnd, control_hwnd, rdp_client)) => {
                // 就地激活协议本身不保证控件的窗口一定带 `WS_VISIBLE`——不要只靠前端
                // 之后调 `rdp_show` 才让它显形，这里直接兜底显式显示一次，和旧的
                // mstsc.exe 内嵌方案里 `embed_window` 显式加 `WS_VISIBLE` 是同一个
                // 用意（真实复现过的 bug：控件创建/连接全部成功、不报错，但界面上
                // 什么都看不到）。
                let _ = ShowWindow(control_hwnd, SW_SHOW);
                let _ = ShowWindow(host_hwnd, SW_SHOW);
                let _ = tx.send(Ok(StaHandshake {
                    thread_id,
                    host_hwnd: host_hwnd.0 as isize,
                    control_hwnd: control_hwnd.0 as isize,
                }));
                run_message_loop(&rdp_client, &probes);
            }
            Err(e) => {
                let _ = tx.send(Err(e));
            }
        }

        CoUninitialize();
    }
}

/// 创建宿主窗口和 `MsTscAx` 控件、完成就地激活、设置连接属性并发起连接，返回
/// （宿主窗口，控件窗口，控件接口）。全程只在调用它的 STA 线程上执行。
#[allow(clippy::too_many_arguments)]
unsafe fn activate_control(
    parent: HWND,
    rect: RECT,
    server: &str,
    domain: &str,
    username: &str,
    password: &str,
    width: u16,
    height: u16,
) -> Result<(HWND, HWND, IMsRdpClient), AppError> {
    // 免注册 COM：不假设 `mstscax.dll` 在这台机器的注册表里已经有 CLSID 条目——
    // 真实测试机上就是文件在但从没 `regsvr32` 过，`CoCreateInstance` 直接
    // `REGDB_E_CLASSNOTREG`。参照 MobaXterm 单文件绿色版"不需要管理员权限前置
    // 步骤就能用 RDP"的路子，用激活上下文把 clsid→dll 映射临时压给这一次
    // `CoCreateInstance` 查，查完就还原，见 `actctx` 模块顶部说明。
    let _actctx = actctx::ActivationGuard::activate()
        .map_err(|e| AppError::Internal(format!("创建 MsTscAx 免注册 COM 激活上下文失败：{e}")))?;
    let ole_object: IOleObject = CoCreateInstance(&CLSID_MSTSCAX, None, CLSCTX_INPROC_SERVER)
        .map_err(|e| AppError::Internal(format!("创建 MsTscAx 控件失败：{e}")))?;

    // 先建自己的宿主窗口，控件挂在它下面、坐标恒为 (0,0,w,h)——见
    // `create_host_window` 上面关于为什么要多这一层的说明。
    let host_hwnd = create_host_window(parent, rect)?;
    let host_rect = RECT { left: 0, top: 0, right: (rect.right - rect.left).max(1), bottom: (rect.bottom - rect.top).max(1) };

    let container = RdpOleContainer::new(host_hwnd, host_rect);
    let client_site: IOleClientSite = container.into();

    ole_object.SetClientSite(&client_site).map_err(|e| AppError::Internal(format!("SetClientSite 失败：{e}")))?;
    OleSetContainedObject(&ole_object, true).map_err(|e| AppError::Internal(format!("OleSetContainedObject 失败：{e}")))?;

    let mut pos_rect = host_rect;
    ole_object
        .DoVerb(OLEIVERB_INPLACEACTIVATE.0, core::ptr::null(), &client_site, 0, host_hwnd, &mut pos_rect as *mut RECT)
        .map_err(|e| AppError::Internal(format!("控件就地激活（DoVerb）失败：{e}")))?;

    // 就地激活完成后，`IOleInPlaceObject`（继承自 `IOleWindow`）能拿到控件自己
    // 真正创建出来的那个子窗口——从这一步开始，这个句柄在整个会话生命周期里
    // 不会再变，不存在方案③"进程生命周期里换好几个顶层窗口"的问题。
    let inplace: windows::Win32::System::Ole::IOleInPlaceObject =
        ole_object.cast().map_err(|e| AppError::Internal(format!("获取 IOleInPlaceObject 失败：{e}")))?;
    let control_hwnd = inplace.GetWindow().map_err(|e| AppError::Internal(format!("获取控件窗口句柄失败：{e}")))?;

    let rdp_client: IMsRdpClient = ole_object.cast().map_err(|e| AppError::Internal(format!("QueryInterface IMsRdpClient 失败：{e}")))?;
    let non_scriptable: IMsTscNonScriptable =
        ole_object.cast().map_err(|e| AppError::Internal(format!("QueryInterface IMsTscNonScriptable 失败：{e}")))?;

    check("put_Server", rdp_client.put_Server(BSTR::from(server)))?;
    check("put_UserName", rdp_client.put_UserName(BSTR::from(username)))?;
    if !domain.is_empty() {
        check("put_Domain", rdp_client.put_Domain(BSTR::from(domain)))?;
    }
    check("put_DesktopWidth", rdp_client.put_DesktopWidth(width as i32))?;
    check("put_DesktopHeight", rdp_client.put_DesktopHeight(height as i32))?;
    // 密码故意不在 `IMsTscAx`/`IMsRdpClient` 这两个"可编写脚本"接口上——得单独
    // `QueryInterface` 到 `IMsTscNonScriptable` 才能设，这是控件自己的安全设计
    // （防止网页脚本直接读写明文密码），不是我们这边多加的一层。
    check("put_ClearTextPassword", non_scriptable.put_ClearTextPassword(BSTR::from(password)))?;
    // 见 `disable_authentication_warning` 上面的注释：没做事件下沉，遇到自签名证书
    // 这类需要用户确认身份的场景会没有接收者去应答，界面会看起来"卡死在黑屏"——
    // 关掉这项校验从根上避开这条路径。这一步失败不应该拖垮整个连接（最坏情况只是
    // 退回默认行为），只记日志不中断。
    if let Err(e) = disable_authentication_warning(&rdp_client) {
        tracing::warn!(%e, "关闭 RDP 身份校验警告失败，继续用默认行为连接（若对方是自签名证书，可能会因为收不到确认应答而卡住）");
    }
    check("Connect", rdp_client.raw_Connect())?;

    Ok((host_hwnd, control_hwnd, rdp_client))
}

/// 把 `AuthenticationLevel` 设成 0（"直接连接，不做身份校验、不警告"）——控件内部
/// 靠这个属性判断要不要在识别不了/信任不了服务器证书时弹出 `OnReceivedTSPublicKey`
/// 确认对话框，默认值是 2（"连接并警告我"）。这台测试服务器（10.203.0.111）已知会
/// 触发这个确认流程；我们没有实现 `IMsTscAxEvents` 事件下沉（见本文件顶部"已知
/// 缺口"），控件没有事件接收者能应答这个请求，很可能就静默卡在这一步——控件本身
/// 没有报错，只是在等一个我们回答不了的问题，界面上表现为"连接成功但一直黑屏"。
///
/// `AuthenticationLevel` 挂在 `IMsRdpClientAdvancedSettings2` 上——这是一个巨大的
/// dual interface，光是它继承链上的基类 `IMsTscAdvancedSettings` 就有几十个属性，
/// 手抄整条 vtable（像 `mstscax.rs` 里对 `IMsTscAx`/`IMsRdpClient`/
/// `IMsTscNonScriptable` 做的那样）风险很高：中间漏一个方法或参数类型搞错，后面
/// 所有方法的 vtable 偏移全部错位，直接是内存越界这种硬故障。改用
/// `IDispatch::GetIDsOfNames`+`Invoke` 这种"晚绑定"方式设置这一个属性——不需要
/// 知道这个接口除了 `IDispatch` 之外的任何 vtable 布局，只要知道属性名字符串，是
/// COM 自动化本来就提供、专门用来避免这类整条接口手抄成本的机制（VBScript/JScript
/// 里 `obj.AuthenticationLevel = 0` 这种写法，底层走的就是这条路）。
unsafe fn disable_authentication_warning(rdp_client: &IMsRdpClient) -> Result<(), AppError> {
    let mut raw: *mut core::ffi::c_void = core::ptr::null_mut();
    check("get_AdvancedSettings2", rdp_client.get_AdvancedSettings2(&mut raw as *mut *mut core::ffi::c_void))?;
    if raw.is_null() {
        return Err(AppError::Internal("get_AdvancedSettings2 返回空指针".into()));
    }
    let dispatch: IDispatch = Interface::from_raw(raw);
    put_property_u32(&dispatch, "AuthenticationLevel", 0)
}

/// 通过 `IDispatch` 晚绑定设置一个 `unsigned int`/VT_UI4 类型的属性——`riid` 参数
/// 在 `GetIDsOfNames`/`Invoke` 里都是保留字段，COM 规范要求恒为 `IID_NULL`
/// （`GUID::zeroed()`），不是随手填的占位值。
unsafe fn put_property_u32(dispatch: &IDispatch, name: &str, value: u32) -> Result<(), AppError> {
    let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let name_pcwstr = PCWSTR(name_wide.as_ptr());
    let mut dispid = 0i32;
    dispatch
        .GetIDsOfNames(&GUID::zeroed(), &name_pcwstr, 1, 0, &mut dispid as *mut i32)
        .map_err(|e| AppError::Internal(format!("GetIDsOfNames({name}) 失败：{e}")))?;

    let mut var = VARIANT::default();
    var.Anonymous.Anonymous = core::mem::ManuallyDrop::new(VARIANT_0_0 {
        vt: VT_UI4,
        wReserved1: 0,
        wReserved2: 0,
        wReserved3: 0,
        Anonymous: VARIANT_0_0_0 { uintVal: value },
    });

    let mut named_arg = DISPID_PROPERTYPUT;
    let params = DISPPARAMS {
        rgvarg: &mut var as *mut VARIANT,
        rgdispidNamedArgs: &mut named_arg as *mut i32,
        cArgs: 1,
        cNamedArgs: 1,
    };

    dispatch
        .Invoke(dispid, &GUID::zeroed(), 0, DISPATCH_PROPERTYPUT, &params as *const DISPPARAMS, None, None, None)
        .map_err(|e| AppError::Internal(format!("Invoke(put {name}) 失败：{e}")))
}

fn check(what: &str, hr: windows::core::HRESULT) -> Result<(), AppError> {
    hr.ok().map_err(|e| AppError::Internal(format!("{what} 失败：{e}")))
}

fn run_message_loop(rdp_client: &IMsRdpClient, probes: &SessionProbes) {
    unsafe {
        let mut msg = MSG::default();
        let mut last_probe = std::time::Instant::now();
        loop {
            let ret = GetMessageW(&mut msg, None, 0, 0).0;
            if ret <= 0 {
                probes.state.store(2, Ordering::Relaxed);
                break; // 0 = 收到 WM_QUIT，-1 = 出错，两种情况都退出循环
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
            if last_probe.elapsed() >= std::time::Duration::from_millis(250) {
                let mut connected = 0i16;
                if rdp_client.get_Connected(&mut connected).is_ok() {
                    probes.connected_raw.store(connected as i32, Ordering::Relaxed);
                    if connected != 0 {
                        probes.state.store(1, Ordering::Relaxed);
                    }
                }
                let mut disconnect_reason = 0i32;
                if rdp_client.get_ExtendedDisconnectReason(&mut disconnect_reason).is_ok() && disconnect_reason != 0 {
                    probes.reason.store(disconnect_reason, Ordering::Relaxed);
                    probes.state.store(3, Ordering::Relaxed);
                }
                last_probe = std::time::Instant::now();
            }
        }
    }
}
