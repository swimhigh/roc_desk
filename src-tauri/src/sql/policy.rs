use sqlparser::ast::Statement;
use sqlparser::dialect::{Dialect, GenericDialect, MySqlDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;

use crate::error::AppError;
use crate::sql::model::DbKind;

/// 语句执行分类（docs/SQL_DESKTOP_PLAN.md §7、§4.2.1，参考 rainfrog
/// `src/database/mod.rs::ExecutionType` 的思路）：
/// - `Normal`：只读或数据源允许直接执行的语句，直接跑。
/// - `Confirm`：DDL 等不支持事务内回滚的语句，需要用户在执行前弹窗确认。
/// - `Transaction`：INSERT/UPDATE/DELETE 等支持事务回滚的写语句，先在独立
///   事务里执行、展示真实受影响行数，用户确认后再 commit，否则 rollback。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionKind {
    Normal,
    Confirm,
    Transaction,
}

fn dialect_for(kind: DbKind) -> Box<dyn Dialect> {
    match kind {
        DbKind::Mysql | DbKind::Tdsql => Box::new(MySqlDialect {}),
        DbKind::Postgres | DbKind::Opengauss => Box::new(PostgreSqlDialect {}),
        // sqlparser 没有专门的 T-SQL/PL-SQL 方言实现，用通用方言做尽力而为的
        // 分类——SQL Server/Oracle 的语句类型分类（UPDATE/DELETE/DDL 这些
        // 基本语法是通用的）不受太大影响，真正的方言细节交给对应 adapter
        // 处理，policy 这一层只关心"这是不是写操作"。
        DbKind::SqlServer | DbKind::Oracle => Box::new(GenericDialect {}),
    }
}

/// 只允许单条语句：多语句一次提交无法给出单一、明确的确认/回滚粒度，
/// 直接拒绝，让用户拆开逐条执行（参考 rainfrog `get_first_query` 的
/// 同款限制）。
fn parse_single_statement(sql: &str, kind: DbKind) -> Result<Statement, AppError> {
    let dialect = dialect_for(kind);
    let statements = Parser::parse_sql(&*dialect, sql)
        .map_err(|e| AppError::Internal(format!("SQL 解析失败：{e}")))?;
    match statements.len() {
        0 => Err(AppError::Internal("SQL 内容为空".into())),
        1 => Ok(statements.into_iter().next().expect("checked len == 1")),
        _ => Err(AppError::Internal(
            "一次只能执行一条语句，请拆分后逐条执行".into(),
        )),
    }
}

/// 语句分类失败（解析失败、遇到未识别的语句类型）一律降级为 `Confirm`，
/// 不放行——docs/SQL_DESKTOP_PLAN.md §7 明确要求"解析失败或分类未知一律
/// 降级为需要确认"，绝不能因为分类器不认识某个语句就当作 `Normal` 直接跑。
pub fn classify(sql: &str, kind: DbKind) -> ExecutionKind {
    match parse_single_statement(sql, kind) {
        Ok(statement) => classify_statement(&statement),
        Err(_) => ExecutionKind::Confirm,
    }
}

/// 供 `commands::sql` 在真正执行前重新拿到解析结果（避免重复解析两次的
/// 唯一理由是错误信息更精确），解析失败时返回错误而不是静默分类成
/// `Confirm`——这个函数是"准备真正执行"用的，调用方需要知道具体哪里解析
/// 错了；`classify` 是"只是想知道要不要弹确认框"用的降级版本。
pub fn parse_and_classify(sql: &str, kind: DbKind) -> Result<(Statement, ExecutionKind), AppError> {
    let statement = parse_single_statement(sql, kind)?;
    let execution_kind = classify_statement(&statement);
    Ok((statement, execution_kind))
}

fn classify_statement(statement: &Statement) -> ExecutionKind {
    match statement {
        Statement::Query(_) | Statement::Explain { .. } => ExecutionKind::Normal,
        Statement::Insert(_) => ExecutionKind::Normal,
        Statement::Update { .. } | Statement::Delete(_) => ExecutionKind::Transaction,
        Statement::CreateTable(_)
        | Statement::AlterTable { .. }
        | Statement::AlterIndex { .. }
        | Statement::AlterView { .. }
        | Statement::AlterRole { .. }
        | Statement::Drop { .. }
        | Statement::DropFunction { .. }
        | Statement::Truncate { .. }
        | Statement::CreateIndex(_)
        | Statement::CreateView { .. }
        | Statement::CreateSchema { .. }
        | Statement::CreateFunction { .. } => ExecutionKind::Confirm,
        // 不认识的语句类型（各方言各种扩展语法太多，穷举不完）一律按最保守的
        // 路径处理：不是明确知道安全的只读语句，就需要确认，不默认放行。
        _ => ExecutionKind::Confirm,
    }
}

pub fn is_readonly_kind(kind: ExecutionKind) -> bool {
    matches!(kind, ExecutionKind::Normal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_statements() {
        assert_eq!(
            classify("SELECT * FROM users", DbKind::Postgres),
            ExecutionKind::Normal
        );
        assert_eq!(
            classify(
                "UPDATE users SET name = 'a' WHERE id = 1",
                DbKind::Postgres
            ),
            ExecutionKind::Transaction
        );
        assert_eq!(
            classify("DELETE FROM users WHERE id = 1", DbKind::Postgres),
            ExecutionKind::Transaction
        );
        assert_eq!(
            classify("DROP TABLE users", DbKind::Postgres),
            ExecutionKind::Confirm
        );
        assert_eq!(
            classify("INSERT INTO users (name) VALUES ('a')", DbKind::Postgres),
            ExecutionKind::Normal
        );
    }

    #[test]
    fn parse_failure_and_multi_statement_downgrade_to_confirm() {
        assert_eq!(classify("SELEC * FORM users", DbKind::Postgres), ExecutionKind::Confirm);
        assert_eq!(
            classify("SELECT 1; SELECT 2;", DbKind::Postgres),
            ExecutionKind::Confirm
        );
    }
}
