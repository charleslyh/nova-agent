import { convertFileSrc } from "@tauri-apps/api/core";
import {
  isLikelyAbsoluteFsPath,
  joinUnderSessionDir,
  pathForConvertFileSrc,
  shouldSkipImgSrcRewrite
} from "@/lib/markdownLocalImages.js";

export function isTauriRuntime() {
  return typeof window !== "undefined" && window.__TAURI_INTERNALS__ != null;
}

/**
 * Resolve a user message image path for <img src> in Tauri (asset protocol).
 */
export function resolveUserImageSrc(rawPath, sessionDir) {
  if (!rawPath || typeof rawPath !== "string") return null;
  const src = rawPath.trim();
  if (!src || shouldSkipImgSrcRewrite(src)) return src;
  if (!isTauriRuntime()) return src;

  let fsPath = isLikelyAbsoluteFsPath(src)
    ? pathForConvertFileSrc(src)
    : joinUnderSessionDir(sessionDir, pathForConvertFileSrc(src));
  if (!fsPath) return null;
  try {
    return convertFileSrc(fsPath);
  } catch {
    return null;
  }
}

export function previewSrcForPickerPath(path) {
  if (!path || !isTauriRuntime()) return null;
  try {
    return convertFileSrc(pathForConvertFileSrc(path));
  } catch {
    return null;
  }
}
