use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLine {
    pub sign: char, // '+' | '-' | ' '
    pub content: String,
}

/// 生成逐行 diff（DESIGN.md §3.8.3 `create_diff`），供前端 FileChangeCard 的
/// `.fcc-diff` 直接渲染——字段形状和 `FileChangeCard.tsx` 里的 `DiffLine` 一一对应。
pub fn generate_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(old, new);
    diff.iter_all_changes()
        .map(|change| {
            let sign = match change.tag() {
                ChangeTag::Delete => '-',
                ChangeTag::Insert => '+',
                ChangeTag::Equal => ' ',
            };
            DiffLine { sign, content: change.to_string().trim_end_matches('\n').to_string() }
        })
        .collect()
}
