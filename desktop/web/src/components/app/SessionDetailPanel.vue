<template>
  <aside class="session-detail-panel" aria-label="会话详情">
    <header v-if="sessionId" class="panel-head">
      <div class="panel-tabs" role="tablist" aria-label="会话详情视图">
        <button
          type="button"
          class="panel-tab-btn"
          role="tab"
          :aria-selected="activeTab === 'config'"
          :class="{ 'is-active': activeTab === 'config' }"
          @click="activeTab = 'config'"
        >
          配置
        </button>
        <button
          type="button"
          class="panel-tab-btn"
          role="tab"
          :aria-selected="activeTab === 'workspace'"
          :class="{ 'is-active': activeTab === 'workspace' }"
          @click="activeTab = 'workspace'"
        >
          工作区
        </button>
      </div>
    </header>

    <section
      v-if="sessionId && activeTab === 'config'"
      class="panel-tab panel-tab--config"
      aria-label="会话配置"
    >
      <p v-if="agentsError" class="config-message config-message--error">{{ agentsError }}</p>
      <p v-else-if="agentsLoading" class="config-message">加载 Agents…</p>
      <template v-else>
        <div class="config-body">
          <section
            v-if="channelType && channelId"
            class="config-block"
            aria-labelledby="config-block-channel-title"
          >
            <header class="config-block-header">
              <h3 id="config-block-channel-title" class="config-block-title">
                <span class="config-block-title-label">频道</span>
                <span v-if="channelTypeName" class="config-block-title-type">{{ channelTypeName }}</span>
              </h3>
            </header>
            <div class="config-block-body">
              <QQChannelConfigForm
                v-if="channelType === 'qq'"
                embedded
                hide-agent
                :channel-id="channelId"
                :session-id="sessionId"
                :is-new="false"
                :get-channel-config="getChannelConfig"
                :save-channel-config="saveChannelConfig"
                :create-channel="createChannel"
                @saved="$emit('channel-config-saved', $event)"
              />
              <WeComChannelConfigForm
                v-else-if="channelType === 'wecom'"
                embedded
                hide-agent
                :channel-id="channelId"
                :session-id="sessionId"
                :is-new="false"
                :get-channel-config="getChannelConfig"
                :save-channel-config="saveChannelConfig"
                :create-channel="createChannel"
                @saved="$emit('channel-config-saved', $event)"
              />
            </div>
          </section>

          <section class="config-block" aria-labelledby="config-block-session-title">
            <header class="config-block-header">
              <h3 id="config-block-session-title" class="config-block-title">
                <span class="config-block-title-label">会话</span>
              </h3>
            </header>
            <div class="config-block-body">
              <label class="config-field">
                <span class="config-field-label">Leader Agent</span>
                <AppSelect
                  v-model="leaderAgentId"
                  size="compact"
                  :options="leaderAgentOptions"
                  :disabled="!agents.length || agentsSaving"
                  @update:model-value="onLeaderAgentChange"
                />
              </label>

              <div
                class="config-sub-agents-field"
                role="group"
                aria-labelledby="sub-agents-label"
              >
                <span id="sub-agents-label" class="config-field-label">Sub Agents</span>
                <ul v-if="selectableSubAgents.length" class="sub-agents-picker">
              <li v-for="agent in selectableSubAgents" :key="agent.id" class="sub-agent-picker-item">
                <label class="sub-agent-chip" :title="agent.id">
                  <input
                    type="checkbox"
                    class="sub-agent-chip-check"
                    :checked="isSubAgentSelected(agent.id)"
                    :disabled="agentsSaving"
                    @change="toggleSubAgent(agent.id)"
                  />
                  <span class="sub-agent-chip-label">{{ agent.name }}</span>
                </label>
                <button
                  v-if="isSubAgentSelected(agent.id)"
                  type="button"
                  class="sub-agent-mode-btn"
                  :title="subAgentModeLabel(agent.id)"
                  :aria-label="subAgentModeLabel(agent.id)"
                  :disabled="agentsSaving"
                  @click.stop="toggleSubAgentContextMode(agent.id)"
                >
                  <span class="sub-agent-mode-btn-inner">
                    <span class="sub-agent-mode-label">{{ getSubAgentContextMode(agent.id) }}</span>
                    <!-- Lucide git-branch + top node (ISC): 三端点分支，继承 leader 上下文 -->
                    <svg
                      v-if="getSubAgentContextMode(agent.id) === 'branch'"
                      viewBox="0 0 24 24"
                      class="sub-agent-mode-icon"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      aria-hidden="true"
                    >
                      <line x1="6" x2="6" y1="9" y2="15" />
                      <circle cx="6" cy="6" r="3" />
                      <circle cx="6" cy="18" r="3" />
                      <circle cx="18" cy="6" r="3" />
                      <path d="M18 9a9 9 0 0 1-9 9" />
                    </svg>
                    <!-- Lucide box (ISC): 独立容器，使用新上下文 -->
                    <svg
                      v-else
                      viewBox="0 0 24 24"
                      class="sub-agent-mode-icon"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      aria-hidden="true"
                    >
                      <path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z" />
                      <path d="m3.3 7 8.7 5 8.7-5" />
                      <path d="M12 22V12" />
                    </svg>
                  </span>
                </button>
              </li>
                </ul>
                <p v-else class="config-empty">暂无可用 Agent</p>
              </div>
            </div>
          </section>
        </div>
      </template>
    </section>

    <section
      v-else-if="sessionId && activeTab === 'workspace'"
      class="panel-tab panel-tab--workspace"
      aria-label="会话工作区"
    >
      <div class="workspace-path">
        <span class="workspace-path-text" :title="workspacePath || undefined"></span>
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
    </section>
  </aside>
