<template>
  <section
    ref="streamRef"
    class="stream"
    aria-label="消息流"
    @scroll="onScroll"
  >
    <div class="stream-inner">
      <div v-for="(turn, turnIndex) in turnGroups" :key="turn.id" class="turn">
        <div class="turn-content">
          <div v-if="turn.user" class="msg-row msg-row--user">
            <div class="user-message">
              <div
                v-if="hasUserText(turn.user)"
                class="bubble bubble--user"
              >
                {{ turn.user.text }}
              </div>
              <div
                v-if="userImageResources(turn.user, props.sessionDir).length"
                class="user-images"
              >
                <img
                  v-for="(img, imgIndex) in userImageResources(turn.user, props.sessionDir)"
                  :key="`${turn.user.id}-img-${imgIndex}`"
                  class="user-images__img"
                  :src="img.src"
                  alt=""
                  loading="lazy"
                />
              </div>
            </div>
          </div>

          <template v-for="item in turn.items" :key="item.id">
            <div
              v-if="shouldShowStreamItem(item)"
              class="msg-row"
              :class="`msg-row--${item.role === 'tool' || item.role === 'think' || item.role === 'error' ? 'assistant' : item.role}`"
            >
              <div class="msg-row-body">
                <span
                  v-if="isAssistantSideItem(item) && agentBadgeLabel(item)"
                  class="agent-badge"
                  :class="{ 'agent-badge--sub': item.agentRole === 'sub' }"
                  :title="item.agentId"
                >{{ agentBadgeLabel(item) }}</span>
                <ToolCallCard
                  v-if="item.role === 'tool'"
                  :item="item"
                  :expanded="isExpanded(item.id)"
                  :read-only="readOnly"
                  :read-only-hint="readOnlyHint"
                  @toggle="toggleToolCard(item.id)"
                  @approve="handleToolAuthApprove"
                  @deny="handleToolAuthDeny"
                />
                <ThinkCard
                  v-else-if="item.role === 'think'"
                  :item="item"
                  :expanded="isThinkExpanded(item.id)"
                  @toggle="toggleThinkCard(item.id)"
                />
                <div
                  v-else-if="item.role === 'error'"
                  class="system-error-line"
                  role="alert"
                  aria-live="polite"
                >
                  <span class="system-error-line__label">失败</span>
                  <span class="system-error-line__content">
                    {{ item.text }}
                  </span>
                </div>
                <div
                  v-else-if="hasVisibleAssistantText(item.text)"
                  class="text text--assistant markdown-body"
                  v-html="renderAssistantMarkdown(item.text, { sessionDir: props.sessionDir })"
                />
              </div>
            </div>
          </template>
        </div>

        <div v-if="showTurnPending(turnIndex)" class="turn-progress" aria-label="生成中">
          <span class="turn-progress-dot" />
          <span class="turn-progress-dot" />
          <span class="turn-progress-dot" />
        </div>
      </div>
    </div>
  </section>
</template>

<script setup>
import { computed, nextTick, ref, watch } from "vue";
import { renderAssistantMarkdown } from "@/lib/markdown.js";
import { resolveUserImageSrc } from "@/lib/userImages.js";
import ToolCallCard from "@/components/chat/ToolCallCard.vue";
import ThinkCard from "@/components/chat/ThinkCard.vue";

const BOTTOM_THRESHOLD = 20;

const props = defineProps({
  /** 会话消息序列（用户 / 助手 / 工具等），按时间顺序排列 */
  messages: {
    type: Array,
    required: true
  },
  status: {
    type: String,
    required: true
  },
  /** Session workspace on disk (for resolving relative markdown image paths). */
  sessionDir: {
    type: String,
    default: null
  },
  readOnly: {
    type: Boolean,
    default: false
  },
  readOnlyHint: {
    type: String,
    default: "IM"
  },
  agents: {
    type: Array,
    default: () => []
  }
});
const emit = defineEmits(["tool-auth-approve", "tool-auth-deny"]);

