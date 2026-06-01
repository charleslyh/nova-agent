<template>
  <footer class="composer-shell">
    <form class="composer" @submit.prevent="onFormSubmit">
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
        <select
          v-if="agents.length"
          class="agent-select"
          :value="currentAgentId"
          @change="$emit('select-agent', $event.target.value)"
        >
          <option v-for="a in agents" :key="a.id" :value="a.id">{{ a.name }}</option>
        </select>
        <span v-else class="agent-spacer" />
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

const emit = defineEmits(["submit", "cancel", "update:draft", "select-agent"]);

const props = defineProps({
  draft: {
    type: String,
    required: true
  },
  status: {
    type: String,
    required: true
  },
  agents: {
    type: Array,
    default: () => []
  },
  currentAgentId: {
    type: String,
    default: ""
  }
});

const isRunning = computed(() => props.status === "running");

const primaryDisabled = computed(() => {
  if (isRunning.value) return false;
  return !props.draft.trim();
});

const buttonState = computed(() => {
  if (isRunning.value) return "running";
  return primaryDisabled.value ? "disabled" : "enabled";
});

const primaryLabel = computed(() => (isRunning.value ? "停止" : "发送"));

const imeComposing = ref(false);

function onCompositionStart() {
  imeComposing.value = true;
}

function onCompositionEnd() {
  imeComposing.value = false;
}

function onPrimaryClick() {
  if (isRunning.value) {
    emit("cancel");
    return;
  }
  if (!props.draft.trim()) return;
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

.composer {
  background: #fff;
  border: 1px solid #d9d9dd;
  border-radius: 16px;
  padding: 14px 16px;
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

.agent-select {
  height: 32px;
  min-width: 140px;
  max-width: 55%;
  padding: 6px 10px;
  border-radius: 8px;
  border: 1px solid #cfcfd4;
  font-size: 13px;
  background: #fff;
  color: #333;
}

.agent-spacer {
  flex: 1;
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
