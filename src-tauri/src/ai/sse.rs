use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, AppError> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some((position, delimiter_len)) = find_delimiter(&self.buffer) {
            let frame = self.buffer.drain(..position).collect::<Vec<_>>();
            self.buffer.drain(..delimiter_len);
            if let Some(event) = parse_frame(&frame)? {
                events.push(event);
            }
        }
        Ok(events)
    }

    pub fn finish(&mut self) -> Result<Option<SseEvent>, AppError> {
        if self.buffer.is_empty() {
            return Ok(None);
        }
        let frame = std::mem::take(&mut self.buffer);
        parse_frame(&frame)
    }
}

fn find_delimiter(bytes: &[u8]) -> Option<(usize, usize)> {
    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] == b'\n' && bytes[i + 1] == b'\n' {
            return Some((i, 2));
        }
        if i + 3 < bytes.len() && bytes[i..i + 4] == *b"\r\n\r\n" {
            return Some((i, 4));
        }
    }
    None
}

fn parse_frame(bytes: &[u8]) -> Result<Option<SseEvent>, AppError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| AppError::Connection(format!("SSE 包含无效 UTF-8：{e}")))?;
    let normalized = text.replace("\r\n", "\n");
    let mut event = None;
    let mut data = Vec::new();
    for line in normalized.lines() {
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').map_or((line, ""), |(field, value)| {
            (field, value.strip_prefix(' ').unwrap_or(value))
        });
        match field {
            "event" => event = Some(value.to_owned()),
            "data" => data.push(value),
            _ => {}
        }
    }
    if data.is_empty() {
        return Ok(None);
    }
    Ok(Some(SseEvent {
        event,
        data: data.join("\n"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lf_and_crlf_frames() {
        let mut parser = SseParser::default();
        let events = parser.push(b"data: one\n\ndata: two\r\n\r\n").unwrap();
        assert_eq!(events[0].data, "one");
        assert_eq!(events[1].data, "two");
    }

    #[test]
    fn joins_multiline_data_and_flushes_eof() {
        let mut parser = SseParser::default();
        assert!(parser
            .push(b"event: message\ndata: one\ndata: two")
            .unwrap()
            .is_empty());
        let event = parser.finish().unwrap().unwrap();
        assert_eq!(event.event.as_deref(), Some("message"));
        assert_eq!(event.data, "one\ntwo");
    }

    #[test]
    fn preserves_split_utf8() {
        let mut parser = SseParser::default();
        let bytes = "data: 中文\n\n".as_bytes();
        assert!(parser.push(&bytes[..8]).unwrap().is_empty());
        assert_eq!(parser.push(&bytes[8..]).unwrap()[0].data, "中文");
    }
}
