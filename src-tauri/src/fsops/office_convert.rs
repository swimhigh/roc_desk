use std::path::{Path, PathBuf};
use std::process::Output;

use tokio::process::Command;

use crate::error::AppError;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 用本机安装的 LibreOffice（`soffice`）把旧版二进制 Office 文档（.doc/.xls/.ppt 等
/// mammoth/xlsx.js 这类纯 JS 库解析不了的格式）临时转成 PDF，再喂给已经能用的
/// `PdfPreview` 组件展示——复用现成的 PDF 渲染管线，不用再另写一套 HTML 转换器
/// （2026-08-28 用户建议）。LibreOffice 在 Linux 上装完通常自带 `soffice` 在 PATH
/// 里，Windows 安装器默认不加 PATH，所以依次尝试 PATH 和常见安装目录——不单独
/// 起一次 `--version` 去"探测"再起一次真正转换，直接对每个候选跑转换命令本身，
/// `NotFound` 就换下一个候选，省一次进程启动的延迟。
///
/// 找不到 `soffice` 或转换失败都给清晰的错误信息，不让整个预览面板炸掉——前端
/// （`kind === "legacy-office"` 分支）拿到错误后退化成"转换失败 + 用系统程序打开"。
pub async fn convert_to_pdf(source_path: &Path, out_dir: &Path) -> Result<PathBuf, AppError> {
    tokio::fs::create_dir_all(out_dir).await.map_err(AppError::from)?;

    let mut output: Option<Output> = None;
    for candidate in soffice_candidates() {
        match run_soffice(&candidate, source_path, out_dir).await {
            Ok(o) => {
                output = Some(o);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(AppError::Internal(format!("启动 {} 失败：{e}", candidate.display()))),
        }
    }

    let output = output.ok_or_else(|| {
        AppError::Internal(
            "未检测到 LibreOffice（soffice），无法转换预览；可安装 LibreOffice 后重试，或用系统默认程序打开".into(),
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Internal(format!(
            "转换失败：{}",
            if stderr.trim().is_empty() { "soffice 退出码非零，未输出错误信息" } else { stderr.trim() }
        )));
    }

    // soffice 用源文件的 stem（不含扩展名）命名输出文件，固定落在 --outdir 下面，
    // 不需要解析它的 stdout 去猜文件名。
    let file_stem = source_path
        .file_stem()
        .ok_or_else(|| AppError::Internal(format!("无效的文件名：{}", source_path.display())))?;
    let pdf_path = out_dir.join(file_stem).with_extension("pdf");
    if !pdf_path.is_file() {
        return Err(AppError::Internal("转换命令执行成功，但没有找到生成的 PDF 文件".into()));
    }
    Ok(pdf_path)
}

fn soffice_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from("soffice")];
    #[cfg(windows)]
    {
        candidates.push(PathBuf::from(r"C:\Program Files\LibreOffice\program\soffice.exe"));
        candidates.push(PathBuf::from(r"C:\Program Files (x86)\LibreOffice\program\soffice.exe"));
    }
    candidates
}

async fn run_soffice(soffice: &Path, source_path: &Path, out_dir: &Path) -> std::io::Result<Output> {
    let mut cmd = Command::new(soffice);
    cmd.arg("--headless").arg("--convert-to").arg("pdf").arg("--outdir").arg(out_dir).arg(source_path);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.output().await
}
