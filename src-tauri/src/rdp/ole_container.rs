//! 承载 MSTSCAX ActiveX 控件所需的最小 OLE 容器实现。
//!
//! ActiveX 控件（这里特指 `mstscax.dll` 的 `MsTscAx`）不是随便 `CoCreateInstance`
//! 出来就能显示——它是"就地激活"（in-place activation）模型：控件需要一个"容器"
//! 通过 `IOleClientSite`/`IOleInPlaceSite`/`IOleInPlaceFrame` 这几个回调接口告诉它
//! "你的宿主窗口是谁""你应该画在哪块矩形区域里"，控件才会真正创建自己的子窗口、
//! 开始接收输入/渲染画面。这几个接口的方法基本都是控件"通知/询问容器"用的，
//! 我们不需要真的实现菜单合并、撤销栈这些完整 OLE 文档容器该有的能力——照 MSDN
//! 对"最小可用容器"的说明，大多数方法直接返回成功/NOTIMPL 即可，真正有实质内容的
//! 只有 `GetWindow`（报告宿主窗口句柄）和 `GetWindowContext`（报告控件应该摆放的
//! 矩形区域，以及把自己交出去当 frame）。
//!
//! 单个 `RdpOleContainer` 同时实现 `IOleClientSite` + `IOleInPlaceSite` +
//! `IOleInPlaceFrame` 三个接口——这是常见做法（不需要分成三个对象），
//! `DoVerb(OLEIVERB_INPLACEACTIVATE, ...)` 时把同一个对象的不同接口指针传给控件。

use std::cell::Cell;

use windows::core::{implement, Error, IUnknownImpl, Ref, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::{E_NOTIMPL, HWND, RECT, SIZE};
use windows::Win32::System::Com::IMoniker;
use windows::Win32::System::Ole::{
    IOleClientSite, IOleClientSite_Impl, IOleContainer, IOleInPlaceActiveObject, IOleInPlaceFrame,
    IOleInPlaceFrame_Impl, IOleInPlaceSite, IOleInPlaceSite_Impl, IOleInPlaceUIWindow,
    IOleInPlaceUIWindow_Impl, IOleWindow_Impl, OLEGETMONIKER, OLEINPLACEFRAMEINFO, OLEMENUGROUPWIDTHS,
    OLEWHICHMK,
};
use windows::Win32::UI::WindowsAndMessaging::{HACCEL, HMENU, MSG};

/// 控件当前应该摆放的屏幕矩形（相对宿主窗口客户区），`rdp/mod.rs` 里 `position_window`
/// 一类调用会更新它——`IOleInPlaceSite::GetWindowContext` 在控件刚激活时会读一次，
/// 后续拖动/缩放不会再回调这个接口，改用直接调用控件的 `IOleInPlaceObject::SetObjectRects`
/// （见 `mod.rs`），这里的 `Cell` 只需要在激活那一刻有个合理初始值。
#[implement(IOleClientSite, IOleInPlaceSite, IOleInPlaceFrame)]
pub struct RdpOleContainer {
    /// roc_desk 主窗口句柄——报告给控件当"父窗口"，控件会把自己的子窗口 `SetParent`
    /// 到这个句柄下面。
    parent_hwnd: HWND,
    rect: Cell<RECT>,
}

impl RdpOleContainer {
    pub fn new(parent_hwnd: HWND, rect: RECT) -> Self {
        Self { parent_hwnd, rect: Cell::new(rect) }
    }
}

impl IOleClientSite_Impl for RdpOleContainer_Impl {
    fn SaveObject(&self) -> Result<()> {
        Ok(())
    }
    fn GetMoniker(&self, _dwassign: &OLEGETMONIKER, _dwwhichmoniker: &OLEWHICHMK) -> Result<IMoniker> {
        Err(Error::from(E_NOTIMPL))
    }
    fn GetContainer(&self) -> Result<IOleContainer> {
        Err(Error::from(E_NOTIMPL))
    }
    fn ShowObject(&self) -> Result<()> {
        Ok(())
    }
    fn OnShowWindow(&self, _fshow: BOOL) -> Result<()> {
        Ok(())
    }
    fn RequestNewObjectLayout(&self) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }
}

