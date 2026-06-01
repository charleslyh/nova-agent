/**
 * Rewrite markdown <img src> for Tauri: absolute paths, paths under session workspace,
 * and percent-encoded paths before convertFileSrc (avoids double-encoding).
 */

/** Strip stray trailing quotes/whitespace often appended to signed image URLs. */
export function sanitizeImageSrc(src) {
  if (typeof src !== "string") return src;
  let s = src.trim();
  if (!s) return s;
  if (!/^https?:\/\//i.test(s)) return src;
  while (/(?:%22|%27|\"|\'|\s)+$/i.test(s)) {
    s = s.replace(/(?:%22|%27|\"|\'|\s)+$/i, "");
  }
  return s;
}

export function shouldSkipImgSrcRewrite(src) {
  const s = src.trim();
  if (!s) return true;
  const lower = s.toLowerCase();
  if (
    lower.startsWith("http://") ||
    lower.startsWith("https://") ||
    lower.startsWith("data:") ||
    lower.startsWith("blob:")
  ) {
    return true;
  }
  if (lower.startsWith("//")) return true;
  if (lower.startsWith("asset:")) return true;
  if (lower.includes("asset.localhost") || lower.includes("tauri.localhost")) {
    return true;
  }
  return false;
}

export function isLikelyAbsoluteFsPath(src) {
  const t = src.trim();
  if (/^[A-Za-z]:[/\\]/.test(t)) return true;
  if (t.startsWith("/")) return true;
  return false;
}

/** Strip HTML URL encoding before convertFileSrc: otherwise `%` becomes `%25` and breaks loading. */
export function pathForConvertFileSrc(raw) {
  let p = raw.trim().split(/[?#]/)[0] ?? "";
  if (!p) return p;
  const lower = p.toLowerCase();
  if (lower.startsWith("file://")) {
    p = p.replace(/^file:\/\//i, "");
    if (p.startsWith("//")) p = p.slice(1);
  }
  if (/%[0-9A-Fa-f]{2}/.test(p)) {
    try {
      let prev = "";
      let cur = p;
      for (let i = 0; i < 4 && cur !== prev && /%[0-9A-Fa-f]{2}/.test(cur); i++) {
        prev = cur;
        cur = decodeURIComponent(cur);
      }
      p = cur;
    } catch {
      // keep raw if decode is invalid
    }
  }
  return p;
}

/**
 * Join relative image href under session root and reject `..` breakout.
 */
export function joinUnderSessionDir(baseRaw, relRaw) {
  const base = baseRaw?.trim();
  if (!base) return null;
  const noQuery = relRaw.trim().split(/[?#]/)[0] ?? "";
  const trimmed = noQuery.replace(/^\.\//, "").replace(/^\.\\/, "");
  const baseNorm = base.replace(/[/\\]+$/, "");
  const isWindowsDrivePath = /^[A-Za-z]:/.test(baseNorm);
  const sep = isWindowsDrivePath ? "\\" : "/";
  const relNorm = trimmed.replace(/[/\\]/g, sep);
  const relParts = relNorm.split(sep).filter((p) => p.length && p !== ".");

  let baseParts;
  let pathPrefix = "";
  if (isWindowsDrivePath) {
    const drive = baseNorm.slice(0, 2);
    const rest = baseNorm.slice(2).replace(/^[/\\]+/, "");
    baseParts = rest ? rest.split(/[/\\]/).filter(Boolean) : [];
    pathPrefix = `${drive}\\`;
  } else if (baseNorm.startsWith("/")) {
    pathPrefix = "/";
    baseParts = baseNorm.slice(1).split("/").filter(Boolean);
  } else {
    baseParts = baseNorm.split(/[/\\]/).filter(Boolean);
  }

  const stack = [...baseParts];
  for (const part of relParts) {
    if (part === "..") {
      if (stack.length === 0) return null;
      stack.pop();
    } else {
      stack.push(part);
    }
  }
  const out =
    pathPrefix === "/"
      ? `/${stack.join("/")}`
      : pathPrefix.endsWith("\\")
        ? `${pathPrefix}${stack.join("\\")}`
        : stack.join(sep);
  const compareBase = baseNorm.replace(/[/\\]/g, "/").toLowerCase();
  const compareOut = out.replace(/[/\\]/g, "/").toLowerCase();
  const prefix = compareBase.endsWith("/") ? compareBase : `${compareBase}/`;
  if (compareOut === compareBase) return null;
  if (!compareOut.startsWith(prefix)) return null;
  return out;
}

/**
 * Rewrite local <img src> in rendered markdown HTML to Tauri asset URLs via convertFileSrc.
 */
export function sanitizeMarkdownImageSources(html) {
  if (!html.includes("<img") || typeof window === "undefined") return html;
  try {
    const parser = new DOMParser();
    const doc = parser.parseFromString(`<div data-mr-root="1">${html}</div>`, "text/html");
    const root = doc.querySelector("[data-mr-root='1']");
    if (!root) return html;
    for (const img of root.querySelectorAll("img")) {
      const src = img.getAttribute("src");
      if (!src) continue;
      const cleaned = sanitizeImageSrc(src);
      if (cleaned && cleaned !== src) {
        img.setAttribute("src", cleaned);
      }
    }
    return root.innerHTML;
  } catch {
    return html;
  }
}

export function rewriteMarkdownImageSources(html, sessionDir, convertFileSrc) {
  if (!html.includes("<img") || typeof window === "undefined") return html;
  try {
    const parser = new DOMParser();
    const doc = parser.parseFromString(`<div data-mr-root="1">${html}</div>`, "text/html");
    const root = doc.querySelector("[data-mr-root='1']");
    if (!root) return html;
    for (const img of root.querySelectorAll("img")) {
      const src = sanitizeImageSrc(img.getAttribute("src") ?? "");
      if (!src) continue;
      if (src !== img.getAttribute("src")) {
        img.setAttribute("src", src);
      }
      if (shouldSkipImgSrcRewrite(src)) continue;
      let fsPath = isLikelyAbsoluteFsPath(src)
        ? pathForConvertFileSrc(src)
        : joinUnderSessionDir(sessionDir, pathForConvertFileSrc(src));
      if (!fsPath) continue;
      try {
        img.setAttribute("src", convertFileSrc(fsPath));
      } catch {
        // keep original src if convertFileSrc throws
      }
    }
    return root.innerHTML;
  } catch {
    return html;
  }
}