const streamRef = ref(null);
const autoStickToBottom = ref(true);
const lastScrollMode = ref("auto");
const expandedToolCardIds = ref(new Set());
const expandedThinkCardIds = ref(new Set());
const manualPinnedThinkId = ref(null);
const lastActiveStreamingThinkId = ref(null);
function hasVisibleAssistantText(text) {
  return typeof text === "string" && text.trim().length > 0;
}

function isAssistantSideItem(item) {
  return item?.role === "assistant" || item?.role === "tool" || item?.role === "think";
}

function agentBadgeLabel(item) {
  if (!item?.agentId) return null;
  const agent = props.agents.find((a) => a.id === item.agentId);
  return agent?.name || item.agentId;
}

function hasUserText(user) {
  return user && hasVisibleAssistantText(user.text);
}

function userImageResources(user, sessionDir) {
  if (!user || !Array.isArray(user.resources)) return [];
  const dir = typeof sessionDir === "string" ? sessionDir.trim() : "";
  return user.resources
    .filter((r) => r?.kind === "image" && typeof r.path === "string")
    .map((r) => {
      const src = resolveUserImageSrc(r.path, dir);
      return src ? { path: r.path, src } : null;
    })
    .filter(Boolean);
}

function shouldShowStreamItem(item) {
  if (!item || typeof item.role !== "string") {
    return false;
  }
  if (item.role === "assistant" && item.agentRole === "sub") {
    return false;
  }
  if (item.role === "tool" || item.role === "think") {
    return true;
  }
  if (item.role === "error") {
    return hasVisibleAssistantText(item.text);
  }
  return hasVisibleAssistantText(item.text);
}

const turnGroups = computed(() => {
  const groups = [];
  let currentTurn = null;

  for (const item of props.messages) {
    if (!item || typeof item.role !== "string") {
      continue;
    }
    if (item.role === "user") {
      currentTurn = {
        id: item.id,
        user: item,
        items: []
      };
      groups.push(currentTurn);
      continue;
    }

    if (!currentTurn) {
      currentTurn = {
        id: `orphan-${item.id}`,
        user: null,
        items: []
      };
      groups.push(currentTurn);
    }

    currentTurn.items.push(item);
  }

  return groups;
});

function isNearBottom(element) {
  return element.scrollHeight - element.scrollTop - element.clientHeight <= BOTTOM_THRESHOLD;
}

function onScroll() {
  const element = streamRef.value;
  if (!element) return;
  autoStickToBottom.value = isNearBottom(element);
}

function isExpanded(id) {
  return expandedToolCardIds.value.has(id);
}

function toggleToolCard(id) {
  const next = new Set(expandedToolCardIds.value);
  if (next.has(id)) {
    next.delete(id);
  } else {
    next.add(id);
  }
  expandedToolCardIds.value = next;
}

function handleToolAuthApprove(callId) {
  if (props.readOnly) return;
  emit("tool-auth-approve", callId);
}

function handleToolAuthDeny(callId) {
  if (props.readOnly) return;
  emit("tool-auth-deny", callId);
}

function isThinkExpanded(id) {
  return expandedThinkCardIds.value.has(id);
}

function toggleThinkCard(id) {
  const currentlyExpanded = expandedThinkCardIds.value.has(id);
  if (currentlyExpanded) {
    expandedThinkCardIds.value = new Set();
    if (manualPinnedThinkId.value === id) {
      manualPinnedThinkId.value = null;
    }
    return;
  }
  expandedThinkCardIds.value = new Set([id]);
  manualPinnedThinkId.value = id;
}

function showTurnPending(turnIndex) {
  const isLastTurn = turnIndex === turnGroups.value.length - 1;
  return isLastTurn && props.status === "running";
}

