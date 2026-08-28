import React from "react";
import { FileWarning } from "lucide-react";
import { PdfPreview } from "./PdfPreview";

interface LegacyOfficePreviewProps {
  path: string;
  /** 转成功后的 PDF data URL（`data:application/pdf;base64,...`），转换失败时为空。*/
  content: string;
  /** 转换失败时的错误信息（通常是没装 LibreOffice），和 BinaryInfoPanel/JarInfoPanel
   * 解析失败时同样的退化模式：仍然能打开这个 Tab，只是展示成"转换失败 + 用系统
   * 程序打开"（2026-08-28 用户建议"DOC 能不能临时转成 PDF 预览"）。*/
  error?: string;
  onOpenExternally: () => void;
}

export const LegacyOfficePreview: React.FC<LegacyOfficePreviewProps> = ({ path, content, error, onOpenExternally }) => {
  if (error) {
    const fileName = path.split(/[\\/]/).pop() ?? path;
    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          height: "100%",
          gap: 12,
          padding: 24,
          textAlign: "center",
          color: "var(--text-secondary)",
          fontSize: 13,
        }}
      >
        <FileWarning style={{ width: 32, height: 32 }} />
        <div>{fileName} 转换预览失败：{error}</div>
        <button className="btn primary sm" onClick={onOpenExternally}>
          用系统默认程序打开
        </button>
      </div>
    );
  }

  const base64 = content.split(",")[1] ?? "";
  return <PdfPreview base64={base64} />;
};
