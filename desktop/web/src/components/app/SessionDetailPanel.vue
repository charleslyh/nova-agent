<template>
  <aside class="session-detail-panel" aria-label="会话详情">
    <div v-if="sessionId" class="workspace-path">
      <span class="workspace-path-text" :title="workspacePath || undefined">会话工作区</span>
      <div class="workspace-path-actions">
        <button
          type="button"
          class="workspace-path-btn"
          :disabled="loading"
          @click="loadWorkspace"
        >
          刷新
        </button>
        <button
          type="button"
          class="workspace-path-btn"
          :disabled="!workspacePath || openingPath"
          @click="openInSystem(workspacePath)"
        >
          打开
        </button>
      </div>
    </div>

    <div class="panel-body">
      <p v-if="error" class="panel-error">{{ error }}</p>
      <p v-else-if="loading" class="panel-hint">加载中…</p>
      <ul v-else-if="visibleRows.length === 0" class="workspace-list">
        <li class="panel-hint">暂无文件</li>
      </ul>
      <ul v-else class="workspace-list">
        <li
          v-for="{ node, depth } in visibleRows"
          :key="node.path"
          class="workspace-item"
          :class="{ 'is-dir': node.is_dir }"
          :style="{ paddingLeft: `${10 + depth * 18}px` }"
          @click="node.is_dir ? toggleDirectory(node.path) : undefined"
        >
          <span class="workspace-item-main">
            <span
              v-if="node.is_dir"
              class="tree-caret"
              :class="{ expanded: expandedDirPaths.has(node.path) }"
              aria-hidden="true"
            >▶</span>
            <span class="workspace-item-name">{{ node.name }}</span>
          </span>
          <button
            type="button"
            class="open-btn workspace-item-open-btn"
            @click.stop="openInSystem(node.path)"
          >
            打开
          </button>
        </li>
      </ul>
    </div>
  </aside>
</template>

<script setup>
import { computed, ref, watch } from "vue";
import { openPath } from "@tauri-apps/plugin-opener";

const props = defineProps({
  sessionId: {
    type: String,
    default: null
  },
  open: {
    type: Boolean,
    default: false
  },
  status: {
    type: String,
    default: "idle"
  },
  fetchWorkspace: {
    type: Function,
    required: true
  }
});

const loading = ref(false);
const error = ref(null);
const workspacePath = ref("");
const entries = ref([]);
const expandedDirPaths = ref(new Set());
const openingPath = ref(false);

function flattenVisible(nodes, depth, out) {
  for (const node of nodes) {
    out.push({ node, depth });
    if (node.is_dir && expandedDirPaths.value.has(node.path) && node.children?.length) {
      flattenVisible(node.children, depth + 1, out);
    }
  }
}

const visibleRows = computed(() => {
  const rows = [];
  flattenVisible(entries.value, 0, rows);
  return rows;
});

function toggleDirectory(path) {
  const next = new Set(expandedDirPaths.value);
  if (next.has(path)) {
    next.delete(path);
  } else {
    next.add(path);
  }
  expandedDirPaths.value = next;
}

function clearWorkspaceState() {
  entries.value = [];
  workspacePath.value = "";
  expandedDirPaths.value = new Set();
  error.value = null;
  loading.value = false;
}

let workspaceLoadSeq = 0;

async function loadWorkspace() {
  if (!props.sessionId) {
    workspaceLoadSeq += 1;
    clearWorkspaceState();
    return;
  }

  const sessionId = props.sessionId;
  const seq = ++workspaceLoadSeq;
  loading.value = true;
  error.value = null;
  try {
    const result = await props.fetchWorkspace(sessionId);
    if (seq !== workspaceLoadSeq || props.sessionId !== sessionId) {
      return;
    }
    workspacePath.value = typeof result?.path === "string" ? result.path : "";
    entries.value = Array.isArray(result?.entries) ? result.entries : [];
    expandedDirPaths.value = new Set();
  } catch (e) {
    if (seq !== workspaceLoadSeq || props.sessionId !== sessionId) {
      return;
    }
    clearWorkspaceState();
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    if (seq === workspaceLoadSeq) {
      loading.value = false;
    }
  }
}

async function openInSystem(path) {
  const target = typeof path === "string" ? path.trim() : "";
  if (!target) return;

  openingPath.value = true;
  try {
    await openPath(target);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    window.alert(`无法打开：${msg}`);
  } finally {
    openingPath.value = false;
  }
}

watch(
  () => [props.open, props.sessionId],
  ([isOpen, sessionId]) => {
    if (isOpen && sessionId) {
      loadWorkspace();
    } else {
      workspaceLoadSeq += 1;
      clearWorkspaceState();
    }
  },
  { immediate: true }
);

let prevStatus = props.status;
watch(
  () => props.status,
  (next) => {
    if (prevStatus === "running" && next === "idle" && props.open && props.sessionId) {
      loadWorkspace();
    }
    prevStatus = next;
  }
);
</script>

<style scoped>
.session-detail-panel {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: #fcfcfc;
  overflow: hidden;
}

.workspace-path {
  height: 44px;
  box-sizing: border-box;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 14px;
  border-bottom: 1px solid rgba(0, 0, 0, 0.06);
  flex-shrink: 0;
  min-width: 0;
  background: #fcfcfc;
}

.workspace-path-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.workspace-path-btn {
  border: 1px solid #d5d5d9;
  border-radius: 6px;
  padding: 5px 10px;
  background-color: #fff;
  color: #4b4b4b;
  font-size: 12px;
  cursor: pointer;
  font-family: inherit;
}

.workspace-path-btn:hover:not(:disabled) {
  background-color: #f5f5f7;
}

.workspace-path-btn:disabled {
  opacity: 0.55;
  cursor: default;
}

.workspace-path-text {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  line-height: 1.4;
  color: #666;
}

.open-btn {
  flex-shrink: 0;
  border: 1px solid #d5d5d9;
  border-radius: 6px;
  padding: 6px 8px;
  background-color: #fff;
  color: #4b4b4b;
  font-size: 11px;
  cursor: pointer;
  font-family: inherit;
}

.open-btn:hover:not(:disabled) {
  background-color: #f5f5f7;
}

.open-btn:disabled {
  opacity: 0.55;
  cursor: default;
}

.panel-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 8px 0 12px;
}

.panel-error {
  margin: 0 14px;
  font-size: 13px;
  color: #c62828;
  line-height: 1.4;
}

.panel-hint {
  margin: 0 14px;
  font-size: 13px;
  color: #888;
  list-style: none;
}

.workspace-list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.workspace-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 4px 8px;
  min-height: 28px;
  padding-right: 14px;
  font-size: 13px;
  color: #333;
  cursor: default;
  user-select: none;
}

.workspace-item.is-dir {
  cursor: pointer;
}

.workspace-item:hover {
  background: rgba(0, 0, 0, 0.04);
}

.workspace-item-main {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 1;
}

.workspace-item-open-btn {
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.12s ease;
}

.workspace-item:hover .workspace-item-open-btn {
  opacity: 1;
  pointer-events: auto;
}

.tree-caret {
  display: inline-block;
  width: 12px;
  font-size: 10px;
  color: #888;
  transition: transform 0.12s ease;
  flex-shrink: 0;
}

.tree-caret.expanded {
  transform: rotate(90deg);
}

.workspace-item-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
