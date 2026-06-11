<template>
  <article class="card">
    <button
      type="button"
      class="header"
      :aria-expanded="expanded ? 'true' : 'false'"
      @click="$emit('toggle')"
    >
      <span class="header-left">
        <span class="kind-icon" title="工具调用" aria-hidden="true">
          <svg viewBox="0 0 24 24" fill="none">
            <path
              d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.106-3.105c.32-.322.863-.22.983.218a6 6 0 0 1-8.259 7.057l-7.91 7.91a1 1 0 0 1-2.999-3l7.91-7.91a6 6 0 0 1 7.057-8.259c.438.12.54.662.219.984z"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
        </span>
        <span class="meta">{{ item.toolName }} · {{ item.callId }}</span>
      </span>
      <span class="header-right">
        <span class="status-icon" :class="statusIconClass(item.status)" :title="statusLabel(item.status)" aria-hidden="true">
          <svg v-if="item.status === 'running'" viewBox="0 0 16 16">
            <circle cx="8" cy="8" r="5.5" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-dasharray="22 14" />
          </svg>
          <svg v-else-if="isToolCallSuccess(item.status)" viewBox="0 0 16 16">
            <circle cx="8" cy="8" r="7" fill="currentColor" />
            <path d="M4.2 8.3l2.3 2.3L11.8 5.3" fill="none" stroke="#ffffff" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
          <svg v-else-if="item.status === 'error'" viewBox="0 0 16 16">
            <circle cx="8" cy="8" r="7" fill="currentColor" />
            <path d="M5.1 5.1l5.8 5.8M10.9 5.1l-5.8 5.8" fill="none" stroke="#ffffff" stroke-width="1.8" stroke-linecap="round" />
          </svg>
          <svg v-else-if="isToolCallCanceled(item.status)" viewBox="0 0 16 16">
            <circle cx="8" cy="8" r="6.2" fill="none" stroke="currentColor" stroke-width="1.6" stroke-dasharray="3.4 2.2" />
            <rect x="6.1" y="6.1" width="3.8" height="3.8" rx="0.5" fill="currentColor" />
          </svg>
          <svg v-else viewBox="0 0 16 16">
            <circle cx="8" cy="8" r="2.2" fill="currentColor" />
          </svg>
        </span>
        <span class="chevron" aria-hidden="true">
          <svg v-if="expanded" viewBox="0 0 16 16">
            <path d="M3.5 10.5L8 6l4.5 4.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
          <svg v-else viewBox="0 0 16 16">
            <path d="M3.5 5.5L8 10l4.5-4.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </span>
      </span>
    </button>

    <p v-if="item.awaitAuthAction && readOnly" class="auth-readonly-hint">
      请在 {{ readOnlyHint }} 中批准或拒绝此工具调用
    </p>
    <div v-else-if="item.awaitAuthAction" class="auth-actions auth-actions--inline">
      <button
        type="button"
        class="auth-button auth-button--deny"
        @click.stop="$emit('deny', item.callId)"
      >
        拒绝
      </button>
      <button
        type="button"
        class="auth-button auth-button--approve"
        @click.stop="$emit('approve', item.callId)"
      >
        同意
      </button>
    </div>

    <div
      v-if="imageCreateMediaMode"
      class="image-preview"
      :class="{ 'image-preview--loading': imageCreateMediaMode === 'loading' }"
    >
      <div
        v-if="imageCreateMediaMode === 'loading'"
        class="image-preview-loading"
        aria-busy="true"
        aria-label="图片生成中"
      >
        <span class="image-preview-spinner" aria-hidden="true" />
        <span class="image-preview-loading-text">生成中…</span>
      </div>
      <img
        v-else
        :src="imageCreateOutput.imageUrl"
        alt="生成的图片"
        loading="lazy"
        decoding="async"
        @error="onImageError"
      />
    </div>

    <div v-if="expanded" class="body">
      <div class="section">
        <div class="label">调用参数</div>
        <pre class="code">{{ formatToolArguments(item.arguments) }}</pre>
      </div>
      <div v-if="!item.awaitAuthAction" class="section section--result">
        <div class="label">调用结果</div>
        <p
          v-if="showCanceledEmptyState"
          class="result-state result-state--canceled"
          role="status"
        >
          工具调用已停止，未产生输出。
        </p>
        <template v-else-if="hasToolResult">
          <p
            v-if="isToolCallCanceled(item.status)"
            class="result-notice result-notice--canceled"
            role="status"
          >
            以下为停止前的部分输出
          </p>
          <pre class="code">{{ formattedResult }}</pre>
        </template>
        <p
          v-else
          class="result-state result-state--waiting"
          role="status"
          :aria-busy="item.status === 'running' ? 'true' : 'false'"
        >
          {{ waitingResultLabel }}
        </p>
      </div>
    </div>
  </article>
