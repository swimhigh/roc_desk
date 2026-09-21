use serde::Deserialize;

use crate::sql::model::{DbKind, ObjectRef};

/// 标识符引用——和 `adapter::preview_sql` 里那份手写的一样，这里独立一份
/// 而不是互相依赖，避免为了共用几行代码在两个已经很小的模块之间建一条
/// 引用关系；出现分叉时两处都要改，但两处逻辑本来就极其稳定（各数据库的
/// 引用符号是固定事实，不会变）。
pub fn quote_ident(kind: DbKind, s: &str) -> String {
    match kind {
        DbKind::Mysql | DbKind::Tdsql => format!("`{}`", s.replace('`', "``")),
        DbKind::SqlServer => format!("[{}]", s.replace(']', "]]")),
        _ => format!("\"{}\"", s.replace('"', "\"\"")),
    }
}

pub fn quote_table(kind: DbKind, object: &ObjectRef) -> String {
    if object.schema.is_empty() {
        quote_ident(kind, &object.name)
    } else {
        format!(
            "{}.{}",
            quote_ident(kind, &object.schema),
            quote_ident(kind, &object.name)
        )
    }
}

/// 表格编辑器传进来的单元格值——前端只区分"是不是 NULL"和"文本内容"，不尝试
/// 按列类型做客户端校验（那是数据库自己的事，交给它在执行时报错）。二进制
/// 列这一版不支持通过表格编辑器改写（见 `commands::sql` 里表格数据接口的
/// 说明），所以这里不需要处理十六进制/二进制字面量。
#[derive(Debug, Clone, Deserialize)]
pub struct CellInput {
    pub is_null: bool,
    pub text: String,
}

/// 主键列名 + 值，`WHERE` 子句按这个精确拼——用带名字段的结构体而不是
/// `(String, CellInput)` 元组，元组在 JSON 上是"两元素数组"，前端写起来
/// 容易搞混顺序，命名字段更不容易出错。
#[derive(Debug, Clone, Deserialize)]
pub struct NamedCell {
    pub name: String,
    pub value: CellInput,
}

/// 统一按字符串字面量拼——三种数据库对字符串字面量赋值给数值/日期等列都会
/// 做隐式转换，不需要知道真实列类型也能正确写入，换来的是不需要在这里维护
/// 一份"列类型 -> 该怎么拼字面量"的映射表。传入的值来自用户在表格里手打的
/// 文本，不是拼接任意来源的字符串，这里只做最基本的引号转义。
fn quote_literal(value: &CellInput) -> String {
    if value.is_null {
        return "NULL".to_string();
    }
    format!("'{}'", value.text.replace('\'', "''"))
}

