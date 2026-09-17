use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::sql::adapter::{new_backend_handle_slot, AdapterSession};
use crate::sql::data_editor::{build_page_query, quote_ident, quote_table};
use crate::sql::model::{Cell, DbKind, ObjectRef};

/// 导出/导入的批大小——太小则往返次数多、太大则单批内存占用高，2000 行是
/// 桌面客户端场景下两头都能接受的折中值，不暴露成用户可调参数（不值得为
/// 这么一个内部实现细节增加一个配置项）。
const BATCH_SIZE: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProgressSidecar {
    offset: u64,
    rows_done: u64,
}

fn sidecar_path(file_path: &str) -> PathBuf {
    PathBuf::from(format!("{file_path}.progress.json"))
}

fn read_sidecar(file_path: &str) -> Option<ProgressSidecar> {
    let text = std::fs::read_to_string(sidecar_path(file_path)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_sidecar(file_path: &str, progress: &ProgressSidecar) {
    if let Ok(text) = serde_json::to_string(progress) {
        let _ = std::fs::write(sidecar_path(file_path), text);
    }
}

fn clear_sidecar(file_path: &str) {
    let _ = std::fs::remove_file(sidecar_path(file_path));
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn cell_to_csv_field(cell: &Cell) -> String {
    if cell.is_null {
        String::new()
    } else {
        csv_escape(&cell.text)
    }
}

/// 手写的最小 CSV 行解析——只处理"双引号包裹 + 内部双引号转义"这一种真实
/// 场景会遇到的复杂情况，不追求覆盖 CSV 方言的所有历史包袱（比如不同的
/// 换行约定、非逗号分隔符）；这一版的导入只吃自己导出的或者标准逗号分隔
/// CSV，不是通用 CSV 解析库的替代品。
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                current.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    fields.push(current);
    fields
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferFormat {
    Csv,
    Json,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransferProgress {
    pub rows_done: u64,
    pub done: bool,
    pub cancelled: bool,
    pub error: Option<String>,
}

struct TransferHandle {
    progress: Arc<StdMutex<TransferProgress>>,
    cancel: Arc<AtomicBool>,
}

/// 导出/导入任务的 spawn/poll/cancel registry——和 `executor::QueryExecutor`
/// 同一种模式（方案 §4.2.1），但导出导入是"可能跑几分钟、要报进度、要能续传"
/// 的长任务，用独立的状态结构体而不是复用 `QueryExecutor`。
#[derive(Default)]
pub struct TransferManager {
    jobs: StdMutex<HashMap<Uuid, TransferHandle>>,
}

impl TransferManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn poll(&self, id: Uuid) -> Result<TransferProgress, AppError> {
        let jobs = self.jobs.lock().unwrap();
        let handle = jobs
            .get(&id)
            .ok_or_else(|| AppError::NotFound(format!("transfer job not found: {id}")))?;
        let progress = handle.progress.lock().unwrap().clone();
        Ok(progress)
    }

    pub fn cancel(&self, id: Uuid) {
        if let Some(handle) = self.jobs.lock().unwrap().get(&id) {
            handle.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// 导出——按 `BATCH_SIZE` 分批查询、边查边追加写盘，不会把整份结果先攒
    /// 在内存里再一次性写出（方案 §4.3"大结果集占用内存"的同一个考量，这里
    /// 影响更大：导出面向的正是"结果集很大"这个场景）。每写完一批就把进度
    /// 落一份 sidecar json，中断（用户取消/进程崩溃）后按这份记录从
    /// `OFFSET` 继续，不用从头来过——注意这只在源表在两次导出之间没有大量
    /// 增删的前提下可靠：`OFFSET` 分页本身不是自洽于并发写入的稳定游标，
    /// 这是简化实现的已知代价，不是缺陷疏漏。
    pub async fn start_export(
        &self,
        session: Arc<dyn AdapterSession>,
        kind: DbKind,
        object: ObjectRef,
        order_col: Option<String>,
        format: TransferFormat,
        file_path: String,
        resume: bool,
    ) -> Result<Uuid, AppError> {
        let id = Uuid::new_v4();
        let start_offset = if resume {
            read_sidecar(&file_path).map(|s| s.offset).unwrap_or(0)
        } else {
            clear_sidecar(&file_path);
            0
        };
        let progress = Arc::new(StdMutex::new(TransferProgress {
            rows_done: start_offset,
            done: false,
            cancelled: false,
            error: None,
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.lock().unwrap().insert(
            id,
            TransferHandle {
                progress: progress.clone(),
                cancel: cancel.clone(),
            },
        );

        tokio::spawn(async move {
            let result = run_export(
                session,
                kind,
                object,
                order_col,
                format,
                file_path.clone(),
                start_offset,
                progress.clone(),
                cancel,
            )
            .await;
            if let Err(e) = result {
                progress.lock().unwrap().error = Some(e.to_string());
            }
        });

        Ok(id)
    }

    pub async fn start_import(
        &self,
        session: Arc<dyn AdapterSession>,
        kind: DbKind,
        object: ObjectRef,
        columns: Vec<String>,
        file_path: String,
        skip_header: bool,
        resume: bool,
    ) -> Result<Uuid, AppError> {
        let id = Uuid::new_v4();
        let start_line = if resume {
            read_sidecar(&file_path).map(|s| s.rows_done).unwrap_or(0)
        } else {
            clear_sidecar(&file_path);
            0
        };
        let progress = Arc::new(StdMutex::new(TransferProgress {
            rows_done: start_line,
            done: false,
            cancelled: false,
            error: None,
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.lock().unwrap().insert(
            id,
            TransferHandle {
                progress: progress.clone(),
                cancel: cancel.clone(),
            },
        );

        tokio::spawn(async move {
            let result = run_import(
                session,
                kind,
                object,
                columns,
                file_path,
                skip_header,
                start_line,
                progress.clone(),
                cancel,
            )
            .await;
            if let Err(e) = result {
                progress.lock().unwrap().error = Some(e.to_string());
            }
        });

        Ok(id)
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_export(
    session: Arc<dyn AdapterSession>,
    kind: DbKind,
    object: ObjectRef,
    order_col: Option<String>,
    format: TransferFormat,
    file_path: String,
    start_offset: u64,
    progress: Arc<StdMutex<TransferProgress>>,
    cancel: Arc<AtomicBool>,
) -> Result<(), AppError> {
    ensure_parent_dir(&file_path)?;
    let append = start_offset > 0;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&file_path)?;

    let mut offset = start_offset as usize;
    let mut wrote_header = append;
    let mut wrote_any_json_row = append; // 续传 JSON 时不再重写开头的 `[`

    if format == TransferFormat::Json && !append {
        file.write_all(b"[\n")?;
    }

    loop {
        if cancel.load(Ordering::Relaxed) {
            let mut p = progress.lock().unwrap();
            p.cancelled = true;
            write_sidecar(
                &file_path,
                &ProgressSidecar {
                    offset: offset as u64,
                    rows_done: offset as u64,
                },
            );
            return Ok(());
        }

        let sql = build_page_query(kind, &object, order_col.as_deref(), BATCH_SIZE, offset);
        let slot = new_backend_handle_slot();
        let result = session.execute_sql(&sql, BATCH_SIZE, slot).await?;
        if result.rows.is_empty() {
            break;
        }

        match format {
            TransferFormat::Csv => {
                if !wrote_header {
                    let header = result
                        .columns
                        .iter()
                        .map(|c| csv_escape(&c.name))
                        .collect::<Vec<_>>()
                        .join(",");
                    writeln!(file, "{header}")?;
                    wrote_header = true;
                }
                for row in &result.rows {
                    let line = row.iter().map(cell_to_csv_field).collect::<Vec<_>>().join(",");
                    writeln!(file, "{line}")?;
                }
            }
            TransferFormat::Json => {
                for row in &result.rows {
                    if wrote_any_json_row {
                        file.write_all(b",\n")?;
                    }
                    let obj: serde_json::Map<String, serde_json::Value> = result
                        .columns
                        .iter()
                        .zip(row.iter())
                        .map(|(col, cell)| {
                            let value = if cell.is_null {
                                serde_json::Value::Null
                            } else {
                                serde_json::Value::String(cell.text.clone())
                            };
                            (col.name.clone(), value)
                        })
                        .collect();
                    file.write_all(serde_json::to_string(&obj).unwrap_or_default().as_bytes())?;
                    wrote_any_json_row = true;
                }
            }
        }
        file.flush()?;

        offset += result.rows.len();
        {
            let mut p = progress.lock().unwrap();
            p.rows_done = offset as u64;
        }
        write_sidecar(
            &file_path,
            &ProgressSidecar {
                offset: offset as u64,
                rows_done: offset as u64,
            },
        );

        if result.rows.len() < BATCH_SIZE {
            break;
        }
    }

    if format == TransferFormat::Json {
        file.write_all(b"\n]\n")?;
    }

    clear_sidecar(&file_path);
    progress.lock().unwrap().done = true;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_import(
    session: Arc<dyn AdapterSession>,
    kind: DbKind,
    object: ObjectRef,
    columns: Vec<String>,
    file_path: String,
    skip_header: bool,
    start_line: u64,
    progress: Arc<StdMutex<TransferProgress>>,
    cancel: Arc<AtomicBool>,
) -> Result<(), AppError> {
    let file = std::fs::File::open(&file_path)?;
    let reader = BufReader::new(file);
    let table = quote_table(kind, &object);
    let quoted_cols = columns
        .iter()
        .map(|c| quote_ident(kind, c))
        .collect::<Vec<_>>()
        .join(", ");

    let mut lines = reader.lines();
    if skip_header {
        lines.next();
    }
    // 续传：跳过已经处理过的行数（按行计数，不去重——见 `TransferManager`
    // 文档注释，导入前提示用户先备份就是为了兜住这种"可能重复插入"的风险）。
    for _ in 0..start_line {
        if lines.next().is_none() {
            break;
        }
    }

    let mut batch: Vec<String> = Vec::with_capacity(BATCH_SIZE);
    let mut rows_done = start_line;

    let flush_batch = |batch: &mut Vec<String>| -> Option<String> {
        if batch.is_empty() {
            return None;
        }
        let values = batch.join(", ");
        batch.clear();
        Some(format!("INSERT INTO {table} ({quoted_cols}) VALUES {values}"))
    };

    for line in lines {
        if cancel.load(Ordering::Relaxed) {
            let mut p = progress.lock().unwrap();
            p.cancelled = true;
            write_sidecar(
                &file_path,
                &ProgressSidecar {
                    offset: rows_done,
                    rows_done,
                },
            );
            return Ok(());
        }
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let fields = parse_csv_line(&line);
        let literals = fields
            .iter()
            .map(|f| format!("'{}'", f.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");
        batch.push(format!("({literals})"));
        rows_done += 1;

        if batch.len() >= BATCH_SIZE {
            if let Some(sql) = flush_batch(&mut batch) {
                let slot = new_backend_handle_slot();
                session.execute_sql(&sql, 0, slot).await?;
            }
            let mut p = progress.lock().unwrap();
            p.rows_done = rows_done;
            drop(p);
            write_sidecar(
                &file_path,
                &ProgressSidecar {
                    offset: rows_done,
                    rows_done,
                },
            );
        }
    }
    if let Some(sql) = flush_batch(&mut batch) {
        let slot = new_backend_handle_slot();
        session.execute_sql(&sql, 0, slot).await?;
    }

    clear_sidecar(&file_path);
    let mut p = progress.lock().unwrap();
    p.rows_done = rows_done;
    p.done = true;
    Ok(())
}

fn ensure_parent_dir(path: &str) -> std::io::Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
