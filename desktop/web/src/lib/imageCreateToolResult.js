/**
 * Parse image_create tool result JSON: { image_url, revised_prompt? }.
 * @param {string | null | undefined} resultText
 * @returns {{ imageUrl: string, revisedPrompt: string | null } | null}
 */
export function parseImageCreateResult(resultText) {
  if (resultText == null) return null;
  const trimmed = String(resultText).trim();
  if (!trimmed) return null;
  try {
    const parsed = JSON.parse(trimmed);
    if (!parsed || typeof parsed !== "object") return null;
    const imageUrl = parsed.image_url;
    if (typeof imageUrl !== "string" || !imageUrl.trim()) return null;
    const revisedPrompt = parsed.revised_prompt;
    return {
      imageUrl: imageUrl.trim(),
      revisedPrompt: typeof revisedPrompt === "string" && revisedPrompt.trim()
        ? revisedPrompt.trim()
        : null
    };
  } catch {
    return null;
  }
}

export function isImageCreateTool(toolName) {
  return toolName === "image_create";
}

export function isToolCallSuccess(status) {
  return status === "success" || status === "completed";
}

export function isToolCallError(status) {
  return status === "error";
}

/** Toolbox `Started` event — tool is executing (after auth if required). */
export function isToolCallRunning(status) {
  return status === "running";
}

/**
 * Show image_create loading placeholder only after toolbox `Started`
 * (user approved when auth was required). Hide while awaiting auth.
 * @param {{ toolName?: string, status?: string, awaitAuthAction?: boolean }} item
 */
export function shouldShowImageCreateLoading(item) {
  if (!isImageCreateTool(item?.toolName)) return false;
  if (item.awaitAuthAction) return false;
  return isToolCallRunning(item.status);
}