</template>

<script setup>
import { computed, ref, watch } from "vue";
import { openPath } from "@tauri-apps/plugin-opener";
import QQChannelConfigForm from "@/components/channels/QQChannelConfigForm.vue";
import WeComChannelConfigForm from "@/components/channels/WeComChannelConfigForm.vue";
import AppSelect from "@/components/form/AppSelect.vue";
import { channelTypeLabel } from "@/channelInstances.js";

defineEmits(["channel-config-saved"]);

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
  },
  agents: {
    type: Array,
    default: () => []
  },
  getSessionAgents: {
    type: Function,
    default: null
  },
  saveSessionAgents: {
    type: Function,
    default: null
  },
  channelType: {
    type: String,
    default: null
  },
  channelId: {
    type: String,
    default: null
  },
  getChannelConfig: {
    type: Function,
    default: null
  },
  saveChannelConfig: {
    type: Function,
    default: null
  },
  createChannel: {
    type: Function,
    default: null
  }
});

const channelTypeName = computed(() => channelTypeLabel(props.channelType));

const activeTab = ref("config");
const loading = ref(false);
const error = ref(null);
const workspacePath = ref("");
const entries = ref([]);
const expandedDirPaths = ref(new Set());
const openingPath = ref(false);
const agentsLoading = ref(false);
const agentsSaving = ref(false);
const agentsError = ref(null);
const leaderAgentId = ref("");
const subAgents = ref([]);

const leaderAgentOptions = computed(() =>
  props.agents.map((agent) => ({ value: agent.id, label: agent.name }))
);

const selectableSubAgents = computed(() =>
  props.agents.filter((agent) => agent.id && agent.id !== leaderAgentId.value)
);

let agentsLoadSeq = 0;
let agentsHydrating = false;
let persistInFlight = 0;

async function loadSessionAgents() {
  if (!props.sessionId || !props.getSessionAgents) {
    leaderAgentId.value = "";
    subAgents.value = [];
    agentsError.value = null;
    agentsLoading.value = false;
    return;
  }

  const sessionId = props.sessionId;
  const seq = ++agentsLoadSeq;
  agentsLoading.value = true;
  agentsError.value = null;
  agentsHydrating = true;
  try {
    const config = await props.getSessionAgents(sessionId);
    if (seq !== agentsLoadSeq || props.sessionId !== sessionId) return;
    leaderAgentId.value = config?.leader_agent_id ?? "";
    subAgents.value = Array.isArray(config?.sub_agents)
      ? config.sub_agents.map((e) => ({
          agent_id: e.agent_id,
          context_mode: e.context_mode ?? "isolated",
          description: e.description ?? ""
        }))
      : [];
  } catch (e) {
    if (seq !== agentsLoadSeq || props.sessionId !== sessionId) return;
    leaderAgentId.value = "";
    subAgents.value = [];
    agentsError.value = e?.message || "加载失败";
  } finally {
    if (seq === agentsLoadSeq) {
      agentsLoading.value = false;
      agentsHydrating = false;
    }
  }
}

