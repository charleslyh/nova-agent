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
    <span class="sidebar-entry__label" :title="rowTitle">
      {{ rowLabel }}
    </span>
    <button
      type="button"
      class="session-delete-btn"
      :aria-label="isChannelRow ? '删除频道' : '删除会话'"
      :title="isChannelRow ? '删除频道' : '删除会话'"
      @click.stop="$emit('delete', item.sessionId)"
    >
      ×
    </button>
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
.sidebar-entry {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  box-sizing: border-box;
  padding: 10px 12px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: #333;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
}

.sidebar-entry:hover {
  background: rgba(0, 0, 0, 0.05);
}

.sidebar-entry.is-active {
  background: rgba(255, 255, 255, 0.55);
  font-weight: 600;
}

.sidebar-entry__icon {
  flex-shrink: 0;
  width: 18px;
  height: 18px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.sidebar-entry__icon-svg {
  width: 18px;
  height: 18px;
}

.sidebar-entry__icon-svg--spinner {
  animation: spin 0.9s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.sidebar-entry__label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
}

.session-delete-btn {
  flex-shrink: 0;
  width: 22px;
  height: 22px;
  padding: 0;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: #888;
  font-size: 16px;
  line-height: 1;
  cursor: pointer;
  opacity: 0;
}

.sidebar-entry:hover .session-delete-btn,
.sidebar-entry.is-active .session-delete-btn {
  opacity: 1;
}

.session-delete-btn:hover {
  background: rgba(0, 0, 0, 0.08);
  color: #333;
}
</style>
