<template>
  <aside class="app-sidebar">
    <div class="sidebar-block sidebar-block--top" data-tauri-drag-region>
      <button type="button" class="sidebar-entry no-drag" @click="$emit('open-settings')">
        <span class="sidebar-entry__icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" class="sidebar-entry__icon-svg" fill="none" stroke="currentColor" stroke-width="1.75">
            <circle cx="12" cy="12" r="3" />
            <path
              stroke-linecap="round"
              d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41"
            />
          </svg>
        </span>
        <span class="sidebar-entry__label">设置</span>
      </button>
    </div>

    <div class="sidebar-block sidebar-block--conv" data-tauri-drag-region>
      <div
        class="segmented-switch session-list-switch no-drag"
        role="tablist"
        aria-label="会话类型"
      >
        <button
          type="button"
          class="segmented-switch-btn"
          role="tab"
          :aria-selected="sessionListTab === 'normal'"
          :class="{ 'is-active': sessionListTab === 'normal' }"
          @click="sessionListTab = 'normal'"
        >
          会话
        </button>
        <button
          type="button"
          class="segmented-switch-btn"
          role="tab"
          :aria-selected="sessionListTab === 'channel'"
          :class="{ 'is-active': sessionListTab === 'channel' }"
          @click="sessionListTab = 'channel'"
        >
          频道
        </button>
      </div>

      <template v-if="sessionListTab === 'channel'">
        <button type="button" class="sidebar-entry sidebar-entry--new no-drag" @click="$emit('add-channel')">
          <span class="sidebar-entry__icon" aria-hidden="true">
            <svg viewBox="0 0 24 24" class="sidebar-entry__icon-svg" fill="none" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" d="M12 5v14M5 12h14" />
            </svg>
          </span>
          <span class="sidebar-entry__label">新建会话</span>
        </button>
        <div class="session-list no-drag" role="list">
          <SessionRow
            v-for="item in channelSessions"
            :key="item.sessionId"
            :item="item"
            :active="item.sessionId === activeSessionId && !welcomeActive"
            :show-platform="true"
            @select="$emit('select-session', $event)"
            @delete="$emit('delete-session', $event)"
          />
        </div>
      </template>

      <template v-else>
        <button
          type="button"
          class="sidebar-entry sidebar-entry--new no-drag"
          :class="{ 'is-active': welcomeActive }"
          :aria-current="welcomeActive ? 'true' : undefined"
          @click="$emit('new-session')"
        >
          <span class="sidebar-entry__icon" aria-hidden="true">
            <svg viewBox="0 0 24 24" class="sidebar-entry__icon-svg" fill="none" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" d="M12 5v14M5 12h14" />
            </svg>
          </span>
          <span class="sidebar-entry__label">新建会话</span>
        </button>
        <div class="session-list no-drag" role="list">
          <SessionRow
            v-for="item in normalSessions"
            :key="item.sessionId"
            :item="item"
            :active="item.sessionId === activeSessionId && !welcomeActive"
            @select="$emit('select-session', $event)"
            @delete="$emit('delete-session', $event)"
          />
        </div>
      </template>
    </div>
  </aside>
</template>

<script setup>
import { ref, watch } from "vue";
import SessionRow from "@/components/app/SessionRow.vue";

const props = defineProps({
  channelSessions: {
    type: Array,
    default: () => []
  },
  normalSessions: {
    type: Array,
    default: () => []
  },
  activeSessionId: {
    type: String,
    default: null
  },
  welcomeActive: {
    type: Boolean,
    default: false
  }
});

const sessionListTab = ref("normal");

watch(
  () => [props.activeSessionId, props.welcomeActive, props.channelSessions],
  () => {
    if (props.welcomeActive) {
      sessionListTab.value = "normal";
      return;
    }
    const id = props.activeSessionId;
    if (!id) return;
    const isChannel = props.channelSessions.some((s) => s.sessionId === id);
    sessionListTab.value = isChannel ? "channel" : "normal";
  },
  { immediate: true }
);

defineEmits([
  "open-settings",
  "add-channel",
  "select-session",
  "delete-session",
  "new-session"
]);
</script>

