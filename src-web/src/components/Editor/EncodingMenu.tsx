import React, { useEffect, useRef, useState } from "react";
import { fsService } from "../../services/fsService";

interface EncodingMenuProps {
  current: string;
  onReopen: (encoding: string) => void;
  onSaveAs: (encoding: string) => void;
}

/**
 * 编辑器状态栏的编码指示器 + 菜单（参考 VS Code 右下角 "UTF-8" 徽标点击后的
 * "Reopen with Encoding" / "Save with Encoding" 两组动作）。默认按 UTF-8/GBK/UTF-16
 * 自动探测（见 fsops::encoding），这里给用户一个显式覆盖的入口，用来处理探测猜错的情况。
 */
export const EncodingMenu: React.FC<EncodingMenuProps> = ({ current, onReopen, onSaveAs }) => {
  const [open, setOpen] = useState(false);
  const [encodings, setEncodings] = useState<string[]>(["UTF-8", "GBK", "UTF-16LE", "UTF-16BE"]);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    fsService.supportedEncodings().then(setEncodings).catch(() => {});
  }, []);

  useEffect(() => {
    if (!open) return;
    const onClickOutside = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onClickOutside);
    return () => document.removeEventListener("mousedown", onClickOutside);
  }, [open]);

  return (
    <div ref={rootRef} style={{ position: "relative" }}>
      <button className="btn ghost sm" onClick={() => setOpen((v) => !v)} title="文件编码">
        {current || "UTF-8"}
      </button>
      {open && (
        <div className="encoding-menu">
          <div className="encoding-menu-group">重新打开为</div>
          {encodings.map((enc) => (
            <div
              key={`reopen-${enc}`}
              className="encoding-menu-item"
              onClick={() => {
                onReopen(enc);
                setOpen(false);
              }}
            >
              {enc}
            </div>
          ))}
          <div className="encoding-menu-group">保存为</div>
          {encodings.map((enc) => (
            <div
              key={`save-${enc}`}
              className="encoding-menu-item"
              onClick={() => {
                onSaveAs(enc);
                setOpen(false);
              }}
            >
              {enc}
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