function isSubAgentSelected(agentId) {
  return subAgents.value.some((entry) => entry.agent_id === agentId);
}

function subAgentDescription(agentId) {
  const agent = props.agents.find((item) => item.id === agentId);
  const name = agent?.name?.trim();
  if (name) return name;
  return agentId;
}

function toggleSubAgent(agentId) {
  if (!agentId || agentId === leaderAgentId.value || agentsSaving.value) return;

  const index = subAgents.value.findIndex((entry) => entry.agent_id === agentId);
  if (index >= 0) {
    subAgents.value.splice(index, 1);
  } else {
    subAgents.value.push({
      agent_id: agentId,
      context_mode: "isolated",
      description: subAgentDescription(agentId)
    });
  }
  void persistSessionAgents();
}

function getSubAgentContextMode(agentId) {
  const entry = subAgents.value.find((item) => item.agent_id === agentId);
  return entry?.context_mode === "branch" ? "branch" : "isolated";
}

function subAgentModeLabel(agentId) {
  return getSubAgentContextMode(agentId) === "branch"
    ? "branch：继承 leader 上下文，点击切换为 isolated"
    : "isolated：独立上下文，仅使用 task，点击切换为 branch";
}

function toggleSubAgentContextMode(agentId) {
  if (!agentId || agentsSaving.value || !isSubAgentSelected(agentId)) return;

  const entry = subAgents.value.find((item) => item.agent_id === agentId);
  if (!entry) return;

  entry.context_mode = entry.context_mode === "branch" ? "isolated" : "branch";
  void persistSessionAgents();
}

function pruneSubAgentsConflictingWithLeader() {
  const leader = leaderAgentId.value;
  if (!leader) return;
  const pruned = subAgents.value.filter((entry) => entry.agent_id !== leader);
  if (pruned.length !== subAgents.value.length) {
    subAgents.value = pruned;
  }
}

function buildSubAgentsPayload() {
  return subAgents.value
    .filter((entry) => entry.agent_id && entry.agent_id !== leaderAgentId.value)
    .map((entry) => ({
      agent_id: entry.agent_id,
      context_mode: entry.context_mode,
      description: entry.description?.trim() || subAgentDescription(entry.agent_id)
    }));
}

async function onLeaderAgentChange() {
  pruneSubAgentsConflictingWithLeader();
  await persistSessionAgents();
}

async function persistSessionAgents() {
  if (
    agentsHydrating ||
    agentsLoading.value ||
    !props.sessionId ||
    !props.saveSessionAgents ||
    !leaderAgentId.value
  ) {
    return;
  }

  const sessionId = props.sessionId;
  persistInFlight += 1;
  agentsSaving.value = true;
  agentsError.value = null;
  try {
    pruneSubAgentsConflictingWithLeader();
    await props.saveSessionAgents(sessionId, {
      leaderAgentId: leaderAgentId.value,
      subAgents: buildSubAgentsPayload()
    });
  } catch (e) {
    if (props.sessionId !== sessionId) return;
    agentsError.value = e?.message || "保存失败";
  } finally {
    persistInFlight = Math.max(0, persistInFlight - 1);
    agentsSaving.value = persistInFlight > 0;
  }
}

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
  () => props.sessionId,
  () => {
    activeTab.value = "config";
  }
);

watch(
  () => [props.open, props.sessionId, activeTab.value],
  ([isOpen, sessionId, tab]) => {
    if (isOpen && sessionId) {
      loadSessionAgents();
      if (tab === "workspace") {
        loadWorkspace();
      }
    } else {
      workspaceLoadSeq += 1;
      clearWorkspaceState();
      loadSessionAgents();
    }
  },
  { immediate: true }
);

