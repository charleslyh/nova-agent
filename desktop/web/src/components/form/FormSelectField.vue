<template>
  <label class="field">
    <span>{{ label }}</span>
    <select
      class="field-input"
      :value="modelValue"
      :disabled="disabled"
      @change="emit('update:modelValue', $event.target.value)"
    >
      <option v-for="opt in options" :key="opt.value" :value="opt.value">
        {{ opt.label }}
      </option>
    </select>
  </label>
</template>

<script setup>
defineProps({
  label: { type: String, required: true },
  modelValue: { type: String, default: "" },
  options: {
    type: Array,
    default: () => []
  },
  disabled: { type: Boolean, default: false }
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
  background-color: #fff;
  color: inherit;
}

.field select.field-input {
  padding: 0 28px 0 10px;
  line-height: 36px;
  cursor: pointer;
}

.field select.field-input:disabled {
  cursor: not-allowed;
  opacity: 0.65;
  background-color: #f5f5f5;
}
</style>
