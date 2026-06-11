<template>
  <div :class="embedded ? 'channel-config-embedded' : 'channel-config-dialog'">
    <h3 v-if="!embedded" class="channel-config-title">企业微信 AI 机器人配置</h3>
    <p v-if="error" class="channel-config-error">{{ error }}</p>
    <component
      :is="embedded ? 'div' : 'form'"
      v-if="!loading"
      class="channel-config-form"
      @submit.prevent="handleSave"
    >
      <label class="channel-field" :class="{ 'channel-field--readonly': hasExisting }">
        <span class="channel-field-label-row">
          <span class="channel-field-label">Bot ID</span>
          <span v-if="hasExisting" class="channel-field-hint">创建后不可修改</span>
        </span>
        <input
          v-model="form.bot_id"
          type="text"
          :required="!embedded"
          :readonly="hasExisting"
          class="channel-input"
          :disabled="embedded && saving"
          autocomplete="off"
        />
      </label>
      <label class="channel-field">
        <span class="channel-field-label">Bot Secret</span>
        <input
          v-model="form.bot_secret"
          type="password"
          :required="!embedded && !hasExisting"
          :placeholder="hasExisting ? '已配置，留空表示不修改' : ''"
          class="channel-input"
          :disabled="embedded && saving"
          autocomplete="off"
          @blur="onEmbeddedSecretBlur"
        />
      </label>
      <ChannelAgentField
        v-if="!hideAgent"
        v-model="selectedAgentId"
        :session-id="sessionIdForAgent"
        :get-settings-catalog="getSettingsCatalog"
        :get-session-agent="getSessionAgent"
      />
      <div v-if="!embedded" class="channel-config-actions">
        <button type="button" class="channel-btn channel-btn--muted" @click="$emit('close')">
          取消
        </button>
        <button type="submit" class="channel-btn channel-btn--primary" :disabled="saving">
          {{ saving ? "保存中…" : "保存" }}
        </button>
      </div>
    </component>
    <p v-else class="channel-config-loading">加载中…</p>
  </div>
</template>

<script setup>
import { computed, onMounted, ref, watch } from "vue";
import ChannelAgentField from "@/components/channels/ChannelAgentField.vue";

const props = defineProps({
  channelId: { type: String, default: "" },
  sessionId: { type: String, default: "" },
  isNew: { type: Boolean, default: true },
  embedded: { type: Boolean, default: false },
  hideAgent: { type: Boolean, default: false },
  getChannelConfig: { type: Function, required: true },
  saveChannelConfig: { type: Function, required: true },
  createChannel: { type: Function, required: true },
  getSettingsCatalog: { type: Function, default: null },
  getSessionAgent: { type: Function, default: null },
  setSessionAgent: { type: Function, default: null },
});

const emit = defineEmits(["close", "saved"]);

const form = ref({
  session_id: "",
  bot_id: "",
  bot_secret: "",
  heartbeat_interval_ms: 30000,
  max_reconnect_attempts: 10,
});
const loading = ref(true);
const saving = ref(false);
const hasExisting = ref(false);
const lockedBotId = ref("");
const selectedAgentId = ref("");
const error = ref("");

let hydrating = false;
let persistInFlight = 0;

const sessionIdForAgent = computed(() => {
  if (props.isNew) return "";
  return props.sessionId || form.value.session_id || "";
});

async function applySessionAgent(sessionId) {
  if (props.hideAgent || !props.setSessionAgent) return;
  const agentId = selectedAgentId.value?.trim();
  if (!sessionId || !agentId) return;
  await props.setSessionAgent(sessionId, agentId);
}

function hasChannelConfigChanges(payload) {
  if (!hasExisting.value) return true;
  return Boolean(String(payload.bot_secret ?? "").trim());
}

function buildPayload() {
  const payload = { ...form.value };
  if (hasExisting.value && !String(payload.bot_secret ?? "").trim()) {
    payload.bot_secret = "";
  }
  const botId = hasExisting.value
    ? lockedBotId.value
    : String(payload.bot_id ?? "").trim();
  payload.bot_id = botId;
  return payload;
}

function applySavedBaseline(payload) {
  if (String(payload.bot_secret ?? "").trim()) {
    form.value.bot_secret = "";
  }
}

