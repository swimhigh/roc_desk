use serde::Deserialize;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Webview, WebviewBuilder, WebviewUrl};

use crate::error::AppError;

/// 网页浏览模块（DESIGN.md §3.5）。
///
/// **2026-08-18 需求变更**（用户原话："浏览器输入地址后不应该是弹框，而是默认在TAB下
/// 打开，和编辑器一样"）：第一版用独立 `WebviewWindow` 弹窗承载网页，用户明确反馈不要
/// 弹窗，要内嵌在应用主窗口的面板区域里，和编辑器视图切换体验一致。改用 Tauri 2.0 的
/// "子 WebView"能力（`Window::add_child`，需要 `unstable` cargo feature）：在主窗口内
/// 按前端传来的矩形区域（`PanelBounds`）叠加一个独立的子 WebView 实例。
///
/// **为什么不用 `<iframe>`**：`<iframe>` 会和宿主页面共享同一个 WebView 进程/JS 上下文
/// （虽然跨域脚本访问受 same-origin policy 限制，但仍在同一个渲染进程里，颗粒度不够干净），
/// 而且大量真实网站用 `X-Frame-Options`/CSP `frame-ancestors` 拒绝被嵌入，`<iframe>` 加载
/// 会直接空白失败。子 WebView 是操作系统级别的独立 WebView2 实例，天然不受这些"防嵌入"
/// 头影响（它不是被嵌入到另一个 HTML 页面里，是被摆放到宿主窗口的一块区域）。
///
/// **安全隔离维持不变**：子 WebView 的 label 是 `browser-panel`，不在
/// `capabilities/default.json` 的 `windows: ["main"]` 列表里，天生没有任何 Tauri command
/// 调用权限，这个特性不因为"内嵌"还是"独立窗口"而改变——两者都是 label 隔离，只是
/// 前者的位置/大小由宿主窗口摆布，后者有自己的标题栏和操作系统级窗口边框。
///
/// **原生子 WebView 的已知局限**（诚实记录）：它是操作系统合成的独立视图，不参与 HTML
/// 的 CSS 层叠上下文——不管前端这边的 `z-index`/`display: none` 怎么设置，只要没有显式
/// 调用 `hide()`，它就会盖在窗口里同一块屏幕坐标上的任何东西上面（包括 toast、弹窗）。
/// 所以切走"网页浏览"这个 Tab 时，前端必须显式调 `browser_hide`，不能只是用 CSS 隐藏
/// 容器——这是子 WebView 方案相对 `<iframe>` 付出的代价。
///
/// 只在用户从地址栏主动输入 URL 这个入口做了协议白名单校验（只放行 http/https），没有
/// 拦截"页面内点击链接跳转到 file://"这类更深层的导航——Tauri 2.0 是否提供了逐次导航
/// 拦截的 hook 需要进一步调研，这版先不做，后续如果发现是真实风险再补。
pub const PANEL_LABEL: &str = "browser-panel";

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PanelBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// 把用户在地址栏输入的字符串规整成一个可导航的 URL：已经带协议的直接透传（但只
/// 放行 http/https，拦截 file:// 等其它协议）；看起来像域名（含点、不含空格）的
/// 自动补全 https:// 前缀；其余一律当成搜索词，交给搜索引擎处理——和主流浏览器
/// 地址栏的行为习惯一致，用户不需要先分清"这是网址还是搜索词"。
pub fn normalize_url(input: &str) -> Result<String, AppError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AppError::Internal("网址不能为空".into()));
    }

    if let Some(rest) = trimmed.split_once("://").map(|(scheme, _)| scheme) {
        let scheme = rest.to_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(AppError::PermissionDenied(format!(
                "不支持的协议：{scheme}（只允许 http/https，这是安全边界，不能绕过）"
            )));
        }
        return Ok(trimmed.to_string());
    }

    // 含点号且不含空格，认为是网址（可能带路径，如 github.com/user/repo），否则当搜索词。
    let looks_like_url = trimmed.contains('.') && !trimmed.contains(' ');
    if looks_like_url {
        return Ok(format!("https://{trimmed}"));
    }

    let query = urlencoding_lite(trimmed);
    Ok(format!("https://www.bing.com/search?q={query}"))
}

