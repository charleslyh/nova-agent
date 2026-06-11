<template>
  <div class="dialog-overlay" @click.self="$emit('close')">
    <WeComChannelConfigForm
      :channel-id="channelId"
      :session-id="sessionId"
      :is-new="isNew"
      :get-channel-config="getChannelConfig"
      :save-channel-config="saveChannelConfig"
      :create-channel="createChannel"
      :get-settings-catalog="getSettingsCatalog"
      :get-session-agent="getSessionAgent"
      :set-session-agent="setSessionAgent"
      @close="$emit('close')"
      @saved="$emit('saved', $event)"
    />
  </div>
</template>

<script setup>
import WeComChannelConfigForm from "@/components/channels/WeComChannelConfigForm.vue";

defineProps({
  channelId: { type: String, default: "" },
  sessionId: { type: String, default: "" },
  isNew: { type: Boolean, default: true },
  getChannelConfig: { type: Function, required: true },
  saveChannelConfig: { type: Function, required: true },
  createChannel: { type: Function, required: true },
  getSettingsCatalog: { type: Function, required: true },
  getSessionAgent: { type: Function, required: true },
  setSessionAgent: { type: Function, required: true },
});

defineEmits(["close", "saved"]);
</script>

<style scoped>
.dialog-overlay {
  position: fixed;
  inset: 0;
  z-index: 200;
  background: rgba(0, 0, 0, 0.35);
  display: flex;
  align-items: center;
  justify-content: center;
}
</style>
