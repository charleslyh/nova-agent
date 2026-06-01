<template>
  <div class="channel-notice" role="status">
    <p class="channel-notice__text">{{ message }}</p>
  </div>
</template>

<script setup>
import { computed } from "vue";

const props = defineProps({
  platform: {
    type: String,
    default: null
  }
});

const platformKey = computed(() => {
  const p = props.platform ?? "";
  if (p === "qq" || p === "wecom") {
    return p;
  }
  const head = p.split(":")[0];
  return head === "qq" || head === "wecom" ? head : null;
});

const message = computed(() => {
  switch (platformKey.value) {
    case "qq":
      return "此会话由 QQ 频道同步。请在 QQ 中向机器人发送消息；桌面端仅用于查看记录，无法在此输入。可使用右上角「重置会话」清空对话记录。";
    case "wecom":
      return "此会话由企业微信同步。请在企微中与机器人对话；桌面端仅用于查看记录，无法在此输入。可使用右上角「重置会话」清空对话记录。";
    default:
      return "此会话由 IM 频道同步。请在对应应用中向机器人发消息；桌面端仅用于查看记录，无法在此输入。可使用右上角「重置会话」清空对话记录。";
  }
});
</script>

<style scoped>
.channel-notice {
  flex-shrink: 0;
  margin-bottom: 8px;
  padding: 10px 14px;
  border-radius: 8px;
  background: #f3f4f6;
  border: 1px solid #e5e7eb;
}

.channel-notice__text {
  margin: 0;
  font-size: 13px;
  line-height: 1.5;
  color: #4b5563;
}
</style>