/// 不引入额外的 urlencoding crate——只是给搜索词做基本的 query string 转义，
/// 覆盖最常见的空格/中文/符号场景就够用，不需要严格符合 RFC 3986 的完整实现。
fn urlencoding_lite(s: &str) -> String {
    let mut out = String::new();
    for byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn apply_bounds(webview: &Webview, bounds: PanelBounds) -> Result<(), AppError> {
    webview
        .set_position(LogicalPosition::new(bounds.x, bounds.y))
        .map_err(|e| AppError::Internal(format!("设置浏览器位置失败：{e}")))?;
    webview
        .set_size(LogicalSize::new(bounds.width, bounds.height))
        .map_err(|e| AppError::Internal(format!("设置浏览器大小失败：{e}")))?;
    Ok(())
}

/// 打开（首次创建）或导航（复用已有的）内嵌浏览器子 WebView，定位到 `bounds` 描述的
/// 矩形区域，并确保可见。`bounds` 用的是逻辑像素（CSS px），和前端 `getBoundingClientRect()`
/// 拿到的值直接对应——Tauri 的 `Logical*` 类型内部按窗口缩放因子换算成物理像素，
/// 不需要在前端手动处理 DPI 缩放。
pub fn open_or_navigate(app_handle: &AppHandle, url: &str, bounds: PanelBounds) -> Result<(), AppError> {
    let parsed: tauri::Url = url.parse().map_err(|e| AppError::Internal(format!("无效的 URL：{e}")))?;

    if let Some(webview) = app_handle.get_webview(PANEL_LABEL) {
        webview
            .navigate(parsed)
            .map_err(|e| AppError::Internal(format!("导航失败：{e}")))?;
        apply_bounds(&webview, bounds)?;
        webview
            .show()
            .map_err(|e| AppError::Internal(format!("显示浏览器失败：{e}")))?;
        return Ok(());
    }

    let window = app_handle
        .get_window("main")
        .ok_or_else(|| AppError::Internal("找不到主窗口".into()))?;
    let builder = WebviewBuilder::new(PANEL_LABEL, WebviewUrl::External(parsed));
    window
        .add_child(
            builder,
            LogicalPosition::new(bounds.x, bounds.y),
            LogicalSize::new(bounds.width, bounds.height),
        )
        .map_err(|e| AppError::Internal(format!("创建内嵌浏览器失败：{e}")))?;
    Ok(())
}

/// 只调整位置/大小（窗口缩放、侧边栏拖拽调宽等场景），不存在时静默跳过——调用方
/// 不需要先判断"当前是否已经打开过网页"。
pub fn set_bounds(app_handle: &AppHandle, bounds: PanelBounds) -> Result<(), AppError> {
    if let Some(webview) = app_handle.get_webview(PANEL_LABEL) {
        apply_bounds(&webview, bounds)?;
    }
    Ok(())
}

/// 切走"网页浏览"Tab 时调用——子 WebView 不受 CSS 影响，必须显式隐藏，否则会一直
/// 盖在窗口的固定屏幕区域上，不管前端切到了编辑器还是别的什么视图。
pub fn hide(app_handle: &AppHandle) -> Result<(), AppError> {
    if let Some(webview) = app_handle.get_webview(PANEL_LABEL) {
        webview.hide().map_err(|e| AppError::Internal(format!("隐藏浏览器失败：{e}")))?;
    }
    Ok(())
}

/// 切回"网页浏览"Tab 且此前已经打开过网页时调用，重新定位（面板尺寸可能在隐藏期间
/// 变了）并显示。
pub fn show(app_handle: &AppHandle, bounds: PanelBounds) -> Result<(), AppError> {
    if let Some(webview) = app_handle.get_webview(PANEL_LABEL) {
        apply_bounds(&webview, bounds)?;
        webview.show().map_err(|e| AppError::Internal(format!("显示浏览器失败：{e}")))?;
    }
    Ok(())
}

/// 彻底销毁子 WebView（工作区关闭、退回工作区选择页时调用），避免留下一个宿主窗口
/// 已经不存在对应 UI 上下文的孤儿子 WebView。
pub fn close(app_handle: &AppHandle) -> Result<(), AppError> {
    if let Some(webview) = app_handle.get_webview(PANEL_LABEL) {
        webview.close().map_err(|e| AppError::Internal(format!("关闭浏览器失败：{e}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_https_url() {
        assert_eq!(normalize_url("https://github.com").unwrap(), "https://github.com");
    }

    #[test]
    fn rejects_file_protocol() {
        assert!(normalize_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn adds_https_to_bare_domain() {
        assert_eq!(normalize_url("github.com").unwrap(), "https://github.com");
    }

    #[test]
    fn treats_plain_text_as_search_query() {
        let result = normalize_url("how to use rust").unwrap();
        assert!(result.starts_with("https://www.bing.com/search?q="));
    }
}