watch(
  () =>
    props.messages
      .filter((item) => item && typeof item.role === "string")
      .map((item) => `${item.id}:${item.role}:${item.done ? "done" : "open"}:${item.expanded ? "expanded" : "collapsed"}`)
      .join("|"),
  () => {
    const thinkItems = props.messages.filter(
      (item) => item && typeof item.role === "string" && item.role === "think"
    );
    if (thinkItems.length === 0) {
      expandedThinkCardIds.value = new Set();
      manualPinnedThinkId.value = null;
      lastActiveStreamingThinkId.value = null;
      return;
    }

    const activeStreaming = [...thinkItems].reverse().find((it) => !it.done) ?? null;
    const activeStreamingId = activeStreaming?.id ?? null;
    if (
      activeStreamingId &&
      activeStreamingId !== lastActiveStreamingThinkId.value
    ) {
      // A new think stream started, so we release user pin and follow latest by default.
      manualPinnedThinkId.value = null;
    }
    lastActiveStreamingThinkId.value = activeStreamingId;

    if (manualPinnedThinkId.value) {
      const pinned = thinkItems.find((it) => it.id === manualPinnedThinkId.value);
      if (pinned) {
        expandedThinkCardIds.value = new Set([pinned.id]);
        return;
      }
      manualPinnedThinkId.value = null;
    }

    if (activeStreaming) {
      expandedThinkCardIds.value = new Set([activeStreaming.id]);
    } else {
      // Auto mode keeps completed think cards folded.
      expandedThinkCardIds.value = new Set();
    }
  },
  { immediate: true }
);

watch(
  () => {
    const last = props.messages[props.messages.length - 1];
    const length = props.messages.length;
    const kind = length === 1 ? "first-item" : (last?.id ?? "");
    const lastTextLength = (last?.text ?? "").length;
    return `${length}:${kind}:${lastTextLength}`;
  },
  async () => {
    const element = streamRef.value;
    if (!element) return;
    const contentFitsViewport = element.scrollHeight <= element.clientHeight;
    const shouldStick = autoStickToBottom.value || isNearBottom(element) || contentFitsViewport;
    if (!shouldStick) return;
    const isFirstMessage = props.messages.length === 1;
    const shouldSmooth = isFirstMessage && lastScrollMode.value !== "smooth";
    await nextTick();
    element.scrollTo({
      top: element.scrollHeight,
      behavior: shouldSmooth ? "smooth" : "auto"
    });
    lastScrollMode.value = shouldSmooth ? "smooth" : "auto";
    autoStickToBottom.value = true;
  }
);
</script>

<style scoped>
.stream {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  padding-top: 2px;
  /* 滚动条贴 chat-main 右缘；消息宽度由 .stream-inner 与 composer 的 content-lane 对齐 */
  scrollbar-gutter: stable;
}

.stream-inner {
  --agent-badge-gutter: 96px;
  width: min(100%, 920px);
  margin: 0 auto;
  padding: 0 16px 0 calc(16px + var(--agent-badge-gutter));
  box-sizing: border-box;
}

.stream::-webkit-scrollbar {
  width: 10px;
}

.stream::-webkit-scrollbar-track {
  background: transparent;
}

.stream::-webkit-scrollbar-thumb {
  background: rgba(120, 120, 125, 0.42);
  border-radius: 999px;
  border: 2px solid transparent;
  background-clip: content-box;
}

.stream::-webkit-scrollbar-thumb:hover {
  background: rgba(95, 95, 100, 0.58);
  background-clip: content-box;
}

.stream {
  scrollbar-width: thin;
  scrollbar-color: rgba(120, 120, 125, 0.42) transparent;
}

.msg-row {
  display: flex;
  width: 100%;
  margin-bottom: 12px;
}

.turn {
  margin-bottom: 6px;
}

.msg-row--user {
  justify-content: flex-end;
}

.msg-row--assistant {
  justify-content: flex-start;
}

.msg-row-body {
  position: relative;
  width: min(72%, 760px);
  max-width: 100%;
  min-width: 0;
}

