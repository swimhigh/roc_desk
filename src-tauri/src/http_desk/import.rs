//! 导入解析器——本轮只落地 curl 命令解析（docs/HTTP_DESKTOP_PLAN.md §4.7 里最快
//! 能覆盖"从浏览器 DevTools/Postman 复制一条 curl 命令"这个高频场景的一种）。
//! Postman Collection/OpenAPI/HAR 导入没有在本轮实现，留作后续任务
//! （见 http_desk/mod.rs 顶部范围说明）。

use crate::error::AppError;

use super::model::*;

/// 简化版 shell 分词：处理单/双引号、反斜杠转义和行尾 `\` 续行（curl 多行命令的
/// 常见写法），不追求 100% 复刻 POSIX shell 分词规则——覆盖"从浏览器/Postman
/// 复制的 curl 命令"这个目标场景足够。
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = input.trim().chars().peekable();

    while let Some(c) = chars.next() {
        if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                current.push(c);
            }
            continue;
        }
        if in_double {
            if c == '"' {
                in_double = false;
            } else if c == '\\' {
                match chars.peek() {
                    Some('"') | Some('\\') | Some('$') | Some('`') => {
                        current.push(chars.next().unwrap());
                    }
                    _ => current.push(c),
                }
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '\'' => {
                in_single = true;
                started = true;
            }
            '"' => {
                in_double = true;
                started = true;
            }
            '\\' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                    continue;
                }
                if let Some(next) = chars.next() {
                    current.push(next);
                    started = true;
                }
            }
            c if c.is_whitespace() => {
                if started {
                    tokens.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        tokens.push(current);
    }
    tokens
}

/// 把一条 curl 命令解析成 `RequestDef`（`id`/`name` 留给调用方补——导入是
/// "生成一份草稿"，落盘/命名由 `commands::http_desk::http_import_curl` 决定）。
pub fn parse_curl(command: &str) -> Result<RequestDef, AppError> {
    let tokens = tokenize(command);
    let mut iter = tokens.into_iter().peekable();
    if iter.peek().map(|s| s.as_str()) == Some("curl") {
        iter.next();
    }

    let mut url: Option<String> = None;
    let mut method: Option<String> = None;
    let mut headers = Vec::new();
    let mut data: Option<String> = None;
    let mut basic_auth: Option<(String, String)> = None;

    while let Some(tok) = iter.next() {
        match tok.as_str() {
            "-X" | "--request" => method = iter.next(),
            "-H" | "--header" => {
                if let Some(h) = iter.next() {
                    if let Some((k, v)) = h.split_once(':') {
                        headers.push(KeyValueItem {
                            key: k.trim().to_string(),
                            value: v.trim().to_string(),
                            enabled: true,
                        });
                    }
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-urlencode" => {
                if let Some(d) = iter.next() {
                    data = Some(match data {
                        Some(existing) => format!("{existing}&{d}"),
                        None => d,
                    });
                }
            }
            "-u" | "--user" => {
                if let Some(cred) = iter.next() {
                    basic_auth = Some(match cred.split_once(':') {
                        Some((u, p)) => (u.to_string(), p.to_string()),
                        None => (cred, String::new()),
                    });
                }
            }
            "-b" | "--cookie" => {
                if let Some(c) = iter.next() {
                    headers.push(KeyValueItem {
                        key: "Cookie".into(),
                        value: c,
                        enabled: true,
                    });
                }
            }
            "-A" | "--user-agent" => {
                if let Some(v) = iter.next() {
                    headers.push(KeyValueItem {
                        key: "User-Agent".into(),
                        value: v,
                        enabled: true,
                    });
                }
            }
            "--compressed" | "-k" | "--insecure" | "-s" | "--silent" | "-v" | "--verbose"
            | "-i" | "--include" | "-L" | "--location" => {}
            other if other.starts_with('-') => {
                // 未识别的 flag：保守地吞掉紧随其后、看起来是它的值的一个 token
                // （不是以 `-` 开头），避免把值误判成 URL——不追求识别所有 curl
                // flag，只求不把大多数场景下的 URL 解析搞错。
                if iter.peek().map(|s| !s.starts_with('-')).unwrap_or(false) {
                    iter.next();
                }
            }
            other => {
                if url.is_none() {
                    url = Some(other.trim_matches(['\'', '"']).to_string());
                }
            }
        }
    }

    let url = url.ok_or_else(|| AppError::Internal("未能从 curl 命令中解析出 URL".into()))?;
    let method = method.unwrap_or_else(|| if data.is_some() { "POST".into() } else { "GET".into() });

    let body = match data {
        Some(d) => {
            let looks_json = headers
                .iter()
                .any(|h| h.key.eq_ignore_ascii_case("content-type") && h.value.contains("json"))
                || d.trim_start().starts_with('{')
                || d.trim_start().starts_with('[');
            if looks_json {
                RequestBody::Json { content: d }
            } else {
                RequestBody::Raw {
                    content: d,
                    content_type: "application/x-www-form-urlencoded".into(),
                }
            }
        }
        None => RequestBody::None,
    };

    let auth = match basic_auth {
        Some((username, password)) => AuthConfig::Basic { username, password },
        None => AuthConfig::None,
    };

    Ok(RequestDef {
        id: String::new(),
        name: "导入的请求".into(),
        method: method.to_uppercase(),
        url,
        params: Vec::new(),
        headers,
        auth,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_get() {
        let req = parse_curl("curl https://api.example.com/users").unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.url, "https://api.example.com/users");
    }

    #[test]
    fn parses_post_with_json_body_and_headers() {
        let req = parse_curl(
            r#"curl -X POST https://api.example.com/users -H "Content-Type: application/json" -H "Authorization: Bearer abc" -d '{"name":"a"}'"#,
        )
        .unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.headers.len(), 2);
        match req.body {
            RequestBody::Json { content } => assert_eq!(content, r#"{"name":"a"}"#),
            _ => panic!("expected json body"),
        }
    }

    #[test]
    fn parses_basic_auth() {
        let req = parse_curl("curl -u admin:secret https://api.example.com/ping").unwrap();
        match req.auth {
            AuthConfig::Basic { username, password } => {
                assert_eq!(username, "admin");
                assert_eq!(password, "secret");
            }
            _ => panic!("expected basic auth"),
        }
    }
}
