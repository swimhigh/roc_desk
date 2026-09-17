use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex as StdMutex;

use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::error::AppError;
use crate::sql::adapter::{new_backend_handle_slot, BackendHandleSlot};
use crate::sql::model::{ExecuteResult, QueryPoll, QueryStatus};

/// 一条正在跑（或刚跑完、还没被前端 poll 走）的查询——所有 adapter 共用同一套
/// spawn/poll/cancel 机制，不需要每个驱动各自实现一遍任务状态机（对比
/// rainfrog 每种驱动各自写一份 `PostgresTask`/`Query`/`TxConnect` 枚举，这里
/// 把"怎么跑、怎么等、怎么取消"收敛成一份通用逻辑，adapter 只需要提供一个
/// `execute_sql` 异步函数）。
struct RunningQuery {
    handle: JoinHandle<Result<ExecuteResult, AppError>>,
    backend_handle: BackendHandleSlot,
}

#[derive(Default)]
pub struct QueryExecutor {
    running: StdMutex<HashMap<Uuid, RunningQuery>>,
}

impl QueryExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    /// 启动一条查询（对应 Tauri command `sql_execute`）：`tokio::spawn` 出去
    /// 立刻返回 `query_id`，不阻塞调用方。`make_fut` 拿到的
    /// `BackendHandleSlot` 要原样传给 `AdapterSession::execute_sql`，这样
    /// `execute_sql` 内部才能把"服务端怎么杀我"的信息写回这个槽位；用回调
    /// 而不是直接传 future 进来，是因为槽位要先造出来才能构造 `fut`，但
    /// `query_id` 要等 spawn 之后才存在——`abort` 只需要 `query_id` 就能
    /// 找到槽位，不需要调用方自己额外保存它。
    pub fn spawn_with_backend_handle<F>(&self, make_fut: impl FnOnce(BackendHandleSlot) -> F) -> Uuid
    where
        F: Future<Output = Result<ExecuteResult, AppError>> + Send + 'static,
    {
        let query_id = Uuid::new_v4();
        let backend_handle = new_backend_handle_slot();
        let fut = make_fut(backend_handle.clone());
        let handle = tokio::spawn(fut);
        self.running.lock().unwrap().insert(
            query_id,
            RunningQuery {
                handle,
                backend_handle,
            },
        );
        query_id
    }

    /// 非阻塞地看一眼这条查询跑完了没有；跑完了就把结果转成 `QueryPoll`
    /// 并从内部表里移除（这个 query_id 用完了，不需要再保留）。
    pub async fn poll(&self, query_id: Uuid) -> Result<QueryPoll, AppError> {
        let finished = {
            let running = self.running.lock().unwrap();
            match running.get(&query_id) {
                Some(rq) => rq.handle.is_finished(),
                None => {
                    return Err(AppError::NotFound(format!("query not found: {query_id}")));
                }
            }
        };

        if !finished {
            return Ok(QueryPoll {
                status: QueryStatus::Running,
                result: None,
                error: None,
            });
        }

        let rq = self.running.lock().unwrap().remove(&query_id);
        let Some(rq) = rq else {
            return Err(AppError::NotFound(format!("query not found: {query_id}")));
        };

        match rq.handle.await {
            Ok(Ok(result)) => Ok(QueryPoll {
                status: QueryStatus::Finished,
                result: Some(result),
                error: None,
            }),
            Ok(Err(e)) => Ok(QueryPoll {
                status: QueryStatus::Error,
                result: None,
                error: Some(e.to_string()),
            }),
            Err(join_err) if join_err.is_cancelled() => Ok(QueryPoll {
                status: QueryStatus::Cancelled,
                result: None,
                error: None,
            }),
            Err(join_err) => Ok(QueryPoll {
                status: QueryStatus::Error,
                result: None,
                error: Some(join_err.to_string()),
            }),
        }
    }

    /// 取消：先中断本地 tokio 任务（防止它继续占着 poll 逻辑），再把
    /// `backend_handle` 交还给调用方——调用方（`commands::sql::sql_cancel`）
    /// 还要拿这个 handle 去调 `AdapterSession::kill_backend` 真正杀掉服务端
    /// 正在执行的语句，`QueryExecutor` 本身不知道怎么杀，那是 adapter 的事。
    pub fn abort(&self, query_id: Uuid) -> Option<String> {
        let rq = self.running.lock().unwrap().remove(&query_id);
        rq.map(|rq| {
            rq.handle.abort();
            rq.backend_handle.lock().unwrap().clone()
        })
        .flatten()
    }
}