function onEmbeddedSecretBlur() {
  if (!props.embedded || hydrating || loading.value) return;
  if (!String(form.value.bot_secret ?? "").trim()) return;
  void persistEmbeddedConfig();
}

async function persistEmbeddedConfig() {
  if (!props.embedded || hydrating || loading.value || props.isNew || !hasExisting.value) {
    return;
  }

  const payload = buildPayload();
  if (!payload.bot_id) return;
  if (!hasChannelConfigChanges(payload)) return;

  persistInFlight += 1;
  saving.value = true;
  error.value = "";
  try {
    await props.saveChannelConfig(props.channelId, { data: payload });
    applySavedBaseline(payload);
    emit("saved", { channelChanged: true });
  } catch (e) {
    error.value = String(e?.message ?? e);
  } finally {
    persistInFlight = Math.max(0, persistInFlight - 1);
    saving.value = persistInFlight > 0;
  }
}

async function loadConfig() {
  if (props.isNew) {
    hasExisting.value = false;
    loading.value = false;
    return;
  }
  hydrating = true;
  loading.value = true;
  error.value = "";
  try {
    const raw = await props.getChannelConfig(props.channelId);
    const cfg = raw?.data ?? raw;
    if (cfg) {
      const { bot_secret: _masked, session_id: _sid, ...rest } = cfg;
      form.value = {
        ...form.value,
        ...rest,
        bot_secret: "",
      };
      lockedBotId.value = cfg.bot_id ?? "";
      hasExisting.value = true;
    }
  } catch (e) {
    console.debug("load wecom config:", e);
    hasExisting.value = false;
  } finally {
    loading.value = false;
    hydrating = false;
  }
}

onMounted(() => {
  void loadConfig();
});

watch(
  () => [props.channelId, props.isNew],
  () => {
    void loadConfig();
  }
);

async function handleSave() {
  saving.value = true;
  error.value = "";
  try {
    const payload = buildPayload();
    const botId = payload.bot_id;
    if (!botId) {
      error.value = "请填写 Bot ID";
      saving.value = false;
      return;
    }
    if (props.isNew) {
      const created = await props.createChannel({
        name: botId,
        type: "wecom",
        data: payload,
      });
      await applySessionAgent(created.session_id);
      emit("saved", { channelChanged: true });
    } else {
      const channelChanged = hasChannelConfigChanges(payload);
      if (channelChanged) {
        await props.saveChannelConfig(props.channelId, { data: payload });
        applySavedBaseline(payload);
      }
      const sessionId = props.sessionId || form.value.session_id;
      await applySessionAgent(sessionId);
      emit("saved", { channelChanged });
    }
  } catch (e) {
    error.value = String(e?.message ?? e);
  } finally {
    saving.value = false;
  }
}
</script>

<style scoped>
.channel-config-dialog {
  width: min(420px, 92vw);
  padding: 20px;
  border-radius: 12px;
  background: #fff;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.15);
}

.channel-config-embedded {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.channel-config-title {
  margin: 0 0 16px;
  font-size: 16px;
}

.channel-config-error {
  margin: 0;
  color: #b42318;
  font-size: 12px;
  line-height: 1.4;
}

.channel-config-form {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.channel-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.channel-field-label-row {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 6px;
}

.channel-field-label {
  font-size: 12px;
  font-weight: 600;
  color: #444;
}

.channel-input {
  box-sizing: border-box;
  width: 100%;
  padding: 7px 10px;
  border: 1px solid #d5d5d9;
  border-radius: 6px;
  font-size: 12px;
  font-family: inherit;
  color: #333;
  background: #fff;
}

.channel-field--readonly .channel-input[readonly] {
  cursor: not-allowed;
  background-color: #f5f5f5;
  color: #666;
}

.channel-field-hint {
  font-size: 11px;
  color: #888;
}

.channel-config-actions {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  margin-top: 4px;
}

.channel-btn {
  padding: 7px 14px;
  border-radius: 6px;
  border: 1px solid rgba(0, 0, 0, 0.12);
  cursor: pointer;
  font-size: 12px;
  font-family: inherit;
}

.channel-btn--primary {
  background-color: #111;
  color: #fff;
  border-color: #111;
}

.channel-btn--muted {
  background-color: #f5f5f5;
}

.channel-btn:disabled {
  opacity: 0.55;
  cursor: default;
}

.channel-config-loading {
  margin: 0;
  font-size: 12px;
  color: #888;
}
</style>
