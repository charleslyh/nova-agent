<template>
  <label class="field">
    <span>{{ label }}</span>
    <AppSelect
      :model-value="modelValue"
      :options="options"
      :disabled="disabled"
      @update:model-value="emit('update:modelValue', $event)"
    />
  </label>
</template>

<script setup>
import AppSelect from "@/components/form/AppSelect.vue";

defineProps({
  label: { type: String, required: true },
  modelValue: { type: String, default: "" },
  options: {
    type: Array,
    default: () => [],
  },
  disabled: { type: Boolean, default: false },
});

const emit = defineEmits(["update:modelValue"]);
</script>

<style scoped>
.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 13px;
  color: #444;
}
</style>
