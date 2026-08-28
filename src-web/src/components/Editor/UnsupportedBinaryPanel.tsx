import React from "react";
import { FileQuestion } from "lucide-react";

interface UnsupportedBinaryPanelProps {
  path: string;
  onOpenExternally: () => void;
}

/** 编辑器/SFTP 查看器打开一个不支持预览的二进制文件（.doc/.xls/.zip/.exe 等）时的
 * 兜底展示（2026-08-28 用户反馈）：不再像以前那样把二进制字节当文本塞进 Monaco
 * 显示一堆乱码，只给"用系统默认程序打开"这一条出路。 */
export const UnsupportedBinaryPanel: React.FC<UnsupportedBinaryPanelProps> = ({ path, onOpenExternally }) => {
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
        color: "var(--text-secondary)",
        fontSize: 13,
      }}
    >
      <FileQuestion style={{ width: 32, height: 32 }} />
      <div>{fileName} 不支持在编辑器中预览</div>
      <button className="btn primary sm" onClick={onOpenExternally}>
        用系统默认程序打开
      </button>
    </div>
  );
};
