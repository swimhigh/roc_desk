use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct OcrWord {
    pub text: String,
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OcrLine {
    pub text: String,
    pub words: Vec<OcrWord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OcrResult {
    pub width: u32,
    pub height: u32,
    pub language: String,
    pub text: String,
    pub lines: Vec<OcrLine>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OcrImageInput {
    pub data_base64: String,
}

#[tauri::command]
pub async fn editor_ocr_image(input: OcrImageInput) -> Result<OcrResult, AppError> {
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, input.data_base64)
        .map_err(|e| AppError::Internal(format!("图片数据解码失败：{e}")))?;
    if bytes.is_empty() || bytes.len() > 20 * 1024 * 1024 {
        return Err(AppError::Internal("图片为空或超过 OCR 20 MB 限制".into()));
    }
    #[cfg(windows)]
    { recognize_windows(&bytes).await }
    #[cfg(not(windows))]
    { let _ = bytes; Err(AppError::Internal("当前平台暂不支持 Windows OCR".into())) }
}

#[cfg(windows)]
async fn recognize_windows(bytes: &[u8]) -> Result<OcrResult, AppError> {
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

    let stream = InMemoryRandomAccessStream::new().map_err(win_err)?;
    let writer = DataWriter::CreateDataWriter(&stream).map_err(win_err)?;
    writer.WriteBytes(bytes).map_err(win_err)?;
    writer.StoreAsync().map_err(win_err)?.await.map_err(win_err)?;
    stream.Seek(0).map_err(win_err)?;
    let decoder = BitmapDecoder::CreateAsync(&stream).map_err(win_err)?.await.map_err(win_err)?;
    let bitmap = decoder.GetSoftwareBitmapAsync().map_err(win_err)?.await.map_err(win_err)?;
    let width = bitmap.PixelWidth().map_err(win_err)? as u32;
    let height = bitmap.PixelHeight().map_err(win_err)? as u32;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|e| AppError::Internal(format!("Windows OCR 不可用，请安装中文语言包：{e}")))?;
    let language = engine.RecognizerLanguage().map_err(win_err)?.LanguageTag().map_err(win_err)?.to_string_lossy();
    let result = engine.RecognizeAsync(&bitmap).map_err(win_err)?.await.map_err(win_err)?;
    let text = result.Text().map_err(win_err)?.to_string_lossy();
    let lines_view = result.Lines().map_err(win_err)?;
    let mut lines = Vec::new();
    for i in 0..lines_view.Size().map_err(win_err)? {
        let line = lines_view.GetAt(i).map_err(win_err)?;
        let line_text = line.Text().map_err(win_err)?.to_string_lossy();
        let words_view = line.Words().map_err(win_err)?;
        let mut words = Vec::new();
        for j in 0..words_view.Size().map_err(win_err)? {
            let word = words_view.GetAt(j).map_err(win_err)?;
            let rect = word.BoundingRect().map_err(win_err)?;
            words.push(OcrWord { text: word.Text().map_err(win_err)?.to_string_lossy(), left: rect.X as f64, top: rect.Y as f64, width: rect.Width as f64, height: rect.Height as f64 });
        }
        lines.push(OcrLine { text: line_text, words });
    }
    Ok(OcrResult { width, height, language, text, lines })
}

#[cfg(windows)]
fn win_err(e: windows::core::Error) -> AppError { AppError::Internal(e.to_string()) }
