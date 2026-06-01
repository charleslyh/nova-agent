<template>
  <div class="channel-config-modals">
    <div v-if="pickerOpen" class="channel-picker-overlay" @click.self="$emit('close-picker')">
      <div class="channel-picker-panel" role="dialog" aria-label="选择频道类型" @click.stop>
        <p class="channel-picker__title">选择频道类型</p>
        <button
          v-for="t in CHANNEL_TYPES"
          :key="t.id"
          type="button"
          class="channel-picker-btn"
          @click="$emit('pick-type', t.id)"
        >
          {{ t.label }}
        </button>
        <button type="button" class="channel-picker-btn channel-picker-btn--muted" @click="$emit('close-picker')">
          取消
        </button>
      </div>
    </div>

    <QQConfigDialog
      v-if="editing?.type === 'qq'"
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
      v-if="editing?.type === 'wecom'"
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
  pickerOpen: { type: Boolean, default: false },
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
  "close-picker",
  "pick-type",
  "close-edit",
  "saved",
  "cancel-delete",
  "confirm-delete"
]);

const CHANNEL_TYPES = [
  { id: "qq", label: "QQ 机器人" },
  { id: "wecom", label: "企业微信" }
];
</script>

<style scoped>
.channel-picker-overlay {
  position: fixed;
  inset: 0;
  z-index: 1900;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  box-sizing: border-box;
  background: rgba(0, 0, 0, 0.35);
}

.channel-picker-panel {
  width: min(320px, 100%);
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 16px;
  border-radius: 12px;
  background: #fff;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.16);
}

.channel-picker__title {
  margin: 0 0 4px;
  font-size: 14px;
  font-weight: 600;
  color: #222;
}

.channel-picker-btn {
  font-size: 13px;
  padding: 10px 12px;
  border-radius: 8px;
  border: 1px solid rgba(0, 0, 0, 0.12);
  background: #fafafa;
  cursor: pointer;
  text-align: left;
}

.channel-picker-btn--muted {
  color: #666;
  background: transparent;
  border-color: transparent;
}

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
