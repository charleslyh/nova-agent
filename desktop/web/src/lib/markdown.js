import { convertFileSrc } from "@tauri-apps/api/core";
import MarkdownIt from "markdown-it";
import multimdTable from "markdown-it-multimd-table";
import { escapeHtml } from "markdown-it/lib/common/utils.mjs";
import texmath from "markdown-it-texmath";
import katex from "katex";
import hljs from "highlight.js";
import {
  rewriteMarkdownImageSources,
  sanitizeMarkdownImageSources
} from "@/lib/markdownLocalImages.js";

function isTauriRuntime() {
  return typeof window !== "undefined" && window.__TAURI_INTERNALS__ != null;
}

function highlightCode(str, lang) {
  const name = (lang || "").trim();
  if (name && hljs.getLanguage(name)) {
    try {
      return (
        `<pre><code class="hljs language-${escapeHtml(name)}">` +
        hljs.highlight(str, { language: name, ignoreIllegals: true }).value +
        "</code></pre>"
      );
    } catch {
      /* fall through */
    }
  }
  return `<pre><code class="hljs">${escapeHtml(str)}</code></pre>`;
}

function createMarkdownRenderer({ texmath: enableTexmath = false } = {}) {
  const md = new MarkdownIt({
    html: false,
    linkify: true,
    breaks: true,
    highlight: highlightCode
  }).use(multimdTable, {
    multiline: true,
    rowspan: true,
    headerless: true
  });

  if (enableTexmath) {
    md.use(texmath, {
      engine: katex,
      delimiters: "dollars",
      katexOptions: { throwOnError: false }
    });
  }

  return md;
}

const assistantMd = createMarkdownRenderer({ texmath: true });
const skillMd = createMarkdownRenderer({ texmath: false });

export function renderAssistantMarkdown(text, options = {}) {
  let html = assistantMd.render(text ?? "");
  if (html.includes("<img")) {
    html = sanitizeMarkdownImageSources(html);
  }
  if (!isTauriRuntime() || !html.includes("<img")) {
    return html;
  }
  const sessionDir =
    typeof options.sessionDir === "string" ? options.sessionDir.trim() : "";
  try {
    return rewriteMarkdownImageSources(html, sessionDir, convertFileSrc);
  } catch {
    return html;
  }
}

export function renderSkillMarkdown(text) {
  return skillMd.render(text ?? "");
}
