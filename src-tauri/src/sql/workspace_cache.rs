use std::path::PathBuf;

use uuid::Uuid;

use crate::error::AppError;

/// SQL 工作区的本地目录缓存（docs/SQL_DESKTOP_PLAN.md §4.4）：每个数据源在
/// `<cache_root>/sql/<data_source_id>/` 下有一份真实的磁盘目录，标签页对应
/// `queries/<tab_id>.sql` 文件，好让 AI 编程助手那套 `ChangeStore` 文件改动
/// 状态机能直接拿它当"文件"用，不用再发明一套"纯内存改动"的机制。
///
/// `cache_root` 复用 `WorkspaceManager` 已有的 cache_root（`app_data_dir` 的
/// 兄弟子目录），不新增一套缓存根目录管理逻辑。
pub struct SqlWorkspaceCache {
    root: PathBuf,
}

impl SqlWorkspaceCache {
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            root: cache_root.join("sql"),
        }
    }

    fn data_source_dir(&self, data_source_id: Uuid) -> PathBuf {
        self.root.join(data_source_id.to_string())
    }

    fn queries_dir(&self, data_source_id: Uuid) -> PathBuf {
        self.data_source_dir(data_source_id).join("queries")
    }

    pub fn schema_cache_dir(&self, data_source_id: Uuid) -> PathBuf {
        self.data_source_dir(data_source_id).join("schema_cache")
    }

    pub fn exports_dir(&self, data_source_id: Uuid) -> PathBuf {
        self.data_source_dir(data_source_id).join("exports")
    }

    fn ensure_dirs(&self, data_source_id: Uuid) -> Result<(), AppError> {
        std::fs::create_dir_all(self.queries_dir(data_source_id))?;
        std::fs::create_dir_all(self.schema_cache_dir(data_source_id))?;
        std::fs::create_dir_all(self.exports_dir(data_source_id))?;
        Ok(())
    }

    /// 新建查询标签页时立刻分配一个确定性路径并建好空文件——AI 的文件改动
    /// 状态机（`ChangeStore::stage`）按路径字符串做身份匹配，需要路径提前
    /// 存在（方案 §4.4："路径在创建时就定好"）。返回存进 `sql_workspace_tabs.
    /// file_path` 的相对路径（`queries/<tab_id>.sql`）。
    pub fn create_tab_file(&self, data_source_id: Uuid, tab_id: Uuid) -> Result<String, AppError> {
        self.ensure_dirs(data_source_id)?;
        let rel = format!("queries/{tab_id}.sql");
        let abs = self.data_source_dir(data_source_id).join(&rel);
        if !abs.exists() {
            std::fs::write(&abs, "")?;
        }
        Ok(rel)
    }

    pub fn delete_tab_file(&self, data_source_id: Uuid, rel_path: &str) -> Result<(), AppError> {
        let abs = self.guard_path(data_source_id, rel_path)?;
        if abs.exists() {
            std::fs::remove_file(abs)?;
        }
        Ok(())
    }

    pub fn read_tab_content(&self, data_source_id: Uuid, rel_path: &str) -> Result<String, AppError> {
        let abs = self.guard_path(data_source_id, rel_path)?;
        Ok(std::fs::read_to_string(abs).unwrap_or_default())
    }

    pub fn write_tab_content(
        &self,
        data_source_id: Uuid,
        rel_path: &str,
        content: &str,
    ) -> Result<(), AppError> {
        self.ensure_dirs(data_source_id)?;
        let abs = self.guard_path(data_source_id, rel_path)?;
        std::fs::write(abs, content)?;
        Ok(())
    }

    pub fn write_schema_ddl(
        &self,
        data_source_id: Uuid,
        schema: &str,
        object: &str,
        ddl: &str,
    ) -> Result<(), AppError> {
        self.ensure_dirs(data_source_id)?;
        let safe_name = format!(
            "{}__{}.ddl.sql",
            sanitize_filename_component(schema),
            sanitize_filename_component(object)
        );
        std::fs::write(self.schema_cache_dir(data_source_id).join(safe_name), ddl)?;
        Ok(())
    }

    /// AI 文件工具（read_file/write_file/edit_file）作用域的安全边界——现有
    /// "AI 编程助手"的同类工具完全没有路径越界校验（只靠系统提示词自觉，见
    /// docs/SQL_DESKTOP_PLAN.md §4.4"必须补的安全边界"），SQL 工作区这里涉及
    /// 连接凭据引用等相对敏感的信息，新写的这条路径必须显式拒绝任何落在
    /// `<cache_root>/sql/<data_source_id>/` 之外的路径，不能沿用现状的
    /// "无边界"实现。故意不去改造/复用 `commands/fs.rs::guard_local_path`——
    /// 那是私有函数且和 `WorkspaceHandle` 绑定，为了这里的用途去改造它会
    /// 牵涉到 Explorer 现有的调用点，风险和收益不成比例，这里独立写一份等价
    /// 但更小范围的校验。
    pub fn guard_path(&self, data_source_id: Uuid, rel_path: &str) -> Result<PathBuf, AppError> {
        let root = self.data_source_dir(data_source_id);
        std::fs::create_dir_all(&root)?;
        let root_canon = root
            .canonicalize()
            .map_err(|e| AppError::Internal(format!("SQL 工作区目录不可用：{e}")))?;
        let candidate = root.join(rel_path);
        let candidate_canon = match candidate.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                // 文件可能还没创建（比如即将写入的新文件）——退化成校验父目录，
                // 和 `commands/fs.rs::guard_local_path` 对"新建文件"场景的处理
                // 思路一致。
                let parent = candidate.parent().ok_or_else(|| {
                    AppError::PermissionDenied(format!("非法路径：{rel_path}"))
                })?;
                let parent_canon = parent
                    .canonicalize()
                    .map_err(|_| AppError::PermissionDenied(format!("非法路径：{rel_path}")))?;
                if !parent_canon.starts_with(&root_canon) {
                    return Err(AppError::PermissionDenied(format!(
                        "路径 {rel_path} 越界，拒绝访问"
                    )));
                }
                return Ok(candidate);
            }
        };
        if !candidate_canon.starts_with(&root_canon) {
            return Err(AppError::PermissionDenied(format!(
                "路径 {rel_path} 越界，拒绝访问"
            )));
        }
        Ok(candidate_canon)
    }

    pub fn data_source_root(&self, data_source_id: Uuid) -> PathBuf {
        self.data_source_dir(data_source_id)
    }
}

fn sanitize_filename_component(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}
