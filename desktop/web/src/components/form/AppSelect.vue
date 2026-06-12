<template>
  <div ref="rootRef" class="app-select" :class="[`app-select--${size}`, { 'app-select--open': open }]">
    <button
      type="button"
      class="app-select-trigger"
      :disabled="disabled"
      :aria-expanded="open ? 'true' : 'false'"
      aria-haspopup="listbox"
      @click="toggle"
    >
      <span class="app-select-value">{{ selectedLabel }}</span>
      <svg
        class="app-select-chevron"
        viewBox="0 0 12 12"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M3 4.5 6 7.5 9 4.5" />
      </svg>
    </button>

    <Teleport to="body">
      <div
        v-if="open"
        class="app-select-layer"
        @click="close"
        @contextmenu.prevent="close"
      >
        <ul
          class="app-select-menu"
          role="listbox"
          :style="menuStyle"
          @click.stop
        >
          <li v-for="opt in options" :key="opt.value" role="none">
            <button
              type="button"
              class="app-select-option"
              role="option"
              :aria-selected="opt.value === modelValue ? 'true' : 'false'"
              :class="{ 'is-selected': opt.value === modelValue }"
              @click="selectOption(opt.value)"
            >
              <span class="app-select-option-label">{{ opt.label }}</span>
              <svg
                v-if="opt.value === modelValue"
                class="app-select-option-check"
                viewBox="0 0 12 12"
                fill="none"
                stroke="currentColor"
                stroke-width="1.75"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M2.5 6 5 8.5 9.5 3.5" />
              </svg>
            </button>
          </li>
        </ul>
      </div>
    </Teleport>
  </div>
</template>

<script setup>
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";

const props = defineProps({
  modelValue: { type: String, default: "" },
  options: {
    type: Array,
    default: () => [],
  },
  disabled: { type: Boolean, default: false },
  placeholder: { type: String, default: "请选择" },
  size: {
    type: String,
    default: "default",
    validator: (v) => ["default", "compact"].includes(v),
  },
});

const emit = defineEmits(["update:modelValue"]);

/** Gap between trigger and dropdown menu (px). */
const MENU_GAP_PX = 4;
/** Maximum dropdown height before scrolling (px). */
const MENU_MAX_HEIGHT_PX = 240;
/** Prefer opening upward when space below is less than this (px). */
const MENU_FLIP_THRESHOLD_PX = 120;

const rootRef = ref(null);
const open = ref(false);
const menuStyle = ref({});

const selectedLabel = computed(() => {
  const match = props.options.find((opt) => opt.value === props.modelValue);
  if (match?.label) return match.label;
  if (props.modelValue) return props.modelValue;
  return props.placeholder;
});

function updateMenuPosition() {
  const el = rootRef.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  const spaceBelow = Math.max(0, window.innerHeight - rect.bottom - MENU_GAP_PX);
  const spaceAbove = Math.max(0, rect.top - MENU_GAP_PX);
  const openUp = spaceBelow < MENU_FLIP_THRESHOLD_PX && spaceAbove > spaceBelow;
  const maxMenuHeight = openUp
    ? Math.min(MENU_MAX_HEIGHT_PX, spaceAbove)
    : Math.min(MENU_MAX_HEIGHT_PX, spaceBelow);
  const base = {
    left: `${Math.round(rect.left)}px`,
    width: `${Math.round(rect.width)}px`,
    maxHeight: `${Math.round(maxMenuHeight)}px`,
  };

  menuStyle.value = openUp
    ? { ...base, bottom: `${Math.round(window.innerHeight - rect.top + MENU_GAP_PX)}px` }
    : { ...base, top: `${Math.round(rect.bottom + MENU_GAP_PX)}px` };
}

function openMenu() {
  if (props.disabled) return;
  open.value = true;
  void nextTick(() => updateMenuPosition());
}

function close() {
  open.value = false;
}

function toggle() {
  if (open.value) {
    close();
  } else {
    openMenu();
  }
}

function selectOption(value) {
  emit("update:modelValue", value);
  close();
}

function onWindowKeydown(event) {
  if (event.key === "Escape") {
    close();
  }
}

function onWindowRelayout() {
  if (open.value) {
    updateMenuPosition();
  }
}

watch(
  () => props.options,
  () => {
    if (open.value) {
      void nextTick(() => updateMenuPosition());
    }
  }
);

onMounted(() => {
  window.addEventListener("keydown", onWindowKeydown);
  window.addEventListener("resize", onWindowRelayout);
  window.addEventListener("scroll", onWindowRelayout, true);
});

onUnmounted(() => {
  window.removeEventListener("keydown", onWindowKeydown);
  window.removeEventListener("resize", onWindowRelayout);
  window.removeEventListener("scroll", onWindowRelayout, true);
});
</script>

<style scoped>
.app-select {
  position: relative;
  width: 100%;
}

.app-select-trigger {
  box-sizing: border-box;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  margin: 0;
  border: 1px solid var(--form-control-border, #d5d5d9);
  border-radius: 6px;
  background: var(--form-control-bg, #fff);
  color: #333;
  font-family: inherit;
  text-align: left;
  cursor: pointer;
  transition:
    border-color 0.15s ease,
    box-shadow 0.15s ease;
}

.app-select--default .app-select-trigger {
  min-height: 38px;
  padding: 0 10px;
  font-size: 14px;
  line-height: 1.35;
}

.app-select--compact .app-select-trigger {
  min-height: 32px;
  padding: 6px 10px;
  font-size: 12px;
  line-height: 1.35;
}

.app-select-trigger:hover:not(:disabled) {
  border-color: #b8b8bf;
}

.app-select--open .app-select-trigger,
.app-select-trigger:focus-visible {
  outline: none;
  border-color: #0d9ea6;
  box-shadow: 0 0 0 2px rgba(13, 158, 166, 0.16);
}

.app-select-trigger:disabled {
  opacity: 0.55;
  cursor: not-allowed;
  background: #f5f5f5;
}

.app-select-value {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.app-select-chevron {
  flex-shrink: 0;
  width: 12px;
  height: 12px;
  color: #888;
  transition: transform 0.15s ease;
}

.app-select--open .app-select-chevron {
  transform: rotate(180deg);
}

.app-select-layer {
  position: fixed;
  inset: 0;
  z-index: 2100;
}

.app-select-menu {
  position: fixed;
  margin: 0;
  padding: 4px;
  list-style: none;
  box-sizing: border-box;
  border-radius: 8px;
  border: 1px solid rgba(0, 0, 0, 0.1);
  background: #fff;
  box-shadow:
    0 4px 16px rgba(0, 0, 0, 0.12),
    0 0 1px rgba(0, 0, 0, 0.08);
  overflow: auto;
}

.app-select-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  margin: 0;
  padding: 7px 10px;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: #1a1a1e;
  font-size: 13px;
  line-height: 1.35;
  font-family: inherit;
  text-align: left;
  cursor: pointer;
  transition: background 0.1s ease;
}

.app-select-option:hover {
  background: rgba(0, 0, 0, 0.05);
}

.app-select-option.is-selected {
  background: rgba(13, 158, 166, 0.08);
  color: #0a8a91;
  font-weight: 500;
}

.app-select-option-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.app-select-option-check {
  flex-shrink: 0;
  width: 14px;
  height: 14px;
  color: #0d9ea6;
}
</style>