</template>

<script setup>
import { computed, ref, watch } from "vue";
import {
  isImageCreateTool,
  isToolCallCanceled,
  isToolCallError,
  isToolCallSuccess,
  parseImageCreateResult,
  shouldShowImageCreateLoading
} from "@/lib/imageCreateToolResult.js";
const props = defineProps({
  item: {
    type: Object,
    required: true
  },
  expanded: {
    type: Boolean,
    required: true
  },
  readOnly: {
    type: Boolean,
    default: false
  },
  readOnlyHint: {
    type: String,
    default: "IM"
  }
});

defineEmits(["toggle", "approve", "deny"]);

const imageLoadFailed = ref(false);

const isImageCreate = computed(() => isImageCreateTool(props.item.toolName));

const imageCreateOutput = computed(() => {
  if (!isImageCreate.value || imageLoadFailed.value) return null;
  return parseImageCreateResult(props.item.result);
});

const imageCreateMediaMode = computed(() => {
  if (!isImageCreate.value) return null;
  if (isToolCallError(props.item.status)) return null;
  if (shouldShowImageCreateLoading(props.item)) return "loading";
  if (isToolCallSuccess(props.item.status) && imageCreateOutput.value) {
    return "image";
  }
  return null;
});

function trimResultRaw(raw) {
  if (raw === null || raw === undefined) return "";
  return String(raw).trim();
}

const hasToolResult = computed(() => trimResultRaw(props.item.result).length > 0);

const showCanceledEmptyState = computed(
  () => isToolCallCanceled(props.item.status) && !hasToolResult.value
);

const waitingResultLabel = computed(() =>
  props.item.status === "running" ? "工具执行中…" : "等待工具返回…"
);

const formattedResult = computed(() => {
  const trimmed = trimResultRaw(props.item.result);
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return trimmed;
  }
});

watch(
  () => props.item.callId,
  () => {
    imageLoadFailed.value = false;
  }
);

watch(
  () => props.item.result,
  () => {
    imageLoadFailed.value = false;
  }
);

function onImageError() {
  imageLoadFailed.value = true;
}

function formatToolArguments(argumentsText) {
  if (argumentsText == null) return "(empty)";
  const trimmed = String(argumentsText).trim();
  if (!trimmed) return "(empty)";
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return trimmed;
  }
}

function statusLabel(status) {
  if (status === "running") return "执行中";
  if (isToolCallSuccess(status)) return "成功";
  if (status === "error") return "失败";
  if (isToolCallCanceled(status)) return "已取消";
  return "等待中";
}

function statusIconClass(status) {
  if (status === "running") return "status-icon--running";
  if (isToolCallSuccess(status)) return "status-icon--success";
  if (status === "error") return "status-icon--error";
  if (isToolCallCanceled(status)) return "status-icon--canceled";
  return "status-icon--pending";
}
</script>

<style scoped>
.card {
  width: min(72%, 760px);
  border: 1px solid #d9d9de;
  border-radius: 8px;
  background: #f7f7f9;
  padding: 10px;
  box-sizing: border-box;
}

.header {
  width: 100%;
  border: 0;
  background: transparent;
  padding: 0;
  margin: 0;
  cursor: pointer;
  text-align: left;
  display: flex;
  justify-content: space-between;
  gap: 10px;
}

