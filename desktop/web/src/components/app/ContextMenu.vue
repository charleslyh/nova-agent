<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="context-menu-layer"
      @click="$emit('close')"
      @contextmenu.prevent="$emit('close')"
    >
      <div
        class="context-menu"
        role="menu"
        :aria-label="ariaLabel"
        :style="{ top: `${y}px`, left: `${x}px` }"
        @click.stop
      >
        <button
          v-for="item in items"
          :key="item.id"
          type="button"
          class="context-menu__item"
          role="menuitem"
          @click="$emit('select', item.id)"
        >
          <span v-if="itemPlatform(item)" class="context-menu__item-icon" aria-hidden="true">
            <ChannelSessionIcon :platform="itemPlatform(item)" />
          </span>
          <span class="context-menu__item-label">{{ item.label }}</span>
        </button>
      </div>
    </div>
  </Teleport>
</template>

<script setup>
import ChannelSessionIcon from "@/components/icons/ChannelSessionIcon.vue";

defineProps({
  open: { type: Boolean, default: false },
  x: { type: Number, default: 0 },
  y: { type: Number, default: 0 },
  items: { type: Array, default: () => [] },
  ariaLabel: { type: String, default: "菜单" }
});

defineEmits(["close", "select"]);

function itemPlatform(item) {
  return item?.platform ?? item?.id ?? "";
}
</script>

<style scoped>
.context-menu-layer {
  position: fixed;
  inset: 0;
  z-index: 2100;
}

.context-menu {
  position: fixed;
  min-width: 160px;
  padding: 4px;
  border-radius: 8px;
  background: #fff;
  border: 1px solid rgba(0, 0, 0, 0.1);
  box-shadow:
    0 4px 16px rgba(0, 0, 0, 0.12),
    0 0 1px rgba(0, 0, 0, 0.08);
  box-sizing: border-box;
}

.context-menu__item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  margin: 0;
  padding: 6px 10px;
  border: none;
  border-radius: 5px;
  background-color: transparent;
  color: #1a1a1e;
  font-size: 13px;
  font-weight: 400;
  line-height: 1.35;
  font-family: inherit;
  text-align: left;
  white-space: nowrap;
  cursor: pointer;
  transition: background 0.1s ease;
}

.context-menu__item-icon {
  flex-shrink: 0;
  width: 16px;
  height: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #5c5c66;
}

.context-menu__item-icon :deep(svg) {
  width: 16px;
  height: 16px;
}

.context-menu__item-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.context-menu__item:hover {
  background: rgba(0, 0, 0, 0.06);
}
</style>
