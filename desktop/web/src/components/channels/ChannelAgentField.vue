<template>
  <FormSelectField
    v-if="agentOptions.length"
    label="Agent"
    :model-value="modelValue"
    :options="agentOptions"
    :disabled="loading"
    @update:model-value="emit('update:modelValue', $event)"
  />
</template>

<script setup>
import { computed, onMounted, ref, watch } from "vue";
import FormSelectField from "@/components/form/FormSelectField.vue";

const props = defineProps({
  modelValue: { type: String, default: "" },
  sessionId: { type: String, default: "" },
  getSettingsCatalog: { type: Function, required: true },
  getSessionAgent: { type: Function, required: true }
});

const emit = defineEmits(["update:modelValue"]);

const agents = ref([]);
const loading = ref(true);

const agentOptions = computed(() =>
  agents.value.map((a) => ({ value: a.id, label: a.name }))
);

async function loadAgents() {
  loading.value = true;
  try {
    const catalog = await props.getSettingsCatalog();
    agents.value = Array.isArray(catalog?.agents) ? catalog.agents : [];
    if (props.sessionId) {
      const cur = await props.getSessionAgent(props.sessionId);
      if (cur?.agent_id) {
        emit("update:modelValue", cur.agent_id);
        return;
      }
    }
    if (!props.modelValue && agents.value.length) {
      emit("update:modelValue", agents.value[0].id);
    }
  } catch (e) {
    console.debug("load channel agent field:", e);
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  void loadAgents();
});

watch(
  () => props.sessionId,
  () => {
    void loadAgents();
  }
);
</script>
