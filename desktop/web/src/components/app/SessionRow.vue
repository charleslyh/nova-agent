<template>
  <div
    class="sidebar-entry sidebar-entry--session"
    :class="{ 'is-active': active }"
    role="listitem"
    :aria-current="active ? 'true' : undefined"
    tabindex="0"
    @click="$emit('select', item.sessionId)"
    @keydown.enter="$emit('select', item.sessionId)"
  >
    <span
      class="sidebar-entry__icon"
      :aria-hidden="item.workStatus === 'running' ? undefined : true"
      :aria-busy="item.workStatus === 'running' ? 'true' : undefined"
    >
      <svg
        v-if="item.workStatus === 'running'"
        viewBox="0 0 24 24"
        class="sidebar-entry__icon-svg sidebar-entry__icon-svg--spinner"
        fill="none"
        stroke="currentColor"
        stroke-width="1.75"
      >
        <path stroke-linecap="round" d="M12 3a9 9 0 1 0 9 9" />
      </svg>
      <ChannelSessionIcon
        v-else-if="channelIconPlatform"
        :platform="channelIconPlatform"
      />
      <svg
        v-else
        viewBox="0 0 24 24"
        class="sidebar-entry__icon-svg"
        fill="none"
        stroke="currentColor"
        stroke-width="1.75"
      >
        <path
          stroke-linecap="round"
          stroke-linejoin="round"
          d="M7 5.5h10a2.5 2.5 0 0 1 2.5 2.5v5a2.5 2.5 0 0 1-2.5 2.5H11l-2.5 2.5V15.5H7a2.5 2.5 0 0 1-2.5-2.5V8a2.5 2.5 0 0 1 2.5-2.5z"
        />
        <path stroke-linecap="round" d="M9.5 10h5" />
      </svg>
    </span>
    <span class="sidebar-entry__label-wrap">
      <span class="sidebar-entry__label" :title="rowTitle">{{ rowLabel }}</span>
      <button
        type="button"
        class="session-delete-btn"
        :aria-label="isChannelRow ? '删除频道' : '删除会话'"
        :title="isChannelRow ? '删除频道' : '删除会话'"
        @click.stop="$emit('delete', item.sessionId)"
      >
        ×
      </button>
    </span>
  </div>
</template>

<script setup>
import { computed } from "vue";
import ChannelSessionIcon from "@/components/icons/ChannelSessionIcon.vue";

const props = defineProps({
  item: { type: Object, required: true },
  active: { type: Boolean, default: false },
  showPlatform: { type: Boolean, default: false }
});

defineEmits(["select", "delete"]);

const isChannelRow = computed(
  () => props.showPlatform || props.item.sessionKind === "channel"
);

const channelIconPlatform = computed(() => {
  if (!isChannelRow.value) {
    return "";
  }
  return props.item.channelType ?? "";
});

const rowLabel = computed(() => {
  return props.item.name;
});

const rowTitle = computed(() => {
  return rowLabel.value;
});
</script>

<style scoped>
.sidebar-entry--session {
  gap: var(--sidebar-entry-gap, 8px);
  padding: var(--sidebar-entry-padding, 5px 8px);
  border-radius: var(--sidebar-entry-radius, 8px);
  font-family: var(--sidebar-font-family, inherit);
  font-size: var(--sidebar-entry-font-size, 13px);
  font-weight: var(--sidebar-entry-font-weight, 400);
  line-height: var(--sidebar-entry-line-height, 1.4);
  letter-spacing: var(--sidebar-entry-letter-spacing, -0.01em);
  color: var(--sidebar-entry-text, #2b2b30);
}

.sidebar-entry {
  display: flex;
  align-items: center;
  width: 100%;
  box-sizing: border-box;
  border: 1px solid transparent;
  background: transparent;
  cursor: pointer;
  transition: background 0.12s ease;
}

.sidebar-entry:hover {
  background: var(--sidebar-entry-hover-bg, rgba(0, 0, 0, 0.05));
}

.sidebar-entry.is-active {
  background: var(--sidebar-entry-active-bg, #e6e6ea);
  box-shadow: 0 0 8px 1px var(--sidebar-entry-active-border, #e6e6ea);
  font-weight: 700;
}

.sidebar-entry__icon {
  flex-shrink: 0;
  width: var(--sidebar-entry-icon-size, 16px);
  height: var(--sidebar-entry-icon-size, 16px);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--sidebar-entry-icon-color, #5c5c66);
}

.sidebar-entry__icon :deep(.sidebar-entry__icon-svg),
.sidebar-entry__icon-svg {
  width: var(--sidebar-entry-icon-size, 16px);
  height: var(--sidebar-entry-icon-size, 16px);
}

.sidebar-entry__icon-svg--spinner {
  animation: spin 0.9s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.sidebar-entry__label-wrap {
  flex: 1;
  min-width: 0;
  position: relative;
}

.sidebar-entry__label {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sidebar-entry:hover .sidebar-entry__label,
.sidebar-entry.is-active .sidebar-entry__label {
  padding-right: 22px;
}

.session-delete-btn {
  position: absolute;
  right: 0;
  top: 50%;
  transform: translateY(-50%);
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: #888;
  font-family: inherit;
  font-size: 14px;
  line-height: 1;
  cursor: pointer;
  opacity: 0;
  transition:
    opacity 0.12s ease,
    background 0.12s ease,
    color 0.12s ease;
}

.sidebar-entry:hover .session-delete-btn,
.sidebar-entry.is-active .session-delete-btn {
  opacity: 1;
}

.session-delete-btn:hover {
  background: rgba(0, 0, 0, 0.06);
  color: var(--sidebar-entry-text, #1a1a1e);
}
</style>
