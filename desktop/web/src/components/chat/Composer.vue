<template>
  <footer class="composer-shell">
    <p v-if="pickError" class="composer-error" role="alert">{{ pickError }}</p>
    <form class="composer" @submit.prevent="onFormSubmit">
      <div v-if="attachments.length" class="composer-attachments" aria-label="待发送图片">
        <div
          v-for="item in attachments"
          :key="item.id"
          class="composer-attachment"
        >
          <img
            v-if="item.previewUrl"
            class="composer-attachment__img"
            :src="item.previewUrl"
            alt=""
          />
          <span v-else class="composer-attachment__placeholder" aria-hidden="true">图</span>
          <button
            type="button"
            class="composer-attachment__remove"
            aria-label="移除图片"
            title="移除"
            @click="$emit('remove-attachment', item.id)"
          >
            ×
          </button>
        </div>
      </div>
      <textarea
        :value="draft"
        class="composer-input"
        placeholder="在这里输入消息 (↩发送, Shift+↩换行)"
        rows="2"
        @input="$emit('update:draft', $event.target.value)"
        @compositionstart="onCompositionStart"
        @compositionend="onCompositionEnd"
        @keydown.enter.exact="onEnter"
      />
      <div class="composer-actions">
        <button
          v-if="showAttachButton"
          type="button"
          class="attach-btn"
          :disabled="isRunning || pickingImages || attachments.length >= MAX_COMPOSER_IMAGE_ATTACHMENTS"
          :title="attachButtonTitle"
          :aria-label="attachButtonTitle"
          @click="onPickImages"
        >
          +
        </button>
        <span v-else class="composer-actions-spacer" />
        <button
          type="button"
          class="send-btn"
          :class="`send-btn--${buttonState}`"
          :disabled="primaryDisabled"
          :aria-label="primaryLabel"
          :title="primaryLabel"
          @click="onPrimaryClick"
        >
          <svg
            v-if="!isRunning"
            class="send-btn-icon"
            viewBox="0 0 24 24"
            aria-hidden="true"
          >
            <path d="M12 5L6.5 10.5L8.12 12.12L10.86 9.38V19H13.14V9.38L15.88 12.12L17.5 10.5L12 5Z" />
          </svg>
          <svg
            v-else
            class="send-btn-icon"
            viewBox="0 0 24 24"
            aria-hidden="true"
          >
            <rect x="8" y="8" width="8" height="8" rx="1" ry="1" />
          </svg>
        </button>
      </div>
    </form>
  </footer>
</template>

