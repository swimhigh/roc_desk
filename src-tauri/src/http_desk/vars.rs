//! 多层变量作用域解析 + `{{ }}` 插值（docs/HTTP_DESKTOP_PLAN.md §4.5）。
//!
//! MVP 范围内是三层（环境 > 集合 > 全局），没有实现方案里第 1 层"请求级局部
//! 变量"和第 3 层"文件夹变量"——两者都要靠前置脚本才有意义（脚本运行期临时写入/
//! 文件夹自己的配置继承），而脚本引擎（方案 §4.3 的 QuickJS 沙箱）本轮没有实现，
//! 见 http_desk/mod.rs 顶部的范围说明。

use uuid::Uuid;

use super::model::EnvVar;

pub struct VarContext<'a> {
    pub environment: &'a [EnvVar],
    pub collection: &'a [EnvVar],
    pub global: &'a [EnvVar],
}

impl<'a> VarContext<'a> {
    pub fn resolve(&self, key: &str) -> Option<String> {
        self.environment
            .iter()
            .chain(self.collection.iter())
            .chain(self.global.iter())
            .find(|v| v.enabled && v.key == key)
            .map(|v| v.value.clone())
    }

    /// 敏感变量集合——插值仍然用明文（发出去的请求必须是真值），但调用方在写
    /// 历史记录/日志/AI 上下文之前应该用这个集合把值替换成 `***`
    /// （docs/HTTP_DESKTOP_PLAN.md §7）。
    pub fn secret_keys(&self) -> std::collections::HashSet<&str> {
        self.environment
            .iter()
            .chain(self.collection.iter())
            .chain(self.global.iter())
            .filter(|v| v.secret)
            .map(|v| v.key.as_str())
            .collect()
    }
}

/// 内置动态值（docs/HTTP_DESKTOP_PLAN.md §4.6）——MVP 阶段只用项目已有的
/// `uuid`/`chrono` 依赖实现最常用的几个，不新增 `fake` crate（降低本轮依赖面）。
fn dynamic_value(name: &str) -> Option<String> {
    match name {
        "$uuid" | "$guid" => Some(Uuid::new_v4().to_string()),
        "$timestamp" => Some(chrono::Utc::now().timestamp().to_string()),
        "$isoTimestamp" => Some(chrono::Utc::now().to_rfc3339()),
        "$randomInt" => {
            // 0..=99999，够用作占位数据；不需要密码学随机，用时间戳低位够了。
            let nanos = chrono::Utc::now().timestamp_subsec_nanos();
            Some((nanos % 100000).to_string())
        }
        _ => None,
    }
}

/// 把字符串里所有 `{{ key }}`/`{{$dynamicFn}}` 替换成实际值；找不到的变量原样
/// 保留（不静默清空，方便用户一眼看出"这个变量没生效"，对齐 §3.3 未定义变量
/// 标红的产品意图——标红是前端 UI 层的事，这里只保证后端不会把它错误地替换成
/// 空字符串）。手写扫描而不是引入 `regex` crate：模式简单（`{{` 到最近的 `}}`），
/// 没必要为了这一处多背一个新依赖。
pub fn interpolate(input: &str, ctx: &VarContext) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = input[i + 2..].find("}}") {
                let key = input[i + 2..i + 2 + end].trim();
                let resolved = dynamic_value(key).or_else(|| ctx.resolve(key));
                match resolved {
                    Some(v) => out.push_str(&v),
                    None => out.push_str(&input[i..i + 2 + end + 2]),
                }
                i += 2 + end + 2;
                continue;
            }
        }
        // 按字节推进但要保持 UTF-8 边界——找下一个字符边界而不是简单 +1。
        let ch_len = input[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&input[i..i + ch_len]);
        i += ch_len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_layered_variables() {
        let env = vec![EnvVar {
            key: "base_url".into(),
            value: "https://dev.example.com".into(),
            secret: false,
            enabled: true,
        }];
        let collection = vec![EnvVar {
            key: "token".into(),
            value: "abc123".into(),
            secret: true,
            enabled: true,
        }];
        let ctx = VarContext {
            environment: &env,
            collection: &collection,
            global: &[],
        };
        let out = interpolate("{{base_url}}/users?token={{token}}", &ctx);
        assert_eq!(out, "https://dev.example.com/users?token=abc123");
    }

    #[test]
    fn leaves_unknown_variables_untouched() {
        let ctx = VarContext {
            environment: &[],
            collection: &[],
            global: &[],
        };
        assert_eq!(interpolate("{{missing}}", &ctx), "{{missing}}");
    }

    #[test]
    fn dynamic_uuid_is_replaced() {
        let ctx = VarContext {
            environment: &[],
            collection: &[],
            global: &[],
        };
        let out = interpolate("{{$uuid}}", &ctx);
        assert!(Uuid::parse_str(&out).is_ok());
    }
}