// `IOleInPlaceSite`（就地激活的核心协商接口）和 `IOleInPlaceFrame`（frame 级 UI
// 协商接口）都继承自 `IOleWindow`——`GetWindow` 两边共用同一份实现：始终返回宿主
// 主窗口句柄，因为这个对象本身没有自己独立的窗口，它只是替宿主"代言"。
impl IOleWindow_Impl for RdpOleContainer_Impl {
    fn GetWindow(&self) -> Result<HWND> {
        Ok(self.parent_hwnd)
    }
    fn ContextSensitiveHelp(&self, _fentermode: BOOL) -> Result<()> {
        Ok(())
    }
}

impl IOleInPlaceUIWindow_Impl for RdpOleContainer_Impl {
    fn GetBorder(&self) -> Result<RECT> {
        Err(Error::from(E_NOTIMPL))
    }
    fn RequestBorderSpace(&self, _pborderwidths: *const RECT) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }
    fn SetBorderSpace(&self, _pborderwidths: *const RECT) -> Result<()> {
        Ok(())
    }
    fn SetActiveObject(&self, _pactiveobject: Ref<'_, IOleInPlaceActiveObject>, _pszobjname: &PCWSTR) -> Result<()> {
        Ok(())
    }
}

impl IOleInPlaceSite_Impl for RdpOleContainer_Impl {
    fn CanInPlaceActivate(&self) -> Result<()> {
        Ok(())
    }
    fn OnInPlaceActivate(&self) -> Result<()> {
        Ok(())
    }
    fn OnUIActivate(&self) -> Result<()> {
        Ok(())
    }
    fn GetWindowContext(
        &self,
        ppframe: windows::core::OutRef<'_, IOleInPlaceFrame>,
        ppdoc: windows::core::OutRef<'_, IOleInPlaceUIWindow>,
        lprcposrect: *mut RECT,
        lprccliprect: *mut RECT,
        lpframeinfo: *mut OLEINPLACEFRAMEINFO,
    ) -> Result<()> {
        let this: IOleInPlaceFrame = self.to_interface();
        ppframe.write(Some(this))?;
        ppdoc.write(None)?;
        let rect = self.rect.get();
        unsafe {
            if !lprcposrect.is_null() {
                *lprcposrect = rect;
            }
            if !lprccliprect.is_null() {
                *lprccliprect = rect;
            }
            if !lpframeinfo.is_null() {
                *lpframeinfo = OLEINPLACEFRAMEINFO {
                    cb: core::mem::size_of::<OLEINPLACEFRAMEINFO>() as u32,
                    fMDIApp: BOOL(0),
                    hwndFrame: self.parent_hwnd,
                    haccel: HACCEL::default(),
                    cAccelEntries: 0,
                };
            }
        }
        Ok(())
    }
    fn Scroll(&self, _scrollextant: &SIZE) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }
    fn OnUIDeactivate(&self, _fundoable: BOOL) -> Result<()> {
        Ok(())
    }
    fn OnInPlaceDeactivate(&self) -> Result<()> {
        Ok(())
    }
    fn DiscardUndoState(&self) -> Result<()> {
        Ok(())
    }
    fn DeactivateAndUndo(&self) -> Result<()> {
        Ok(())
    }
    fn OnPosRectChange(&self, _lprcposrect: *const RECT) -> Result<()> {
        Ok(())
    }
}

impl IOleInPlaceFrame_Impl for RdpOleContainer_Impl {
    fn InsertMenus(&self, _hmenushared: HMENU, _lpmenuwidths: *mut OLEMENUGROUPWIDTHS) -> Result<()> {
        Ok(())
    }
    fn SetMenu(&self, _hmenushared: HMENU, _holemenu: isize, _hwndactiveobject: HWND) -> Result<()> {
        Ok(())
    }
    fn RemoveMenus(&self, _hmenushared: HMENU) -> Result<()> {
        Ok(())
    }
    fn SetStatusText(&self, _pszstatustext: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn EnableModeless(&self, _fenable: BOOL) -> Result<()> {
        Ok(())
    }
    fn TranslateAccelerator(&self, _lpmsg: *const MSG, _wid: u16) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }
}
