<template>
  <div v-if="open && agent" class="backdrop" @click.self="$emit('close')">
    <div
      class="dialog"
      role="dialog"
      aria-labelledby="agent-edit-title"
      aria-modal="true"
      @click.stop
    >
      <header class="dialog-head">
        <h2 id="agent-edit-title" class="title">Agent 编辑</h2>
        <button type="button" class="close-btn" @click="$emit('close')">关闭</button>
      </header>
      <div class="body">
        <label class="field">
          <span>名称</span>
          <input
            v-model.trim="draftName"
            type="text"
            class="field-input"
            autocomplete="off"
            spellcheck="false"
          />
        </label>
        <FormSelectField
          v-model="draftCompletionId"
          label="模型"
          :options="completionOptions"
        />
        <label class="field">
          <span>人设</span>
          <textarea
            v-model="draftCharacter"
            class="field-input field-textarea"
            rows="4"
            placeholder="可选，用于强调助手的自定义名称、风格、偏好、价值观等"
            spellcheck="false"
          />
        </label>
        <div
          class="field tools-field"
          role="group"
          aria-labelledby="agent-tools-heading"
        >
          <div id="agent-tools-heading" class="tools-heading">
            <span>工具白名单</span>
            <span class="tools-hint">勾选启用</span>
          </div>
          <ul v-if="availableTools.length" class="tools-picker">
            <li v-for="tool in availableTools" :key="tool.id">
              <label class="tool-chip" :title="tool.id">
                <input
                  type="checkbox"
                  class="tool-chip-check"
                  :checked="draftAllowedTools.has(tool.id)"
                  @change="toggleTool(tool.id)"
                />
                <span class="tool-chip-label">{{ tool.label }}</span>
              </label>
            </li>
          </ul>
          <p v-else class="tools-empty">服务端未注册可用工具</p>
        </div>
      </div>
      <footer class="dialog-foot">
        <button
          type="button"
          class="save-btn"
          :disabled="saveDisabled"
          @click="onSave"
        >
          {{ saving ? "保存中…" : "保存" }}
        </button>
      </footer>
    </div>
  </div>
</template>

<script setup>
import { computed, ref, watch } from "vue";
import FormSelectField from "@/components/form/FormSelectField.vue";

const props = defineProps({
  open: { type: Boolean, required: true },
  agent: {
    type: Object,
    default: null
  },
  completions: {
    type: Array,
    required: true
  },
  availableTools: {
    type: Array,
    default: () => []
  },
  saveAgent: {
    type: Function,
    required: true
  }
});

const emit = defineEmits(["close"]);

const completionOptions = computed(() =>
  props.completions.map((c) => ({ value: c.id, label: c.name }))
);

const draftName = ref("");
const draftCompletionId = ref("");
const draftCharacter = ref("");
const draftAllowedTools = ref(new Set());
const saving = ref(false);

function agentAllowedToolNames(agent) {
  if (!Array.isArray(agent?.allowed_tools)) {
    return [];
  }
  return agent.allowed_tools;
}

function normalizeAllowedTools(selected, catalogTools) {
  const set = new Set(Array.isArray(selected) ? selected : []);
  const order = Array.isArray(catalogTools) ? catalogTools : [];
  return order.map((t) => t.id).filter((id) => set.has(id));
}

function sameAllowedToolList(a, b, catalogTools) {
  const left = normalizeAllowedTools(a, catalogTools);
  const right = normalizeAllowedTools(b, catalogTools);
  return left.length === right.length && left.every((name, i) => name === right[i]);
}

function resetDraftFromAgent() {
  if (!props.agent) return;
  draftName.value = props.agent.name ?? "";
  draftCompletionId.value = props.agent.completion_id ?? "";
  draftCharacter.value = props.agent.character ?? "";
  draftAllowedTools.value = new Set(agentAllowedToolNames(props.agent));
}

watch(
  () => [props.open, props.agent?.id],
  () => {
    if (!props.open || !props.agent) return;
    resetDraftFromAgent();
  },
  { immediate: true }
);