fn where_clause(kind: DbKind, pk: &[NamedCell]) -> String {
    pk.iter()
        .map(|c| format!("{} = {}", quote_ident(kind, &c.name), quote_literal(&c.value)))
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// 分页浏览用的查询——SQL Server 的 `OFFSET ... FETCH` 语法强制要求
/// `ORDER BY`，没有主键可用时退回 `(SELECT NULL)` 这个常见占位写法（顺序
/// 不保证跨查询稳定，但分页浏览本来就是"看一眼"用途，不是要求强一致性的
/// 场景）。
pub fn build_page_query(
    kind: DbKind,
    object: &ObjectRef,
    order_col: Option<&str>,
    limit: usize,
    offset: usize,
) -> String {
    let table = quote_table(kind, object);
    match kind {
        DbKind::SqlServer => {
            let order = order_col
                .map(|c| quote_ident(kind, c))
                .unwrap_or_else(|| "(SELECT NULL)".to_string());
            format!(
                "SELECT * FROM {table} ORDER BY {order} OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY"
            )
        }
        _ => format!("SELECT * FROM {table} LIMIT {limit} OFFSET {offset}"),
    }
}

pub fn build_count_query(kind: DbKind, object: &ObjectRef) -> String {
    format!("SELECT COUNT(*) FROM {}", quote_table(kind, object))
}

/// 单元格编辑走独立一条 `UPDATE ... WHERE <主键组合>` 直接执行，不经过
/// `sql_execute` 的 Confirm/Transaction 确认闸门——那套闸门是给"用户手打的
/// 任意 SQL"设计的防线（怕 WHERE 写漏了改到一整张表）；这里的 WHERE 由
/// 后端按主键精确拼出，不可能误伤别的行，点一下单元格改一个值就弹一次
/// "确认执行"的二次确认反而是骚扰。仍然尊重数据源的只读开关（调用方
/// `commands::sql` 检查），并记入查询历史留痕。
pub fn build_update_cell(
    kind: DbKind,
    object: &ObjectRef,
    pk: &[NamedCell],
    column: &str,
    value: &CellInput,
) -> String {
    format!(
        "UPDATE {} SET {} = {} WHERE {}",
        quote_table(kind, object),
        quote_ident(kind, column),
        quote_literal(value),
        where_clause(kind, pk)
    )
}

pub fn build_delete_row(kind: DbKind, object: &ObjectRef, pk: &[NamedCell]) -> String {
    format!(
        "DELETE FROM {} WHERE {}",
        quote_table(kind, object),
        where_clause(kind, pk)
    )
}

pub fn build_insert_row(kind: DbKind, object: &ObjectRef, values: &[NamedCell]) -> String {
    let cols = values
        .iter()
        .map(|c| quote_ident(kind, &c.name))
        .collect::<Vec<_>>()
        .join(", ");
    let vals = values
        .iter()
        .map(|c| quote_literal(&c.value))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO {} ({}) VALUES ({})",
        quote_table(kind, object),
        cols,
        vals
    )
}

/// 表结构修改（2026-09 用户反馈"查看表的界面里可以修改表结构"）——这里只
/// 负责生成 DDL 文本，不直接执行：调用方把生成的语句塞进一个新 SQL 标签页
/// 让用户过一眼再运行，复用已有的 DDL 确认闸门（`policy::classify` 会把
/// ALTER TABLE 分类成 `Confirm`），不需要为表结构修改单独做一套确认 UI，
/// 也让用户在真正执行前有机会用 AI 面板"解释/优化"一下这段 DDL。
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AlterOp {
    AddColumn {
        name: String,
        data_type: String,
        nullable: bool,
    },
    DropColumn {
        name: String,
    },
    RenameColumn {
        old_name: String,
        new_name: String,
    },
}

pub fn build_alter_table(kind: DbKind, object: &ObjectRef, ops: &[AlterOp]) -> Vec<String> {
    let table = quote_table(kind, object);
    ops.iter()
        .map(|op| match op {
            AlterOp::AddColumn {
                name,
                data_type,
                nullable,
            } => {
                let null_sql = if *nullable { "" } else { " NOT NULL" };
                // SQL Server 的 ADD 语法不带 COLUMN 关键字（带了是语法错误），
                // Postgres/MySQL 两种写法都认，这里统一用带 COLUMN 的写法更
                // 明确可读。
                match kind {
                    DbKind::SqlServer => format!(
                        "ALTER TABLE {table} ADD {} {data_type}{null_sql}",
                        quote_ident(kind, name)
                    ),
                    _ => format!(
                        "ALTER TABLE {table} ADD COLUMN {} {data_type}{null_sql}",
                        quote_ident(kind, name)
                    ),
                }
            }
            AlterOp::DropColumn { name } => {
                format!("ALTER TABLE {table} DROP COLUMN {}", quote_ident(kind, name))
            }
            AlterOp::RenameColumn { old_name, new_name } => match kind {
                // SQL Server 没有 `ALTER TABLE ... RENAME COLUMN` 语法，重命名
                // 走存储过程 `sp_rename`，参数形式是 "schema.table.column"。
                DbKind::SqlServer => format!(
                    "EXEC sp_rename '{}.{}.{}', '{}', 'COLUMN'",
                    object.schema, object.name, old_name, new_name
                ),
                _ => format!(
                    "ALTER TABLE {table} RENAME COLUMN {} TO {}",
                    quote_ident(kind, old_name),
                    quote_ident(kind, new_name)
                ),
            },
        })
        .collect()
}
