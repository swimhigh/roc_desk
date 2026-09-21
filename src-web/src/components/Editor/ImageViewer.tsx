import React, { useCallback, useEffect, useRef, useState } from "react";
import { ZoomIn, ZoomOut, RotateCcw } from "lucide-react";
import { FileText, Copy, LoaderCircle } from "lucide-react";
import { recognizeImage, type OcrResult } from "../../services/ocrService";

interface ImageViewerProps {
  src: string;
  alt?: string;
}

const MIN_SCALE = 0.1;
const MAX_SCALE = 8;
const WHEEL_STEP = 1.15;
const BUTTON_STEP = 1.25;

const clampScale = (s: number) => Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));

/**
 * 图片查看器：默认按容器缩放到完整可见（scale=1 对应"适应窗口"，不是原始像素），
 * 支持鼠标滚轮以光标为中心缩放、工具栏按钮缩放/重置、双击切换缩放，以及放大后
 * 拖拽平移。放大倍数是相对"适应窗口"大小的倍数，不是像素级 100%——
 * 图片查看器场景下"能看清细节"比"精确像素比例"更重要。
 */
export const ImageViewer: React.FC<ImageViewerProps> = ({ src, alt }) => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const [scale, setScale] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const [ocr, setOcr] = useState<OcrResult | null>(null);
  const [ocrBusy, setOcrBusy] = useState(false);
  const [ocrError, setOcrError] = useState<string | null>(null);
  const [imageBox, setImageBox] = useState({ left: 0, top: 0, width: 0, height: 0 });
  const dragStateRef = useRef<{ startX: number; startY: number; origX: number; origY: number } | null>(null);

  useEffect(() => {
    setScale(1);
    setOffset({ x: 0, y: 0 });
    setOcr(null);
    setOcrError(null);
  }, [src]);

  const syncImageBox = useCallback(() => {
    const container = containerRef.current;
    const image = imageRef.current;
    if (!container || !image) return;
    const c = container.getBoundingClientRect();
    const r = image.getBoundingClientRect();
    setImageBox({ left: r.left - c.left, top: r.top - c.top, width: r.width, height: r.height });
  }, []);

  useEffect(() => {
    syncImageBox();
    const observer = new ResizeObserver(syncImageBox);
    if (containerRef.current) observer.observe(containerRef.current);
    if (imageRef.current) observer.observe(imageRef.current);
    return () => observer.disconnect();
  }, [syncImageBox, src, scale, offset]);

  const recognize = async () => {
    const comma = src.indexOf(",");
    if (comma < 0) { setOcrError("图片数据格式不支持 OCR"); return; }
    setOcrBusy(true); setOcrError(null);
    try {
      setOcr(await recognizeImage(src.slice(comma + 1)));
      requestAnimationFrame(syncImageBox);
    } catch (e) {
      setOcrError(String(e));
    } finally { setOcrBusy(false); }
  };

  const copyOcrText = async () => {
    if (ocr?.text) await navigator.clipboard.writeText(ocr.text);
  };

  const zoomAt = useCallback((factor: number, clientX?: number, clientY?: number) => {
    setScale((prev) => {
      const next = clampScale(prev * factor);
      if (next === prev) return prev;
      if (next === 1) {
        setOffset({ x: 0, y: 0 });
        return next;
      }
      const container = containerRef.current;
      if (container && clientX !== undefined && clientY !== undefined) {
        const rect = container.getBoundingClientRect();
        const dx = clientX - rect.left - rect.width / 2;
        const dy = clientY - rect.top - rect.height / 2;
        const ratio = next / prev;
        setOffset((o) => ({
          x: dx - ratio * (dx - o.x),
          y: dy - ratio * (dy - o.y),
        }));
      }
      return next;
    });
  }, []);

  const handleWheel = (e: React.WheelEvent) => {
    e.preventDefault();
    zoomAt(e.deltaY < 0 ? WHEEL_STEP : 1 / WHEEL_STEP, e.clientX, e.clientY);
  };

  const handleMouseDown = (e: React.MouseEvent) => {
    if (scale <= 1) return;
    e.preventDefault();
    dragStateRef.current = { startX: e.clientX, startY: e.clientY, origX: offset.x, origY: offset.y };
    setDragging(true);
  };

  useEffect(() => {
    if (!dragging) return;
    const handleMove = (e: MouseEvent) => {
      const d = dragStateRef.current;
      if (!d) return;
      setOffset({ x: d.origX + (e.clientX - d.startX), y: d.origY + (e.clientY - d.startY) });
    };
    const handleUp = () => {
      dragStateRef.current = null;
      setDragging(false);
    };
    window.addEventListener("mousemove", handleMove);
    window.addEventListener("mouseup", handleUp);
    return () => {
      window.removeEventListener("mousemove", handleMove);
      window.removeEventListener("mouseup", handleUp);
    };
  }, [dragging]);

  const handleDoubleClick = (e: React.MouseEvent) => {
    if (scale !== 1) {
      setScale(1);
      setOffset({ x: 0, y: 0 });
    } else {
      zoomAt(2, e.clientX, e.clientY);
    }
  };

  const reset = () => {
    setScale(1);
    setOffset({ x: 0, y: 0 });
  };

  return (
    <div
      ref={containerRef}
      onWheel={handleWheel}
      style={{
        flex: 1,
        minHeight: 0,
        position: "relative",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        overflow: "hidden",
        background: "var(--bg-base)",
      }}
    >
      <img
        ref={imageRef}
        src={src}
        alt={alt}
        draggable={false}
        onMouseDown={handleMouseDown}
        onDoubleClick={handleDoubleClick}
        style={{
          maxWidth: "100%",
          maxHeight: "100%",
          objectFit: "contain",
          transform: `translate(${offset.x}px, ${offset.y}px) scale(${scale})`,
          transformOrigin: "center center",
          cursor: scale > 1 ? (dragging ? "grabbing" : "grab") : "zoom-in",
          userSelect: "none",
        }}
      />
      {ocr && imageBox.width > 0 && (
        <div style={{ position: "absolute", inset: 0, pointerEvents: "none" }}>
          {ocr.lines.flatMap((line) => line.words).map((word, index) => (
            <span key={`${index}-${word.left}-${word.top}`} style={{
              position: "absolute", left: imageBox.left + word.left / ocr.width * imageBox.width,
              top: imageBox.top + word.top / ocr.height * imageBox.height,
              width: word.width / ocr.width * imageBox.width,
              height: word.height / ocr.height * imageBox.height,
              color: "transparent", cursor: "text", userSelect: "text", pointerEvents: "auto",
              whiteSpace: "nowrap", overflow: "hidden", lineHeight: 1,
            }}>{word.text}</span>
          ))}
        </div>
      )}
      <div
        style={{
          position: "absolute",
          bottom: 12,
          right: 12,
          display: "flex",
          gap: 4,
          alignItems: "center",
          background: "var(--bg-elevated)",
          border: "1px solid var(--border)",
          borderRadius: 6,
          padding: "4px 6px",
        }}
      >
        <button className="btn ghost sm" onClick={() => zoomAt(1 / BUTTON_STEP)} title="缩小">
          <ZoomOut style={{ width: 14, height: 14 }} />
        </button>
        <span style={{ fontSize: 12, color: "var(--text-secondary)", minWidth: 40, textAlign: "center" }}>
          {Math.round(scale * 100)}%
        </span>
        <button className="btn ghost sm" onClick={() => zoomAt(BUTTON_STEP)} title="放大">
          <ZoomIn style={{ width: 14, height: 14 }} />
        </button>
        <button className="btn ghost sm" onClick={reset} title="重置缩放" disabled={scale === 1}>
          <RotateCcw style={{ width: 14, height: 14 }} />
        </button>
        <button className="btn ghost sm" onClick={() => void recognize()} title="使用 Windows OCR 识别图片文字" disabled={ocrBusy}>
          {ocrBusy ? <LoaderCircle className="spin" style={{ width: 14, height: 14 }} /> : <FileText style={{ width: 14, height: 14 }} />} OCR
        </button>
        {ocr && <button className="btn ghost sm" onClick={() => void copyOcrText()} title="复制全部识别文字"><Copy style={{ width: 14, height: 14 }} /> 复制</button>}
      </div>
      {ocrError && <div style={{ position: "absolute", bottom: 52, right: 12, maxWidth: 320, color: "var(--danger)", fontSize: 11, background: "var(--bg-elevated)", padding: "4px 7px", borderRadius: 4 }}>{ocrError}</div>}
    </div>
  );
};
