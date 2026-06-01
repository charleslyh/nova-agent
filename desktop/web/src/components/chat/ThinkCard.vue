<template>
  <article class="think-card">
    <button
      type="button"
      class="think-header"
      :aria-expanded="expanded ? 'true' : 'false'"
      @click="$emit('toggle')"
    >
      <span class="think-title">
        <span class="think-icon" aria-hidden="true">
          <svg viewBox="0 0 16 16" fill="none">
            <path
              d="M8 1.5a5.5 5.5 0 0 0-3.64 9.63c.34.3.54.73.54 1.2v.17h6.2v-.17c0-.47.2-.9.54-1.2A5.5 5.5 0 0 0 8 1.5z"
              stroke="currentColor"
              stroke-width="1.4"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
            <path d="M6.2 14h3.6M6.7 12.5h2.6" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
          </svg>
        </span>
        <span>{{ item.done ? "思考" : "思考中" }}</span>
      </span>
      <span class="think-meta">
        <span class="chevron" aria-hidden="true">
          <svg v-if="expanded" viewBox="0 0 16 16">
            <path d="M3.5 10.5L8 6l4.5 4.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
          <svg v-else viewBox="0 0 16 16">
            <path d="M3.5 5.5L8 10l4.5-4.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </span>
      </span>
    </button>
    <pre v-if="expanded" class="think-body">{{ item.text || "(empty)" }}</pre>
  </article>
</template>

<script setup>
defineProps({
  item: {
    type: Object,
    required: true
  },
  expanded: {
    type: Boolean,
    required: true
  }
});

defineEmits(["toggle"]);
</script>

<style scoped>
.think-card {
  width: min(72%, 760px);
  border: 1px solid #d9d9de;
  border-radius: 8px;
  background: #f4f5fb;
  padding: 10px;
  box-sizing: border-box;
}

.think-header {
  width: 100%;
  border: 0;
  background: transparent;
  padding: 0;
  margin: 0;
  cursor: pointer;
  text-align: left;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.think-title {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  color: #3f3f46;
  font-weight: 600;
}

.think-icon {
  width: 15px;
  height: 15px;
  display: inline-flex;
  color: #6366f1;
}

.think-icon svg {
  width: 100%;
  height: 100%;
}

.think-meta {
  display: inline-flex;
  align-items: center;
  gap: 0;
  color: #6b7280;
  font-size: 12px;
}

.chevron {
  width: 14px;
  height: 14px;
  display: inline-flex;
}

.chevron svg {
  width: 100%;
  height: 100%;
}

.think-body {
  margin: 9px 0 0;
  padding: 8px 10px;
  border-radius: 8px;
  background: #eceef9;
  color: #3f3f46;
  font-size: 13px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 240px;
  overflow: auto;
}
</style>
