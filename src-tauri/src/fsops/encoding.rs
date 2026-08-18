/// 编辑器状态栏展示 + "Reopen/Save with Encoding" 菜单用的编码标签集合
/// （参考 VS Code 的编码选择器）。
pub const SUPPORTED_ENCODINGS: &[&str] = &["UTF-8", "GBK", "UTF-16LE", "UTF-16BE"];

/// 把任意字节内容解码成 `String`，同时返回探测到的编码标签
/// （DESIGN.md §3.1.4 编辑器要支持常见文本文件）。
///
/// 不能假设都是 UTF-8——Windows 上大量遗留 `.txt`/`.log`/`.ini`/`.h` 是 GBK 编码，
/// 用 `String::from_utf8`/`read_to_string` 直接读会因为不是合法 UTF-8 而失败
/// （本地）或被 `from_utf8_lossy` 静默替换成乱码（远程），两种都不算真正"支持"。
/// 探测顺序：BOM 优先（明确声明了编码）→ 严格 UTF-8 → GBK → 都失败就 UTF-8 lossy 兜底，
/// 保证任何字节序列都能返回点什么，而不是让编辑器直接打不开。
pub fn decode_text_detect(bytes: &[u8]) -> (String, &'static str) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (String::from_utf8_lossy(rest).into_owned(), "UTF-8");
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return (encoding_rs::UTF_16LE.decode(rest).0.into_owned(), "UTF-16LE");
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return (encoding_rs::UTF_16BE.decode(rest).0.into_owned(), "UTF-16BE");
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return (s.to_string(), "UTF-8");
    }
    let (text, _, had_errors) = encoding_rs::GBK.decode(bytes);
    if !had_errors {
        return (text.into_owned(), "GBK");
    }
    (String::from_utf8_lossy(bytes).into_owned(), "UTF-8")
}

pub fn decode_text(bytes: &[u8]) -> String {
    decode_text_detect(bytes).0
}

fn encoding_for(label: &str) -> Option<&'static encoding_rs::Encoding> {
    match label {
        "UTF-8" => Some(encoding_rs::UTF_8),
        "GBK" => Some(encoding_rs::GBK),
        "UTF-16LE" => Some(encoding_rs::UTF_16LE),
        "UTF-16BE" => Some(encoding_rs::UTF_16BE),
        _ => None,
    }
}

/// "Reopen with Encoding"：用户显式指定编码时不再自动探测，直接按指定编码解码
/// （去掉可能存在的对应 BOM，避免 BOM 被当成一个字符解出来）。
pub fn decode_with(bytes: &[u8], label: &str) -> Result<String, String> {
    let enc = encoding_for(label).ok_or_else(|| format!("不支持的编码：{label}"))?;
    let bytes = match label {
        "UTF-8" => bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes),
        "UTF-16LE" => bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes),
        "UTF-16BE" => bytes.strip_prefix(&[0xFE, 0xFF]).unwrap_or(bytes),
        _ => bytes,
    };
    Ok(enc.decode(bytes).0.into_owned())
}

/// "Save with Encoding"：按指定编码把文本编码回字节，写盘前用。
pub fn encode_with(text: &str, label: &str) -> Result<Vec<u8>, String> {
    let enc = encoding_for(label).ok_or_else(|| format!("不支持的编码：{label}"))?;
    Ok(enc.encode(text).0.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_plain_utf8() {
        assert_eq!(decode_text("你好 hello".as_bytes()), "你好 hello");
    }

    #[test]
    fn decodes_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("hello".as_bytes());
        assert_eq!(decode_text(&bytes), "hello");
    }

    #[test]
    fn decodes_gbk() {
        let (encoded, _, _) = encoding_rs::GBK.encode("你好世界");
        assert_eq!(decode_text(&encoded), "你好世界");
    }

    #[test]
    fn roundtrips_forced_encoding() {
        let bytes = encode_with("你好世界", "GBK").unwrap();
        assert_eq!(decode_with(&bytes, "GBK").unwrap(), "你好世界");
    }
}
