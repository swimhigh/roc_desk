//! 手写的 MSTSCAX（`mstscax.dll`，Windows 自带 RDP ActiveX 控件）COM 接口定义。
//!
//! `windows` crate 只覆盖 Win32 核心 API 的类型库元数据，不含 MSTSCAX 这种第三方/
//! 系统自带控件各自的类型库——这几个接口是照着 MSVC `#import "mstscax.dll"` 生成的
//! 权威 `.tlh`/`.h`（比对了两份独立来源：nogginware/mstscdump 和 Devolutions/MsRdpEx，
//! 两者一致）逐个方法手抄的，vtable 槽位顺序、参数类型都以那份生成文件为准，不是
//! 凭文档页面（MS Learn 的方法列表是按字母排序的，不是真实 vtable 顺序，不能直接抄）
//! 拼出来的。只抄了会用到的三个接口：
//!   - `IMsTscAx`（IUnknown+IDispatch 之后 30 个槽位）：Server/Domain/UserName/
//!     DesktopWidth/DesktopHeight/Connected/Connect/Disconnect 等基础属性方法。
//!   - `IMsRdpClient`（继承 IMsTscAx，再加 9 个槽位）：ColorDepth/
//!     ExtendedDisconnectReason/FullScreen 等。
//!   - `IMsTscNonScriptable`（纯 IUnknown 接口，不是 dual interface，只有 10 个
//!     槽位）：`put_ClearTextPassword`——密码字段特意不在可编写脚本的接口上，
//!     这是安全设计，得单独 QueryInterface 这个"不安全脚本访问"接口才能设。
//!
//! CLSID 用的是 `MsTscAx`（三个初始 coclass 里最早出现的一个，同样暴露
//! `IMsRdpClient`/`IMsTscNonScriptable`，兼容性覆盖面最广）。

#![allow(non_snake_case, non_camel_case_types)]

// `#[interface(...)]` 宏展开出来的代码里会直接写死 `<父接口名>_Vtbl`/`_Impl` 这种
// 裸标识符（不是完整路径），指望它们能在这个模块的作用域里直接被解析到——所以哪怕
// 我们自己从来不会手写 `IUnknown_Vtbl`/`IDispatch_Vtbl` 这些类型，也得显式 `use`
// 进来，宏才展开得通过。
use windows::core::{interface, IUnknown, IUnknown_Vtbl, BSTR, GUID, HRESULT};
use windows::Win32::Foundation::VARIANT_BOOL;
use windows::Win32::System::Com::{IDispatch, IDispatch_Impl, IDispatch_Vtbl};

/// `CLSID_MsTscAx`。
pub const CLSID_MSTSCAX: GUID = GUID::from_u128(0x1fb464c8_09bb_4017_a2f5_eb742f04392f);

