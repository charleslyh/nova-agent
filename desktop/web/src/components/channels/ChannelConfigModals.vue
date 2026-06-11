<template>
  <div class="channel-config-modals">
    <QQConfigDialog
      v-if="editing?.isNew && editing?.type === 'qq'"
      :channel-id="editing.channelId"
      :session-id="editing.sessionId ?? ''"
      :is-new="editing.isNew"
      :get-channel-config="getChannelConfig"
      :save-channel-config="saveChannelConfig"
      :create-channel="createChannel"
      :get-settings-catalog="getSettingsCatalog"
      :get-session-agent="getSessionAgent"
      :set-session-agent="setSessionAgent"
      @close="$emit('close-edit')"
      @saved="$emit('saved', $event)"
    />
    <WeComConfigDialog
      v-if="editing?.isNew && editing?.type === 'wecom'"
      :channel-id="editing.channelId"
      :session-id="editing.sessionId ?? ''"
      :is-new="editing.isNew"
      :get-channel-config="getChannelConfig"
      :save-channel-config="saveChannelConfig"
      :create-channel="createChannel"
      :get-settings-catalog="getSettingsCatalog"
      :get-session-agent="getSessionAgent"
      :set-session-agent="setSessionAgent"
      @close="$emit('close-edit')"
      @saved="$emit('saved', $event)"
    />

    <div
      v-if="deleteTarget"
      class="confirm-overlay"
      role="alertdialog"
      aria-labelledby="channel-delete-title"
      aria-describedby="channel-delete-desc"
      @click.stop
    >
      <div class="confirm-panel">
        <p id="channel-delete-title" class="confirm-title">删除频道</p>
        <p id="channel-delete-desc" class="confirm-desc">
          确定删除频道「{{ deleteTarget.name }}」（{{ deleteTarget.platformName }}）？此操作会移除该频道的会话与配置，且不可恢复。
        </p>
        <p v-if="deleteError" class="confirm-error">{{ deleteError }}</p>
        <div class="confirm-actions">
          <button
            type="button"
            class="confirm-btn confirm-btn--ghost"
            :disabled="deleting"
            @click="$emit('cancel-delete')"
          >
            取消
          </button>
          <button
            type="button"
            class="confirm-btn confirm-btn--danger"
            :disabled="deleting"
            @click="$emit('confirm-delete')"
          >
            {{ deleting ? "删除中…" : "删除" }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import QQConfigDialog from "@/components/settings/QQConfigDialog.vue";
import WeComConfigDialog from "@/components/settings/WeComConfigDialog.vue";

defineProps({
  editing: { type: Object, default: null },
  deleteTarget: { type: Object, default: null },
  deleting: { type: Boolean, default: false },
  deleteError: { type: String, default: "" },
  getChannelConfig: { type: Function, required: true },
  saveChannelConfig: { type: Function, required: true },
  createChannel: { type: Function, required: true },
  getSettingsCatalog: { type: Function, required: true },
  getSessionAgent: { type: Function, required: true },
  setSessionAgent: { type: Function, required: true }
});

defineEmits([
  "close-edit",
  "saved",
  "cancel-delete",
  "confirm-delete"
]);
</script>

<style scoped>
.confirm-overlay {
  position: fixed;
  inset: 0;
  z-index: 2000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  box-sizing: border-box;
  background: rgba(0, 0, 0, 0.35);
}

.confirm-panel {
  width: min(400px, 100%);
  padding: 20px 22px;
  border-radius: 12px;
  background: #fff;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.16);
}

.confirm-title {
  margin: 0 0 8px;
  font-size: 16px;
  font-weight: 600;
  color: #222;
}

.confirm-desc {
  margin: 0 0 20px;
  font-size: 14px;
  line-height: 1.5;
  color: #444;
}

.confirm-error {
  margin: -12px 0 16px;
  font-size: 13px;
  line-height: 1.4;
  color: #c62828;
}

.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.confirm-btn {
  border: none;
  border-radius: 8px;
  padding: 8px 16px;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
}

.confirm-btn:disabled {
  opacity: 0.55;
  cursor: default;
}

.confirm-btn--ghost {
  background: transparent;
  color: #444;
}

.confirm-btn--ghost:hover:not(:disabled) {
  background: #f3f3f5;
}

.confirm-btn--danger {
  color: #fff;
  background: #c62828;
}

.confirm-btn--danger:hover:not(:disabled) {
  background: #b71c1c;
}
</style>
