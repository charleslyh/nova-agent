<template>
  <div class="dialog-overlay" @click.self="$emit('close')">
    <div class="dialog" role="dialog" @click.stop>
      <h3 class="dialog-title">企业微信 AI 机器人配置</h3>
      <p v-if="error" class="dialog-error">{{ error }}</p>
      <form v-if="!loading" class="dialog-form" @submit.prevent="handleSave">
        <label class="field" :class="{ 'field--readonly': hasExisting }">
          <span>Bot ID</span>
          <input
            v-model="form.bot_id"
            type="text"
            required
            :readonly="hasExisting"
            autocomplete="off"
          />
          <span v-if="hasExisting" class="field-hint">创建后不可修改</span>
        </label>
        <label class="field">
          <span>Bot Secret</span>
          <input
            v-model="form.bot_secret"
            type="password"
            :required="!hasExisting"
            :placeholder="hasExisting ? '已配置，留空表示不修改' : ''"
            autocomplete="off"
          />
        </label>
        <ChannelAgentField
          v-model="selectedAgentId"
          :session-id="sessionIdForAgent"
          :get-settings-catalog="getSettingsCatalog"
          :get-session-agent="getSessionAgent"
        />
        <label class="field field--row">
          <input v-model="welcomeDisabled" type="checkbox" />
          <span>不发送进入会话欢迎语</span>
        </label>
        <label class="field" :class="{ 'field--disabled': welcomeDisabled }">
          <span>欢迎语（留空使用默认）</span>
          <textarea
            v-model="form.welcome_message"
            rows="2"
            :disabled="welcomeDisabled"
            placeholder="你好！我是 Moray 助手，有什么可以帮你的吗？"
          />
        </label>
        <div class="dialog-actions">
          <button type="button" class="btn btn--muted" @click="$emit('close')">取消</button>
          <button type="submit" class="btn btn--primary" :disabled="saving">保存</button>
        </div>
      </form>
      <p v-else class="dialog-loading">加载中…</p>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from "vue";
import ChannelAgentField from "@/components/channels/ChannelAgentField.vue";

const props = defineProps({
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

const emit = defineEmits(["close", "saved"]);

const form = ref({
  session_id: "",
  bot_id: "",
  bot_secret: "",
  heartbeat_interval_ms: 30000,
  max_reconnect_attempts: 10,
  welcome_message: ""
});
const loading = ref(true);
const saving = ref(false);
const hasExisting = ref(false);
const lockedBotId = ref("");
const welcomeDisabled = ref(false);
const selectedAgentId = ref("");
const error = ref("");
const initialWelcomeDisabled = ref(false);
const initialWelcomeMessage = ref("");

const sessionIdForAgent = computed(() => {
  if (props.isNew) return "";
  return props.sessionId || form.value.session_id || "";
});

async function applySessionAgent(sessionId) {
  const agentId = selectedAgentId.value?.trim();
  if (!sessionId || !agentId) return;
  await props.setSessionAgent(sessionId, agentId);
}

function normalizeWelcome(payload) {
  if (payload.welcome_message === "") return "";
  return String(payload.welcome_message ?? "").trim();
}

function hasChannelConfigChanges(payload) {
  if (!hasExisting.value) return true;
  if (String(payload.bot_secret ?? "").trim()) return true;
  const currentWelcomeDisabled = normalizeWelcome(payload) === "";
  if (currentWelcomeDisabled !== initialWelcomeDisabled.value) return true;
  return normalizeWelcome(payload) !== initialWelcomeMessage.value;
}

onMounted(async () => {
  if (props.isNew) {
    hasExisting.value = false;
    loading.value = false;
    return;
  }
  try {
    const raw = await props.getChannelConfig(props.channelId);
    const cfg = raw?.data ?? raw;
    if (cfg) {
      const { bot_secret: _masked, session_id: _sid, ...rest } = cfg;
      form.value = {
        ...form.value,
        ...rest,
        bot_secret: "",
        welcome_message: cfg.welcome_message ?? ""
      };
      welcomeDisabled.value = cfg.welcome_message === "";
      initialWelcomeDisabled.value = cfg.welcome_message === "";
      initialWelcomeMessage.value = String(cfg.welcome_message ?? "").trim();
      lockedBotId.value = cfg.bot_id ?? "";
      hasExisting.value = true;
    }
  } catch (e) {
    console.debug("load wecom config:", e);
    hasExisting.value = false;
  } finally {
    loading.value = false;
  }
});

async function handleSave() {
  saving.value = true;
  error.value = "";
  try {
    const payload = { ...form.value };
    if (hasExisting.value && !String(payload.bot_secret ?? "").trim()) {
      payload.bot_secret = "";
    }
    if (welcomeDisabled.value) {
      payload.welcome_message = "";
    } else {
      const welcome = (payload.welcome_message ?? "").trim();
      if (welcome) {
        payload.welcome_message = welcome;
      } else {
        delete payload.welcome_message;
      }
    }
    const botId = hasExisting.value
      ? lockedBotId.value
      : String(payload.bot_id ?? "").trim();
    if (!botId) {
      error.value = "请填写 Bot ID";
      saving.value = false;
      return;
    }
    payload.bot_id = botId;
    if (props.isNew) {
      const created = await props.createChannel({
        name: botId,
        type: "wecom",
        data: payload
      });
      await applySessionAgent(created.session_id);
      emit("saved", { channelChanged: true });
    } else {
      const channelChanged = hasChannelConfigChanges(payload);
      if (channelChanged) {
        await props.saveChannelConfig(props.channelId, { data: payload });
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
.dialog-overlay {
  position: fixed;
  inset: 0;
  z-index: 200;
  background: rgba(0, 0, 0, 0.35);
  display: flex;
  align-items: center;
  justify-content: center;
}

.dialog {
  width: min(420px, 92vw);
  padding: 20px;
  border-radius: 12px;
  background: #fff;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.15);
}

.dialog-title {
  margin: 0 0 16px;
  font-size: 16px;
}

.dialog-error {
  color: #b42318;
  font-size: 13px;
}

.dialog-form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 13px;
}

.field input[type="text"],
.field input[type="password"],
.field textarea {
  box-sizing: border-box;
  padding: 8px 10px;
  border: 1px solid rgba(0, 0, 0, 0.15);
  border-radius: 6px;
  font-family: inherit;
  font-size: 13px;
  resize: vertical;
  background-color: #fff;
}

.field--row {
  flex-direction: row;
  align-items: center;
  gap: 8px;
}

.field--readonly input[readonly] {
  cursor: not-allowed;
  background-color: #f5f5f5;
  color: #666;
}

.field-hint {
  font-size: 12px;
  color: #888;
}

.field--disabled {
  opacity: 0.55;
}

.field--disabled textarea:disabled {
  cursor: not-allowed;
  background-color: #f5f5f5;
}

.dialog-actions {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  margin-top: 8px;
}

.btn {
  padding: 8px 14px;
  border-radius: 6px;
  border: 1px solid rgba(0, 0, 0, 0.12);
  cursor: pointer;
  font-size: 13px;
  font-family: inherit;
}

.btn--primary {
  background-color: #111;
  color: #fff;
  border-color: #111;
}

.btn--danger {
  color: #b42318;
  margin-right: auto;
}

.btn--muted {
  background-color: #f5f5f5;
}

.dialog-loading {
  font-size: 13px;
  color: #888;
}
</style>
