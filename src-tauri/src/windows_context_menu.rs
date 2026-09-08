#[cfg(target_os = "windows")]
#[tauri::command]
pub fn show_windows_context_menu(window: tauri::Window, path: String) -> Result<(), String> {
    // tauri 的 command 处理函数跑在异步任务线程池上，不是拥有主窗口 hwnd 的那个
    // 线程（那个线程专门跑 tao/winit 的消息循环）。TrackPopupMenu 弹出的菜单窗口
    // 属于*调用它的线程*，如果这个线程不是当前的前台/输入线程，弹出的菜单拿不到
    // 鼠标点击，表现就是右键完全没反应——这正是之前这个功能一直呼不出来的原因。
    // 用 run_on_main_thread 切回主线程执行，结果通过 channel 传回来。
    // FileEntry.path 在这个 app 里全程用 `/` 分隔（见 fsops/local.rs 的
    // `full_path.to_string_lossy().replace('\\', "/")`），但 Shell 命名空间解析
    // （SHParseDisplayName）不认 `/`，会直接返回 E_INVALIDARG——之前"参数错误
    // (0x80070057)"就是这么来的。这里转回原生反斜杠再喂给 Shell API。
    let win_path = path.replace('/', "\\");
    let window_main = window.clone();
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    window
        .run_on_main_thread(move || {
            let result = unsafe { show_context_menu_impl(&window_main, &win_path) };
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

#[cfg(target_os = "windows")]
unsafe fn show_context_menu_impl(window: &tauri::Window, path: &str) -> Result<(), String> {
    use std::ptr::null_mut;
    use windows::core::{PCSTR, PCWSTR};
    use windows::Win32::Foundation::{HANDLE, LPARAM, POINT, WPARAM};
    use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        IContextMenu, IShellFolder, CMINVOKECOMMANDINFO, CMF_NORMAL, SHBindToParent,
        SHParseDisplayName,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreatePopupMenu, DestroyMenu, GetCursorPos, PostMessageW, SetForegroundWindow,
        TrackPopupMenu, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_NULL,
    };

    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let mut pidl = null_mut();
    SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None)
        .map_err(|e| e.to_string())?;

    let mut child = null_mut();
    let parent: IShellFolder =
        SHBindToParent(pidl, Some(&mut child)).map_err(|e| e.to_string())?;
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let context: IContextMenu = parent
        .GetUIObjectOf(hwnd, &[child], None)
        .map_err(|e| e.to_string())?;
    let popup = CreatePopupMenu().map_err(|e| e.to_string())?;
    context
        .QueryContextMenu(popup, 0, 1, 0x7fff, CMF_NORMAL)
        .ok()
        .map_err(|e| e.to_string())?;

    let mut point = POINT::default();
    GetCursorPos(&mut point).map_err(|e| e.to_string())?;
    // 参考微软对托盘图标弹出菜单的官方建议：TrackPopupMenu 前先把窗口设为前台
    // 窗口，结束后再 post 一个空消息——否则菜单可能收不到点击，或者点击菜单外
    // 时无法正常关闭。
    let _ = SetForegroundWindow(hwnd);
    let command = TrackPopupMenu(
        popup,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        point.x,
        point.y,
        None,
        hwnd,
        None,
    )
    .0 as u32;
    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    if command > 0 {
        let info = CMINVOKECOMMANDINFO {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd,
            lpVerb: PCSTR((command - 1) as usize as *const u8),
            nShow: SW_SHOWNORMAL.0,
            hIcon: HANDLE::default(),
            ..Default::default()
        };
        context.InvokeCommand(&info).map_err(|e| e.to_string())?;
    }
    let _ = DestroyMenu(popup);
    CoTaskMemFree(Some(pidl.cast()));
    Ok(())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub fn show_windows_context_menu(_window: tauri::Window, _path: String) -> Result<(), String> {
    Err("Windows context menu is only available on Windows".into())
}
