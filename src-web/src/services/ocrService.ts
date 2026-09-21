import { invoke } from "@tauri-apps/api/core";

export interface OcrWord {
  text: string;
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface OcrLine {
  text: string;
  words: OcrWord[];
}

export interface OcrResult {
  width: number;
  height: number;
  language: string;
  text: string;
  lines: OcrLine[];
}

export function recognizeImage(dataBase64: string): Promise<OcrResult> {
  return invoke("editor_ocr_image", { input: { data_base64: dataBase64 } });
}
