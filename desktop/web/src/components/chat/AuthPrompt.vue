<template>
  <section v-if="pendingAuth" class="auth-panel">
    <p>
      工具调用授权：{{ pendingAuth.toolName || "(tool)" }}
      <span class="auth-call-id">({{ pendingAuth.callId }})</span>
    </p>
    <div class="auth-actions">
      <button @click="$emit('approve')">同意</button>
      <button @click="$emit('deny')">拒绝</button>
    </div>
  </section>
</template>

<script setup>
defineProps({
  pendingAuth: {
    type: Object,
    default: null
  }
});
</script>

<style scoped>
.auth-panel {
  border: 1px solid #f0d88b;
  background: #fff6d0;
  border-radius: 12px;
  padding: 10px 12px;
  margin-bottom: 10px;
  font-size: 14px;
}

.auth-actions {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}

.auth-call-id {
  color: #75757b;
}
</style>