.header-left {
  display: flex;
  gap: 8px;
  align-items: center;
  min-width: 0;
}

.kind-icon {
  width: 15px;
  height: 15px;
  display: inline-flex;
  color: #4b5563;
  flex: 0 0 auto;
}

.kind-icon svg {
  width: 100%;
  height: 100%;
}

.header-right {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  color: #5f5f67;
}

.status-icon {
  width: 15px;
  height: 15px;
  display: inline-flex;
}

.status-icon svg {
  width: 100%;
  height: 100%;
}

.status-icon--running {
  color: #3b82f6;
}

.status-icon--running svg {
  animation: status-icon-spin 0.9s linear infinite;
}

@keyframes status-icon-spin {
  to {
    transform: rotate(360deg);
  }
}

.status-icon--success {
  color: #16a34a;
}

.status-icon--error {
  color: #dc2626;
}

.status-icon--pending {
  color: #6b7280;
}

.status-icon--canceled {
  color: #64748b;
}

.chevron {
  width: 14px;
  height: 14px;
  display: inline-flex;
}

.chevron svg {
  width: 100%;
  height: 100%;
}

.meta {
  color: #6c6c73;
  word-break: break-all;
  text-align: right;
}

.section {
  margin-top: 8px;
}

.label {
  font-size: 12px;
  color: #65656b;
  margin-bottom: 4px;
}

.auth-actions {
  display: flex;
  gap: 8px;
  margin-top: 12px;
  justify-content: flex-end;
}

.auth-actions--inline {
  margin-top: 10px;
}

.auth-readonly-hint {
  margin: 10px 0 0;
  font-size: 12px;
  line-height: 1.45;
  color: #6b6b76;
}

.auth-button {
  min-height: 28px;
  min-width: 64px;
  padding: 0 14px;
  border-radius: 9px;
  border: 1px solid transparent;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}

.auth-button--deny {
  background: #dc2626;
  border-color: #dc2626;
  color: #ffffff;
}

.auth-button--approve {
  background: #2563eb;
  border-color: #2563eb;
  color: #ffffff;
}

.body {
  margin-top: 8px;
}

.image-preview {
  margin-top: 8px;
}

.image-preview--loading {
  border: 1px solid #d9d9de;
  border-radius: 8px;
  background: #ececf1;
  overflow: hidden;
}

.image-preview-loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  min-height: 180px;
  padding: 24px 16px;
  box-sizing: border-box;
}

.image-preview-spinner {
  width: 28px;
  height: 28px;
  border: 2.5px solid #d1d5db;
  border-top-color: #3b82f6;
  border-radius: 50%;
  animation: image-preview-spin 0.75s linear infinite;
}

.image-preview-loading-text {
  font-size: 13px;
  color: #6b7280;
}

@keyframes image-preview-spin {
  to {
    transform: rotate(360deg);
  }
}

.image-preview img {
  display: block;
  max-width: 100%;
  height: auto;
  border-radius: 8px;
  border: 1px solid #d9d9de;
  background: #ececf1;
}

.code {
  margin: 0;
  padding: 7px 8px;
  border-radius: 8px;
  background: #ececf1;
  color: #2a2a30;
  font-size: 12px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 220px;
  overflow: auto;
}

.result-state,
.result-notice {
  margin: 0;
  font-size: 12px;
  line-height: 1.45;
}

.result-state--canceled {
  padding: 8px 10px;
  border-radius: 8px;
  border: 1px dashed #c5cad6;
  background: #eef0f4;
  color: #5c6478;
}

.result-state--waiting {
  padding: 8px 10px;
  border-radius: 8px;
  background: #ececf1;
  color: #6b7280;
}

.result-notice--canceled {
  margin-bottom: 6px;
  padding: 6px 10px;
  border-radius: 6px;
  border-left: 3px solid #94a3b8;
  background: #f3f4f6;
  color: #5c6478;
}
</style>