let prevStatus = props.status;
watch(
  () => props.status,
  (next) => {
    if (
      prevStatus === "running" &&
      next === "idle" &&
      props.open &&
      props.sessionId &&
      activeTab.value === "workspace"
    ) {
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

.panel-head {
  flex-shrink: 0;
  height: 44px;
  box-sizing: border-box;
  background: #fcfcfc;
}

.panel-tabs {
  display: flex;
  width: 100%;
  height: 100%;
  box-sizing: border-box;
  border-bottom: 1px solid rgba(0, 0, 0, 0.06);
}

.panel-tab-btn {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  border: none;
  margin: 0;
  padding: 0 8px;
  font-family: inherit;
  font-size: 13px;
  font-weight: 500;
  line-height: 1.2;
  color: #6b6b76;
  background: transparent;
  cursor: pointer;
  white-space: nowrap;
  position: relative;
  transition: color 0.18s ease;
}

.panel-tab-btn:hover:not(.is-active) {
  color: #333;
}

.panel-tab-btn.is-active {
  color: #0d9ea6;
  font-weight: 600;
}

.panel-tab-btn.is-active::after {
  content: "";
  position: absolute;
  left: 16px;
  right: 16px;
  bottom: 0;
  height: 3px;
  border-radius: 3px 3px 0 0;
  background: #0d9ea6;
}

.panel-tab {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.panel-tab--config {
  overflow: hidden;
}

.config-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

.config-block {
  padding: 0;
}

.config-block + .config-block {
  border-top: 1px solid rgba(0, 0, 0, 0.08);
}

.config-block-header {
  padding: 9px 14px;
  background: #f0f0f2;
  border-bottom: 1px solid rgba(0, 0, 0, 0.06);
}

.config-block-title {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
  margin: 0;
  line-height: 1.3;
}

.config-block-title-label {
  font-size: 13px;
  font-weight: 600;
  color: #1a1a1e;
  letter-spacing: 0.01em;
}

.config-block-title-type {
  font-size: 11px;
  font-weight: 500;
  color: #5c5c66;
  padding: 2px 8px;
  border-radius: 4px;
  background: #fff;
  border: 1px solid rgba(0, 0, 0, 0.08);
  line-height: 1.35;
}

.config-block-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 14px;
}

.config-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.config-field-label {
  font-size: 12px;
  font-weight: 600;
  color: #444;
}

.config-sub-agents-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.config-message {
  margin: 14px;
  font-size: 13px;
  color: #888;
  line-height: 1.4;
}

.config-message--error {
  color: #c62828;
}

.config-empty {
  margin: 0;
  font-size: 13px;
  color: #888;
  line-height: 1.4;
}

.sub-agents-picker {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
  border: 1px solid #e5e7eb;
  border-radius: 6px;
  overflow: hidden;
  background: #e5e7eb;
}

.sub-agent-picker-item {
  display: flex;
  align-items: center;
  width: 100%;
  min-width: 0;
  background: #fff;
}

.sub-agent-picker-item:hover {
  background: #f8f9fb;
}

.sub-agent-chip {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 0;
  min-height: 32px;
  padding: 4px 0 4px 8px;
  cursor: pointer;
  user-select: none;
}

.sub-agent-mode-btn {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  margin-left: auto;
  margin-right: 3px;
  border: none;
  border-radius: 5px;
  padding: 8px 6px;
  background: transparent;
  color: #6b7280;
  cursor: pointer;
}

.sub-agent-mode-btn-inner {
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: flex-end;
  gap: 0;
}

.sub-agent-mode-btn:hover:not(:disabled) {
  background: #f3f4f6;
  color: #374151;
}

.sub-agent-mode-btn:disabled {
  opacity: 0.55;
  cursor: default;
}

.sub-agent-mode-icon {
  display: block;
  width: 18px;
  height: 18px;
}

.sub-agent-mode-label {
  max-width: 0;
  opacity: 0;
  overflow: hidden;
  white-space: nowrap;
  font-size: 10px;
  line-height: 1;
  font-weight: 500;
  letter-spacing: 0.01em;
  text-transform: uppercase;
}

.sub-agent-picker-item:hover .sub-agent-mode-label {
  max-width: 4.5rem;
  opacity: 1;
  margin-right: 5px;
}

.sub-agent-picker-item:hover .sub-agent-mode-btn:not(:disabled) {
  background: #f3f4f6;
  color: #374151;
}

.sub-agent-chip-check {
  margin: 0;
  flex-shrink: 0;
}

.sub-agent-chip-label {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  font-weight: 500;
  color: #222;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
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