#[interface("8c11efae-92c3-11d1-bc1e-00c04fa31489")]
pub unsafe trait IMsTscAx: IDispatch {
    pub fn put_Server(&self, val: BSTR) -> HRESULT;
    pub fn get_Server(&self, out: *mut BSTR) -> HRESULT;
    pub fn put_Domain(&self, val: BSTR) -> HRESULT;
    pub fn get_Domain(&self, out: *mut BSTR) -> HRESULT;
    pub fn put_UserName(&self, val: BSTR) -> HRESULT;
    pub fn get_UserName(&self, out: *mut BSTR) -> HRESULT;
    pub fn put_DisconnectedText(&self, val: BSTR) -> HRESULT;
    pub fn get_DisconnectedText(&self, out: *mut BSTR) -> HRESULT;
    pub fn put_ConnectingText(&self, val: BSTR) -> HRESULT;
    pub fn get_ConnectingText(&self, out: *mut BSTR) -> HRESULT;
    pub fn get_Connected(&self, out: *mut i16) -> HRESULT;
    pub fn put_DesktopWidth(&self, val: i32) -> HRESULT;
    pub fn get_DesktopWidth(&self, out: *mut i32) -> HRESULT;
    pub fn put_DesktopHeight(&self, val: i32) -> HRESULT;
    pub fn get_DesktopHeight(&self, out: *mut i32) -> HRESULT;
    pub fn put_StartConnected(&self, val: i32) -> HRESULT;
    pub fn get_StartConnected(&self, out: *mut i32) -> HRESULT;
    pub fn get_HorizontalScrollBarVisible(&self, out: *mut i32) -> HRESULT;
    pub fn get_VerticalScrollBarVisible(&self, out: *mut i32) -> HRESULT;
    pub fn put_FullScreenTitle(&self, val: BSTR) -> HRESULT;
    pub fn get_CipherStrength(&self, out: *mut i32) -> HRESULT;
    pub fn get_Version(&self, out: *mut BSTR) -> HRESULT;
    pub fn get_SecuredSettingsEnabled(&self, out: *mut i32) -> HRESULT;
    pub fn get_SecuredSettings(&self, out: *mut *mut core::ffi::c_void) -> HRESULT;
    pub fn get_AdvancedSettings(&self, out: *mut *mut core::ffi::c_void) -> HRESULT;
    pub fn get_Debugger(&self, out: *mut *mut core::ffi::c_void) -> HRESULT;
    pub fn raw_Connect(&self) -> HRESULT;
    pub fn raw_Disconnect(&self) -> HRESULT;
    pub fn raw_CreateVirtualChannels(&self, val: BSTR) -> HRESULT;
    pub fn raw_SendOnVirtualChannel(&self, chan_name: BSTR, chan_data: BSTR) -> HRESULT;
}

#[interface("92b4a539-7115-4b7c-a5a9-e5d9efc2780a")]
pub unsafe trait IMsRdpClient: IMsTscAx {
    pub fn put_ColorDepth(&self, val: i32) -> HRESULT;
    pub fn get_ColorDepth(&self, out: *mut i32) -> HRESULT;
    pub fn get_AdvancedSettings2(&self, out: *mut *mut core::ffi::c_void) -> HRESULT;
    pub fn get_SecuredSettings2(&self, out: *mut *mut core::ffi::c_void) -> HRESULT;
    pub fn get_ExtendedDisconnectReason(&self, out: *mut i32) -> HRESULT;
    pub fn put_FullScreen(&self, val: VARIANT_BOOL) -> HRESULT;
    pub fn get_FullScreen(&self, out: *mut VARIANT_BOOL) -> HRESULT;
    pub fn raw_SetVirtualChannelOptions(&self, chan_name: BSTR, chan_options: i32) -> HRESULT;
    pub fn raw_GetVirtualChannelOptions(&self, chan_name: BSTR, out: *mut i32) -> HRESULT;
    pub fn raw_RequestClose(&self, out: *mut i32) -> HRESULT;
}

#[interface("c1e6743a-41c1-4a74-832a-0dd06c1c7a0e")]
pub unsafe trait IMsTscNonScriptable: IUnknown {
    pub fn put_ClearTextPassword(&self, val: BSTR) -> HRESULT;
    pub fn put_PortablePassword(&self, val: BSTR) -> HRESULT;
    pub fn get_PortablePassword(&self, out: *mut BSTR) -> HRESULT;
    pub fn put_PortableSalt(&self, val: BSTR) -> HRESULT;
    pub fn get_PortableSalt(&self, out: *mut BSTR) -> HRESULT;
    pub fn put_BinaryPassword(&self, val: BSTR) -> HRESULT;
    pub fn get_BinaryPassword(&self, out: *mut BSTR) -> HRESULT;
    pub fn put_BinarySalt(&self, val: BSTR) -> HRESULT;
    pub fn get_BinarySalt(&self, out: *mut BSTR) -> HRESULT;
    pub fn raw_ResetPassword(&self) -> HRESULT;
}
