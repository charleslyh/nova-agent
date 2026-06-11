<template>
  <header class="app-titlebar" data-tauri-drag-region>
    <div class="app-titlebar-spacer" aria-hidden="true" />
    <button
      v-if="showActions"
      type="button"
      class="titlebar-btn"
      @click="$emit('reset')"
    >
      重置会话
    </button>
    <button
      v-if="showSessionDetail"
      type="button"
      class="titlebar-icon-btn"
      :class="{ 'titlebar-icon-btn--active': sessionDetailOpen }"
      aria-label="会话详情"
      :aria-expanded="sessionDetailOpen"
      @click="$emit('toggle-session-detail')"
    >
      <svg
        class="titlebar-icon"
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <rect x="3" y="4" width="18" height="16" rx="3" />
        <path d="M14 4v16" />
      </svg>
    </button>
  </header>
</template>

<script setup>
defineProps({
  showActions: {
    type: Boolean,
    default: true
  },
  showSessionDetail: {
    type: Boolean,
    default: false
  },
  sessionDetailOpen: {
    type: Boolean,
    default: false
  }
});

defineEmits(["reset", "toggle-session-detail"]);
</script>

<style scoped>
.app-titlebar {
  height: 44px;
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 12px;
  padding: 0 16px;
  box-sizing: border-box;
  border-bottom: 1px solid rgba(0, 0, 0, 0.06);
  flex-shrink: 0;
  background: #fcfcfc;
  -webkit-app-region: drag;
  app-region: drag;
}

.app-titlebar-spacer {
  flex: 1;
  min-width: 0;
  min-height: 1px;
}

.titlebar-btn {
  border: 1px solid #d5d5d9;
  border-radius: 6px;
  padding: 5px 12px;
  background-color: #fff;
  color: #4b4b4b;
  font-size: 12px;
  cursor: pointer;
  font-family: inherit;
  -webkit-app-region: no-drag;
  app-region: no-drag;
  flex-shrink: 0;
}

.titlebar-btn:hover {
  background-color: #f5f5f7;
}

.titlebar-icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: #5a5a5a;
  cursor: pointer;
  -webkit-app-region: no-drag;
  app-region: no-drag;
  flex-shrink: 0;
}

.titlebar-icon-btn:hover {
  background: #f0f0f2;
  border-color: #e0e0e4;
}

.titlebar-icon-btn--active {
  background: #ebebef;
  border-color: #d0d0d6;
  color: #212121;
}

.titlebar-icon {
  width: 18px;
  height: 18px;
}
</style>