<script setup>
import { computed, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { MAX_COMPOSER_IMAGE_ATTACHMENTS } from "@/composables/useChatSession.js";
import { isTauriRuntime } from "@/lib/userImages.js";

const emit = defineEmits([
  "submit",
  "cancel",
  "update:draft",
  "add-attachments",
  "remove-attachment"
]);

const props = defineProps({
  draft: {
    type: String,
    required: true
  },
  status: {
    type: String,
    required: true
  },
  attachments: {
    type: Array,
    default: () => []
  }
});

const isRunning = computed(() => props.status === "running");
const showAttachButton = computed(() => isTauriRuntime());
const attachAtLimit = computed(
  () => props.attachments.length >= MAX_COMPOSER_IMAGE_ATTACHMENTS
);
const attachButtonTitle = computed(() =>
  attachAtLimit.value
    ? `最多添加 ${MAX_COMPOSER_IMAGE_ATTACHMENTS} 张图片`
    : "添加图片"
);
const hasSendableContent = computed(
  () => !!props.draft.trim() || props.attachments.length > 0
);

const primaryDisabled = computed(() => {
  if (isRunning.value) return false;
  return !hasSendableContent.value;
});

const buttonState = computed(() => {
  if (isRunning.value) return "running";
  return primaryDisabled.value ? "disabled" : "enabled";
});

const primaryLabel = computed(() => (isRunning.value ? "停止" : "发送"));

const imeComposing = ref(false);
const pickError = ref("");
const pickingImages = ref(false);

const IMAGE_FILTER = {
  name: "Images",
  extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"]
};

function onCompositionStart() {
  imeComposing.value = true;
}

function onCompositionEnd() {
  imeComposing.value = false;
}

async function onPickImages() {
  pickError.value = "";
  if (pickingImages.value) return;
  if (isRunning.value) {
    pickError.value = "会话运行中，暂无法添加图片";
    return;
  }
  if (attachAtLimit.value) {
    pickError.value = attachButtonTitle.value;
    return;
  }
  pickingImages.value = true;
  try {
    const selected = await open({
      multiple: true,
      filters: [IMAGE_FILTER]
    });
    if (selected == null) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    const normalized = paths.filter((p) => typeof p === "string" && p.length > 0);
    if (!normalized.length) {
      pickError.value = "未选择到有效的图片路径";
      return;
    }
    emit("add-attachments", normalized);
  } catch (error) {
    pickError.value = error?.message || "打开图片选择器失败";
    console.error("image picker failed", error);
  } finally {
    pickingImages.value = false;
  }
}

function onPrimaryClick() {
  if (isRunning.value) {
    emit("cancel");
    return;
  }
  if (!hasSendableContent.value) return;
  emit("submit");
}

function onFormSubmit() {
  onPrimaryClick();
}

function onEnter(event) {
  const imeActive = event.isComposing || imeComposing.value || event.keyCode === 229;

  if (imeActive) return;
  if (!isRunning.value && primaryDisabled.value) return;

  event.preventDefault();
  onPrimaryClick();
}
</script>

<style scoped>
.composer-shell {
  margin-top: 8px;
}

.composer-error {
  margin: 0 0 8px;
  font-size: 13px;
  color: #b42318;
}

.composer {
  background: #fff;
  border: 1px solid #d9d9dd;
  border-radius: 16px;
  padding: 14px 16px;
}

.composer-attachments {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 10px;
}

.composer-attachment {
  position: relative;
  width: 72px;
  height: 72px;
  border-radius: 10px;
  overflow: hidden;
  border: 1px solid #e0e0e4;
  background: #f5f5f7;
}

.composer-attachment__img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.composer-attachment__placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  font-size: 13px;
  color: #888;
}

.composer-attachment__remove {
  position: absolute;
  top: 4px;
  right: 4px;
  width: 20px;
  height: 20px;
  border: none;
  border-radius: 999px;
  background: rgba(0, 0, 0, 0.55);
  color: #fff;
  font-size: 14px;
  line-height: 1;
  cursor: pointer;
  padding: 0;
}

.composer-input {
  width: 100%;
  resize: none;
  border: none;
  outline: none;
  background: transparent;
  color: #575757;
  font-size: 15px;
  line-height: 1.5;
  min-height: 48px;
}

.composer-actions {
  margin-top: 20px;
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.composer-actions-spacer {
  flex: 1;
  min-width: 0;
}

.attach-btn {
  flex: 0 0 auto;
  width: 32px;
  height: 32px;
  border-radius: 8px;
  border: 1px solid #cfcfd4;
  background: #fff;
  color: #333;
  font-size: 20px;
  line-height: 1;
  cursor: pointer;
  padding: 0;
}

.attach-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.send-btn {
  border-radius: 999px;
  width: 36px;
  height: 36px;
  border: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  color: #fff;
  transition: background-color 0.15s ease;
  cursor: pointer;
  flex: 0 0 auto;
}

.send-btn-icon {
  width: 28px;
  height: 28px;
  fill: currentColor;
  pointer-events: none;
}

.send-btn--enabled {
  background: #1976ff;
}

.send-btn--disabled {
  background: #a6aab3;
  cursor: not-allowed;
}

.send-btn--running {
  background: #000;
}

.send-btn:disabled {
  cursor: not-allowed;
}
</style>
