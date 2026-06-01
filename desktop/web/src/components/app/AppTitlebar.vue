<template>
  <header class="app-titlebar" data-tauri-drag-region>
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
  </header>
</template>

<script setup>
defineProps({
  showActions: {
    type: Boolean,
    default: true
  },
  showChannelSettings: {
    type: Boolean,
    default: false
  }
});

defineEmits(["reset", "open-channel-settings"]);
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
</style>
