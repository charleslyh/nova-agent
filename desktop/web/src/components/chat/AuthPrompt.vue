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

.auth-actions button {
  padding: 6px 14px;
  border-radius: 6px;
  border: 1px solid #d5d5d9;
  background-color: #fff;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
}

.auth-actions button:first-child {
  border-color: #1976ff;
  background-color: #1976ff;
  color: #fff;
}

.auth-actions button:last-child {
  color: #8b2e2e;
}

.auth-actions button:last-child:hover {
  background-color: #fff5f5;
  border-color: #e0a0a0;
}

.auth-call-id {
  color: #75757b;
}
</style>
