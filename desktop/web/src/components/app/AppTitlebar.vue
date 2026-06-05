<template>
  <header class="app-titlebar" data-tauri-drag-region>
    <div v-if="showAgentSelect" class="app-titlebar-leading">
      <label class="agent-select-label">
        <select
          class="agent-select"
          :value="currentAgentId"
          @change="$emit('select-agent', $event.target.value)"
        >
          <option v-for="a in agents" :key="a.id" :value="a.id">{{ a.name }}</option>
        </select>
      </label>
    </div>
    <div class="app-titlebar-spacer" aria-hidden="true" />
    <button
      v-if="showChannelSettings"
      type="button"
      class="titlebar-btn"
      @click="$emit('open-channel-settings')"
    >
      频道设置
    </button>
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
  agents: {
    type: Array,
    default: () => []
  },
  currentAgentId: {
    type: String,
    default: ""
  },
  showAgentSelect: {
    type: Boolean,
    default: false
  },
  showActions: {
    type: Boolean,
    default: true
  },
  showChannelSettings: {
    type: Boolean,
    default: false
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

defineEmits(["reset", "open-channel-settings", "toggle-session-detail", "select-agent"]);
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

.app-titlebar-leading {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
  -webkit-app-region: no-drag;
  app-region: no-drag;
}

.agent-select-label {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0;
}

.agent-select-label__text {
  font-size: 12px;
  color: #6a6a6a;
  white-space: nowrap;
}

.agent-select {
  height: 28px;
  min-width: 140px;
  max-width: min(240px, 40vw);
  padding: 4px 10px;
  border-radius: 6px;
  border: 1px solid #d5d5d9;
  font-size: 12px;
  background: #fff;
  color: #333;
  cursor: pointer;
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
  background: #fff;
  color: #4b4b4b;
  font-size: 12px;
  cursor: pointer;
  -webkit-app-region: no-drag;
  app-region: no-drag;
  flex-shrink: 0;
}

.titlebar-btn:hover {
  background: #f5f5f7;
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