<style scoped>
.app-sidebar {
  --sidebar-font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Hiragino Sans GB", "Helvetica Neue", sans-serif;
  --sidebar-entry-font-size: 13px;
  --sidebar-entry-font-weight: 400;
  --sidebar-entry-line-height: 1.4;
  --sidebar-entry-letter-spacing: -0.01em;
  --sidebar-entry-gap: 8px;
  --sidebar-entry-padding: 8px 8px;
  --sidebar-entry-radius: 8px;
  --sidebar-entry-text: #2b2b30;
  --sidebar-entry-icon-size: 16px;
  --sidebar-entry-icon-color: #5c5c66;
  --sidebar-entry-hover-bg: rgba(0, 0, 0, 0.05);
  --sidebar-entry-active-bg: #fcfcfc;
  --sidebar-switch-font-size: 11px;

  font-family: var(--sidebar-font-family);
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;

  width: 200px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  min-height: 0;
  background: #f3f3f5;
  border-right: 1px solid rgba(0, 0, 0, 0.06);
  box-sizing: border-box;
}

.sidebar-block {
  display: flex;
  flex-direction: column;
}

.sidebar-block--top {
  flex-shrink: 0;
  padding: 44px 10px 12px;
}

.sidebar-block--conv {
  flex: 1;
  min-height: 0;
  padding: 0 10px 12px;
  display: flex;
  flex-direction: column;
  overflow-y: auto;
}

.session-list-switch {
  width: 100%;
  margin: 8px 0 10px;
  box-sizing: border-box;
}

.session-list-switch .segmented-switch-btn {
  flex: 1;
}

.segmented-switch {
  display: inline-flex;
  align-items: stretch;
  padding: 1px;
  border-radius: 6px;
  background: #ebebef;
  border: 1px solid #dcdce1;
}

.segmented-switch-btn {
  border: none;
  margin: 0;
  padding: 6px 12px;
  font-family: inherit;
  font-size: var(--sidebar-switch-font-size);
  font-weight: 500;
  line-height: 1.25;
  letter-spacing: 0.02em;
  color: #5c5c66;
  background: transparent;
  border-radius: 5px;
  cursor: pointer;
  white-space: nowrap;
  transition:
    color 0.12s ease,
    background 0.12s ease;
}

.segmented-switch-btn:hover:not(.is-active) {
  color: #333;
}

.segmented-switch-btn.is-active {
  background: #fff;
  color: #1a1a1e;
  font-weight: 600;
}

.segmented-switch-btn + .segmented-switch-btn {
  margin-left: 1px;
}

.session-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-bottom: 4px;
}

/* 仅作用于本组件内的 button，避免 scoped 泄漏覆盖 SessionRow（div） */
button.sidebar-entry {
  display: flex;
  align-items: center;
  gap: var(--sidebar-entry-gap);
  width: 100%;
  box-sizing: border-box;
  padding: var(--sidebar-entry-padding);
  border: 1px solid transparent;
  border-radius: var(--sidebar-entry-radius);
  background: transparent;
  color: var(--sidebar-entry-text);
  font-size: var(--sidebar-entry-font-size);
  font-weight: var(--sidebar-entry-font-weight);
  line-height: var(--sidebar-entry-line-height);
  letter-spacing: var(--sidebar-entry-letter-spacing);
  font-family: inherit;
  text-align: left;
  cursor: pointer;
  transition: background 0.12s ease, border-color 0.12s ease;
}

button.sidebar-entry:hover {
  background: var(--sidebar-entry-hover-bg);
}

button.sidebar-entry--new {
  margin-bottom: 4px;
  border-style: dashed;
  border-color: rgba(0, 0, 0, 0.14);
  color: #4a4a52;
}

button.sidebar-entry--new:hover {
  border-color: rgba(0, 0, 0, 0.22);
  background: var(--sidebar-entry-hover-bg);
  color: var(--sidebar-entry-text);
}

button.sidebar-entry--new.is-active {
  border-color: rgba(0, 0, 0, 0.18);
  background: var(--sidebar-entry-active-bg);
  color: var(--sidebar-entry-text);
}

button.sidebar-entry .sidebar-entry__icon {
  flex-shrink: 0;
  width: var(--sidebar-entry-icon-size);
  height: var(--sidebar-entry-icon-size);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--sidebar-entry-icon-color);
}

button.sidebar-entry .sidebar-entry__icon-svg {
  width: var(--sidebar-entry-icon-size);
  height: var(--sidebar-entry-icon-size);
}

button.sidebar-entry .sidebar-entry__label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
