<template>
  <svg
    v-if="icon"
    :viewBox="icon.viewBox"
    class="sidebar-entry__icon-svg"
    :style="iconScaleStyle"
    aria-hidden="true"
  >
    <path
      v-for="(d, i) in icon.paths"
      :key="i"
      :d="d"
      :fill="icon.stroke ? 'none' : 'currentColor'"
      :stroke="icon.stroke ? 'currentColor' : undefined"
      :stroke-width="icon.stroke ? 2 : undefined"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
  </svg>
</template>

<script setup>
import { computed } from "vue";
import { channelIconForPlatform } from "@/components/icons/channelIcons.js";

const props = defineProps({
  platform: { type: String, default: "" }
});

const icon = computed(() => {
  const def = channelIconForPlatform(props.platform);
  if (!def) return null;
  return { ...def, stroke: def.viewBox !== "0 0 24 24" };
});

const iconScaleStyle = computed(() => {
  const scale = icon.value?.scale;
  if (!scale || scale === 1) return undefined;
  return { transform: `scale(${scale})` };
});
</script>
