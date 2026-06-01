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
      <p class="session-section-title no-drag">频道会话</p>
      <button type="button" class="sidebar-entry sidebar-entry--new no-drag" @click="$emit('add-channel')">
        <span class="sidebar-entry__icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" class="sidebar-entry__icon-svg" fill="none" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" d="M12 5v14M5 12h14" />
          </svg>
        </span>
        <span class="sidebar-entry__label">频道</span>
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

      <p class="session-section-title no-drag">会话</p>
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
    </div>
  </aside>
</template>

<script setup>
import SessionRow from "@/components/app/SessionRow.vue";

defineProps({
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
  width: 200px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  min-height: 0;
  background: #f0f0f2;
  border-right: 1px solid rgba(0, 0, 0, 0.08);
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

.session-section-title {
  margin: 10px 4px 6px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: #888;
}

.session-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-bottom: 4px;
}

.sidebar-entry {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  box-sizing: border-box;
  padding: 10px 12px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: #333;
  font-size: 14px;
  font-weight: 500;
  line-height: 1.2;
  text-align: left;
  cursor: pointer;
  transition: background 0.12s ease, border-color 0.12s ease;
}

button.sidebar-entry {
  font-family: inherit;
}

.sidebar-entry--new {
  margin-bottom: 6px;
  border-style: dashed;
  border-color: rgba(0, 0, 0, 0.22);
}

.sidebar-entry--new:hover {
  border-color: rgba(0, 0, 0, 0.32);
  background: rgba(255, 255, 255, 0.45);
}

.sidebar-entry--new.is-active {
  border-color: rgba(0, 0, 0, 0.28);
  background: rgba(255, 255, 255, 0.55);
  font-weight: 600;
}

.sidebar-entry:hover {
  background: rgba(0, 0, 0, 0.05);
}

.sidebar-entry.is-active:not(.sidebar-entry--new) {
  background: rgba(255, 255, 255, 0.55);
  font-weight: 600;
}

.sidebar-entry__icon {
  flex-shrink: 0;
  width: 18px;
  height: 18px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #4a4a52;
}

.sidebar-entry__icon-svg {
  width: 18px;
  height: 18px;
}

.sidebar-entry__label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