function toggleTool(name) {
  const next = new Set(draftAllowedTools.value);
  if (next.has(name)) {
    next.delete(name);
  } else {
    next.add(name);
  }
  draftAllowedTools.value = next;
}

const draftAllowedToolsList = computed(() =>
  normalizeAllowedTools([...draftAllowedTools.value], props.availableTools)
);

const isDirty = computed(() => {
  if (!props.agent) return false;
  return (
    draftName.value !== (props.agent.name ?? "") ||
    draftCompletionId.value !== (props.agent.completion_id ?? "") ||
    (draftCharacter.value.trim() !== (props.agent.character ?? "").trim()) ||
    !sameAllowedToolList(
      draftAllowedToolsList.value,
      agentAllowedToolNames(props.agent),
      props.availableTools
    )
  );
});

const saveDisabled = computed(
  () => saving.value || !draftName.value || !isDirty.value
);

async function onSave() {
  if (!props.agent || saveDisabled.value) return;
  saving.value = true;
  try {
    await props.saveAgent({
      agentId: props.agent.id,
      name: draftName.value,
      completionId: draftCompletionId.value,
      allowedTools: draftAllowedToolsList.value,
      character: draftCharacter.value.trim() || null
    });
    emit("close");
  } catch (e) {
    console.error(e);
  } finally {
    saving.value = false;
  }
}
</script>

<style scoped>
.backdrop {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  z-index: 60;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  box-sizing: border-box;
}

.dialog {
  width: min(440px, 100%);
  max-height: min(80vh, 560px);
  background: #fff;
  border-radius: 12px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.22);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.dialog-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 16px;
  border-bottom: 1px solid #ececec;
}

.title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
}

.close-btn {
  border: 1px solid #d5d5d9;
  border-radius: 6px;
  padding: 4px 12px;
  background: #fff;
  font-size: 12px;
  cursor: pointer;
}

.body {
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  overflow-y: auto;
  flex: 1 1 auto;
  min-height: 0;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 13px;
  color: #444;
}

.field-input {
  box-sizing: border-box;
  width: 100%;
  height: 38px;
  padding: 0 10px;
  border-radius: 6px;
  border: 1px solid #ccc;
  font-size: 14px;
  line-height: 22px;
  font-family: inherit;
  background: #fff;
  color: inherit;
}

.field-textarea {
  height: auto;
  min-height: 5rem;
  padding: 8px 10px;
  resize: vertical;
  line-height: 1.4;
}

.tools-field {
  min-width: 0;
}

.tools-heading {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: 13px;
  color: #444;
  margin-bottom: 6px;
}

.tools-hint {
  font-size: 11px;
  font-weight: 400;
  color: #888;
}

.tools-empty {
  margin: 0;
  font-size: 12px;
  color: #888;
}

.tools-picker {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 1px;
  border: 1px solid #e5e7eb;
  border-radius: 6px;
  overflow: hidden;
}

.tools-picker > li {
  min-width: 0;
}

.tool-chip {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 30px;
  padding: 4px 8px;
  cursor: pointer;
  user-select: none;
}

.tool-chip:hover {
  background: #f3f4f6;
  /* border-color: #d1d5db; */
}

.tool-chip-check {
  margin: 0;
  flex-shrink: 0;
  width: 14px;
  height: 14px;
}

.tool-chip-label {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  font-weight: 500;
  color: #222;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dialog-foot {
  display: flex;
  justify-content: flex-end;
  padding: 12px 16px 16px;
  border-top: 1px solid #ececec;
  flex-shrink: 0;
}

.save-btn {
  border: none;
  border-radius: 8px;
  padding: 8px 20px;
  font-size: 14px;
  font-weight: 500;
  color: #fff;
  background: #1976ff;
  cursor: pointer;
}

.save-btn:hover:not(:disabled) {
  background: #1565c0;
}

.save-btn:disabled {
  background: #a6aab3;
  cursor: not-allowed;
}
</style>