.msg-row-body > .agent-badge {
  position: absolute;
  top: 6px;
  right: calc(100% + 8px);
  max-width: var(--agent-badge-gutter, 96px);
  padding: 2px 8px;
  border-radius: 4px;
  font-size: 11px;
  line-height: 1.4;
  font-weight: 600;
  color: #5c5c62;
  background: #f0f0f2;
  border: 1px solid #e4e4e8;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  pointer-events: none;
}

.turn-content {
  display: flex;
  flex-direction: column;
  margin-left: calc(-1 * var(--agent-badge-gutter));
}

.agent-badge--sub {
  color: #6b4f1d;
  background: #fff6e8;
  border-color: #f0ddb8;
}

.bubble {
  font-size: 15px;
  line-height: 1.5;
  padding: 10px 14px;
  border-radius: 14px;
  box-sizing: border-box;
}

.user-message {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 8px;
  width: min(72%, 760px);
  max-width: 100%;
  min-width: 0;
  margin-left: auto;
}

.bubble--user {
  width: fit-content;
  max-width: 100%;
  background: #ececed;
  color: #2d2d2d;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  word-break: break-word;
}

.user-images {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 8px;
  width: 100%;
}

.user-images__img {
  display: block;
  max-width: 256px;
  max-height: 256px;
  width: auto;
  height: auto;
  object-fit: contain;
  border-radius: 12px;
  border: 1px solid #e0e0e4;
}

.text {
  width: 100%;
  min-width: 0;
  font-size: 15px;
  line-height: 1.6;
}

.msg-row-body :deep(.card),
.msg-row-body :deep(.think-card) {
  width: 100%;
}

.text--assistant {
  padding-top: 4px;
}

.system-error-line {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 8px;
  background: #f7f7f8;
  border: 1px solid #e7e7ea;
}

.system-error-line__label {
  flex: 0 0 auto;
  color: #b42318;
  font-size: 12px;
  font-weight: 600;
  line-height: 1.4;
}

.system-error-line__content {
  color: #5f5f67;
  font-size: 13px;
  line-height: 1.5;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.turn-progress {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 0 0 10px 2px;
}

.turn-progress-dot {
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background: #9b9ba1;
  opacity: 0.3;
  animation: turn-progress-dot 1.2s infinite ease-in-out;
}

.turn-progress-dot:nth-child(2) {
  animation-delay: 0.2s;
}

.turn-progress-dot:nth-child(3) {
  animation-delay: 0.4s;
}

@keyframes turn-progress-dot {
  0%,
  80%,
  100% {
    opacity: 0.3;
    transform: translateY(0);
  }
  40% {
    opacity: 0.9;
    transform: translateY(-1px);
  }
}

.markdown-body :deep(p) {
  margin: 0 0 0.6em;
}

.markdown-body :deep(p:last-child) {
  margin-bottom: 0;
}

.markdown-body :deep(img) {
  display: block;
  max-width: 100%;
  height: auto;
  margin: 0.5em 0;
  border-radius: 8px;
}

.markdown-body :deep(code) {
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
  background: rgba(0, 0, 0, 0.06);
  border-radius: 6px;
  padding: 0.08em 0.35em;
}

.markdown-body :deep(pre code) {
  display: block;
  padding: 0;
  margin: 0;
  border-radius: 0;
  background: transparent;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
}

.markdown-body :deep(pre code:not(.hljs)) {
  padding: 12px 14px;
  font-size: 13px;
  line-height: 1.55;
  overflow-x: auto;
  color: #24292e;
  background: #f6f8fa;
}

.markdown-body :deep(pre) {
  margin: 0 0 0.7em;
  padding: 0;
  overflow: hidden;
  border-radius: 10px;
  border: 1px solid rgba(27, 31, 36, 0.12);
  background: transparent;
}

.markdown-body :deep(pre code.hljs) {
  padding: 1em;
  font-size: 13px;
  line-height: 1.55;
}

.markdown-body :deep(section) {
  margin: 0;
  padding: 0;
}

.markdown-body :deep(.katex-display) {
  display: block;
  margin: 0.55em 0;
  max-width: 100%;
  padding-block: 0.25em;
}
</style>
