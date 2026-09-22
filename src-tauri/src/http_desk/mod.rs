//! HTTP 测试桌面（docs/HTTP_DESKTOP_PLAN.md）——类似 Apifox/Postman 的接口调试
//! 工作台，复用现有 `workspace` 模块（本地/SSH/Agent 工作区 + `FileOps`），不
//! 自建一套"集合"存储；集合/环境/请求都落在已打开工作区目录的 `.rock_desk/http/`
//! 下（YAML 文件，见 `service.rs`）。
//!
//! **本轮实现范围**（对照方案的 Phase 划分，一次性落地了 Phase 0 + Phase 1 的
//! 核心闭环，Phase 2/3 里最有性价比的一小块，其余明确留作后续任务）：
//!
//! - 已实现：工作区级集合/环境/请求 CRUD（`service.rs`）、变量插值与内置动态值
//!   （`vars.rs`）、请求执行引擎（`client.rs`）、curl/Postman Collection v2.x/
//!   OpenAPI 3.x 导入 + Postman Collection v2.1 导出（`import.rs`/`export.rs`，
//!   2026-09-22 补齐后两种）、请求历史（`history.rs`）、工作区标签页元数据
//!   （`db/repo/http_workspace_tabs_repo.rs`）。
//! - **未实现，明确留作后续任务**：
//!   - 前置/后置脚本引擎（方案 §4.3 的 QuickJS/`rquickjs` 沙箱）——`RequestDef`
//!     目前没有 script 字段，Auth/Body 之外的"脚本改写请求/断言响应"能力本轮
//!     完全没有落地。这是范围缩减最大的一块，因为要做对（沙箱边界、`pm.*` API、
//!     执行超时）本身工作量接近本轮其余部分之和。
//!   - HAR 导入、OpenCollection YAML 导出——HAR 只记录"已发生的请求/响应"，不是
//!     给别的工具当集合导入用的格式，暂不是优先级。
//!   - AI 面板（方案 §8）——没有接入 `ChangeStore`/AI provider，请求文件的改动
//!     目前只能通过 UI 手工编辑。
//!   - Mock 服务（方案 §9，本来就标注 Phase 3+）。
//!   - Collection Runner 批量运行文件夹、响应体真正的流式大小上限（目前是
//!     "读完再截断"，见 `client.rs` 顶部注释）。
//!
//! 换句话说：这是一个能真实跑通"打开工作区 → 建集合/请求/环境 → 填 URL/方法/
//! Headers/Body/Auth → 发送 → 看响应 → 历史记录"这条主链路的可用版本，但还不是
//! 完整覆盖方案里所有 Phase 的最终形态。

pub mod client;
pub mod export;
pub mod import;
pub mod model;
pub mod service;
pub mod vars;
