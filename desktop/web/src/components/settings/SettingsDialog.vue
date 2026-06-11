<template>
  <div v-if="open" class="backdrop" @click.self="$emit('close')">
    <div class="dialog" role="dialog" :aria-labelledby="contentTitleId" @click.stop>
      <p v-if="error" class="error">{{ error }}</p>
      <div v-else class="settings-body">
        <nav class="settings-nav" aria-label="设置分类">
          <button
            v-for="entry in navEntries"
            :key="entry.id"
            type="button"
            class="nav-item"
            :class="{ 'is-active': selectedNavId === entry.id }"
            @click="selectedNavId = entry.id"
          >
            <span class="nav-ico" aria-hidden="true">
              <svg
                v-if="entry.id === 'agents'"
                viewBox="0 0 24 24"
                class="nav-ico-svg"
                fill="none"
                stroke="currentColor"
                stroke-width="1.75"
              >
                <path
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  d="M17 21v-2a4 4 0 00-4-4H5a4 4 0 00-4 4v2M9 11a4 4 0 100-8 4 4 0 000 8zM23 21v-2a4 4 0 00-3-3.87M16 3.13a4 4 0 010 7.75"
                />
              </svg>
              <svg
                v-else-if="entry.id === 'skills'"
                viewBox="0 0 24 24"
                class="nav-ico-svg"
                fill="none"
                stroke="currentColor"
                stroke-width="1.75"
              >
                <path
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  d="M12 6.253v13m0-13C10.832 5.477 9.246 5 7.5 5S4.168 5.477 3 6.253v13C4.168 18.477 5.754 18 7.5 18s3.332.477 4.5 1.253m0-13C13.168 5.477 14.754 5 16.5 5c1.747 0 3.332.477 4.5 1.253v13C19.832 18.477 18.247 18 16.5 18c-1.746 0-3.332.477-4.5 1.253"
                />
              </svg>
            </span>
            <span class="nav-label">{{ entry.label }}</span>
          </button>
        </nav>

        <div class="settings-main">
          <header class="content-head" :class="{ 'content-head--skills': showSkillsSegment }">
            <h2
              :id="contentTitleId"
              class="content-head-title"
              :class="{ 'content-head-title--skill': inSkillDetail }"
            >
              {{ contentTitle }}
            </h2>
            <div
              v-if="showSkillsSegment"
              class="segmented-switch"
              role="tablist"
              aria-label="Skills 视图"
            >
              <button
                type="button"
                class="segmented-switch-btn"
                role="tab"
                :aria-selected="skillsSubTab === 'installed'"
                :class="{ 'is-active': skillsSubTab === 'installed' }"
                @click="skillsSubTab = 'installed'"
              >
                已安装
              </button>
              <button
                type="button"
                class="segmented-switch-btn"
                role="tab"
                :aria-selected="skillsSubTab === 'hub'"
                :class="{ 'is-active': skillsSubTab === 'hub' }"
                @click="skillsSubTab = 'hub'"
              >
                Skill Hub
              </button>
            </div>
            <button
              type="button"
              class="close-btn"
              @click="inSkillDetail ? closeSkillDetail() : $emit('close')"
            >
              {{ inSkillDetail ? "返回" : "关闭" }}
            </button>
          </header>

          <div class="content-body">
            <div v-if="selectedNavId === 'agents'" class="agents-panel">
              <p v-if="createAgentError" class="error">{{ createAgentError }}</p>
              <div class="agents-toolbar">
                <button
                  type="button"
                  class="agents-create-btn"
                  :disabled="creatingAgent"
                  @click="startCreateAgent"
                >
                  {{ creatingAgent ? "创建中…" : "新建 Agent" }}
                </button>
              </div>
              <div class="tile-row">
                <button
                  v-for="a in settingsCatalog.agents"
                  :key="a.id"
                  type="button"
                  class="agent-tile"
                  @click="openEdit(a)"
                >
                  <span class="agent-tile-name">{{ a.name }}</span>
                  <span class="agent-tile-meta">{{ completionName(a.completion_id) }}</span>
                </button>
              </div>
            </div>
            <div v-else-if="selectedNavId === 'skills'" class="skills-panel">
              <template v-if="inSkillDetail">
                <p v-if="skillDetailLoading" class="body-empty">加载中…</p>
                <p v-else-if="skillDetailError" class="error">{{ skillDetailError }}</p>
                <div v-else-if="skillDetail" class="skill-detail">
                  <dl class="skill-meta">
                    <div v-if="skillDetail.version" class="skill-meta-row">
                      <dt>版本</dt>
                      <dd>{{ skillDetail.version }}</dd>
                    </div>
                    <div v-if="skillDetail.location" class="skill-meta-row">
                      <dt>路径</dt>
                      <dd class="skill-meta-path">{{ skillDetail.location }}</dd>
                    </div>
                    <div v-if="skillDetail.always" class="skill-meta-row">
                      <dt>模式</dt>
                      <dd>始终注入完整指令</dd>
                    </div>
                  </dl>
                  <p v-if="skillDetail.description" class="skill-description">
                    {{ skillDetail.description }}
                  </p>
                  <div
                    class="skill-detail-content markdown-body"
                    v-html="renderSkillMarkdown(skillDetail.content)"
                  />
                </div>
              </template>
              <template v-else-if="skillsSubTab === 'installed'">
                <p v-if="uninstallError" class="error">{{ uninstallError }}</p>
                <p v-if="!settingsCatalog.skills?.length" class="body-empty">
                  暂无已加载的 Skills
                </p>
                <ul v-else class="skill-list" role="list">
                  <li v-for="s in settingsCatalog.skills" :key="s.id" class="skill-list-row">
                    <button type="button" class="skill-item" @click="openSkillDetail(s)">
                      <span class="skill-item-name">{{ s.id }}</span>
                      <span v-if="s.description" class="skill-item-meta">{{ s.description }}</span>
                    </button>
                    <button
                      v-if="s.removable"
                      type="button"
                      class="skill-uninstall-btn"
                      :disabled="uninstallingSkillId === s.id"
                      @click.stop="requestUninstallSkill(s)"
                    >
                      {{ uninstallingSkillId === s.id ? "卸载中…" : "卸载" }}
                    </button>
                  </li>
                </ul>
              </template>

              <template v-else>
                <div class="hub-panel">
                  <div class="hub-search">
                    <input
                      v-model="hubQuery"
                      type="search"
                      class="hub-search-input"
                      placeholder="搜索 Skill Hub…"
                    />
                  </div>
                  <p v-if="hubError" class="error hub-error">{{ hubError }}</p>
                  <p v-else-if="hubLoading" class="body-empty">加载中…</p>
                  <p
                    v-else-if="hubQuery.trim() && !hubResults.length"
                    class="body-empty"
                  >
                    无匹配结果
                  </p>
                  <p v-else-if="!hubResults.length" class="body-empty">暂无推荐</p>
                  <template v-else>
                    <p v-if="!hubQuery.trim()" class="hub-section-label">推荐</p>
                    <ul class="hub-list" role="list">
                    <li v-for="entry in hubResults" :key="entry.slug" class="hub-item">
                      <div class="hub-item-main">
                        <span class="hub-item-name">{{ entry.name || entry.slug }}</span>
                        <span v-if="entry.version" class="hub-item-version">v{{ entry.version }}</span>
                        <p v-if="entry.summary" class="hub-item-summary">{{ entry.summary }}</p>
                      </div>
                      <button
                        type="button"
                        class="hub-install-btn"
                        :class="{
                          'hub-install-btn--installed': isInstalledSlug(
                            entry.slug,
                            settingsCatalog.skills
                          )
                        }"
                        :disabled="
                          isInstalledSlug(entry.slug, settingsCatalog.skills) ||
                          installingSlug === entry.slug
                        "
                        @click="installHubSkill(entry.slug)"
                      >
                        {{
                          isInstalledSlug(entry.slug, settingsCatalog.skills)
                            ? "已安装"
                            : installingSlug === entry.slug
                              ? "安装中…"
                              : "安装"
                        }}
                      </button>
                    </li>
                    </ul>
                  </template>
                </div>
              </template>
            </div>
            <p v-else class="body-empty">暂无内容</p>
          </div>
        </div>
      </div>

      <div
        v-if="pendingUninstallSkill"
        class="confirm-overlay"
        role="alertdialog"
        aria-labelledby="uninstall-confirm-title"
        aria-describedby="uninstall-confirm-desc"
        @click.stop
      >
        <div class="confirm-panel">
          <p id="uninstall-confirm-title" class="confirm-title">卸载 Skill</p>
          <p id="uninstall-confirm-desc" class="confirm-desc">
            确定卸载 skill「{{ pendingUninstallSkill.id }}」？此操作会删除本地文件且不可恢复。
          </p>
          <div class="confirm-actions">
            <button
              type="button"
              class="confirm-btn confirm-btn--ghost"
              :disabled="!!uninstallingSkillId"
              @click="cancelUninstallConfirm"
            >
              取消
            </button>
            <button
              type="button"
              class="confirm-btn confirm-btn--danger"
              :disabled="!!uninstallingSkillId"
              @click="executeUninstallSkill"
            >
              {{ uninstallingSkillId ? "卸载中…" : "卸载" }}
            </button>
          </div>
        </div>
      </div>
    </div>

    <AgentEditDialog
      :open="editOpen"
      :agent="editingAgent"
      :completions="settingsCatalog?.completions ?? []"
      :available-tools="availableTools"
      :save-agent="saveAgent"
      :delete-agent="deleteAgent"
      @close="closeEdit"
      @deleted="closeEdit"
    />
  </div>
</template>

<script setup>
import { computed, ref, watch } from "vue";
import { renderSkillMarkdown } from "@/lib/markdown.js";
import AgentEditDialog from "@/components/settings/AgentEditDialog.vue";
import { useSkillHub } from "@/composables/useSkillHub.js";

const navEntries = [
  { id: "agents", label: "Agents" },
  { id: "skills", label: "Skills" }
];

const DEFAULT_NEW_AGENT = {
  name: "新 Agent",
  allowedTools: [],
  character: null,
  desc: null
};

const props = defineProps({
  open: { type: Boolean, required: true },
  settingsCatalog: {
    type: Object,
    required: true
  },
  availableTools: {
    type: Array,
    default: () => []
  },
  saveAgent: {
    type: Function,
    required: true
  },
  createAgent: {
    type: Function,
    required: true
  },
  deleteAgent: {
    type: Function,
    required: true
  },
  getSkillDetail: {
    type: Function,
    required: true
  },
  searchSkillHub: {
    type: Function,
    required: true
  },
  installSkill: {
    type: Function,
    required: true
  },
  refreshSkillsList: {
    type: Function,
    required: true
  },
  uninstallSkill: {
    type: Function,
    required: true
  }
});

const {
  hubQuery,
  hubResults,
  hubLoading,
  hubError,
  installingSlug,
  scheduleHubSearch,
  loadHubRecommendations,
  installFromHub,
  resetHub,
  isInstalledSlug
} = useSkillHub({
  searchSkillHub: (q) => props.searchSkillHub(q),
  installSkill: (slug, opts) => props.installSkill(slug, opts),
  listSkills: () => Promise.resolve(props.settingsCatalog.skills ?? [])
});

const emit = defineEmits(["close"]);

const contentTitleId = "settings-content-title";

const error = ref("");
const selectedNavId = ref("agents");
const editOpen = ref(false);
const editingAgent = ref(null);
const viewingSkillId = ref(null);
const skillDetail = ref(null);
const skillDetailLoading = ref(false);
const skillDetailError = ref("");
const skillsSubTab = ref("installed");
const uninstallingSkillId = ref(null);
const uninstallError = ref("");
const createAgentError = ref("");
const creatingAgent = ref(false);
const pendingUninstallSkill = ref(null);

const inSkillDetail = computed(() => viewingSkillId.value != null);

const showSkillsSegment = computed(
  () => selectedNavId.value === "skills" && !inSkillDetail.value
);

const activeNavLabel = computed(
  () => navEntries.find((e) => e.id === selectedNavId.value)?.label ?? "设置"
);

const contentTitle = computed(() => {
  if (inSkillDetail.value) return viewingSkillId.value;
  return activeNavLabel.value;
});

watch(
  () => props.open,
  (v) => {
    if (v) {
      error.value = "";
      selectedNavId.value = "agents";
    } else {
      closeEdit();
      closeSkillDetail();
      resetHub();
      skillsSubTab.value = "installed";
      uninstallError.value = "";
      uninstallingSkillId.value = null;
      pendingUninstallSkill.value = null;
    }
  }
);

watch(selectedNavId, () => {
  if (selectedNavId.value !== "skills") {
    closeSkillDetail();
    resetHub();
    skillsSubTab.value = "installed";
  }
});

watch(skillsSubTab, (tab) => {
  if (tab === "hub") {
    props.refreshSkillsList().catch(() => {});
    loadHubRecommendations().catch(() => {});
  }
});

watch(hubQuery, (q) => {
  scheduleHubSearch(q);
});

async function installHubSkill(slug) {
  try {
    await installFromHub(slug, {
      onInstalled: () => props.refreshSkillsList()
    });
  } catch {
    /* hubError set in composable */
  }
}

function requestUninstallSkill(skill) {
  if (!skill?.removable || uninstallingSkillId.value) return;
  pendingUninstallSkill.value = skill;
}

function cancelUninstallConfirm() {
  if (uninstallingSkillId.value) return;
  pendingUninstallSkill.value = null;
}

async function executeUninstallSkill() {
  const skill = pendingUninstallSkill.value;
  if (!skill?.removable || uninstallingSkillId.value) return;

  uninstallError.value = "";
  uninstallingSkillId.value = skill.id;
  try {
    await props.uninstallSkill(skill.id);
    pendingUninstallSkill.value = null;
    if (viewingSkillId.value === skill.id) {
      closeSkillDetail();
    }
    await props.refreshSkillsList();
  } catch (e) {
    uninstallError.value = e?.message || "卸载失败";
  } finally {
    uninstallingSkillId.value = null;
  }
}

async function openSkillDetail(skill) {
  viewingSkillId.value = skill.id;
  skillDetail.value = null;
  skillDetailError.value = "";
  skillDetailLoading.value = true;
  try {
    skillDetail.value = await props.getSkillDetail(skill.id);
  } catch (e) {
    skillDetailError.value = e?.message || "加载失败";
  } finally {
    skillDetailLoading.value = false;
  }
}

function closeSkillDetail() {
  viewingSkillId.value = null;
  skillDetail.value = null;
  skillDetailError.value = "";
  skillDetailLoading.value = false;
}

function completionName(id) {
  const c = props.settingsCatalog?.completions?.find((x) => x.id === id);
  return c?.name ?? id;
}

function openEdit(agent) {
  const fresh = props.settingsCatalog?.agents?.find((a) => a.id === agent.id);
  editingAgent.value = fresh ? { ...fresh } : { ...agent };
  editOpen.value = true;
}

function closeEdit() {
  editOpen.value = false;
  editingAgent.value = null;
}

async function startCreateAgent() {
  if (creatingAgent.value) return;
  const completionId = props.settingsCatalog?.completions?.[0]?.id;
  if (!completionId) {
    createAgentError.value = "未配置可用模型，无法创建 Agent";
    return;
  }
  createAgentError.value = "";
  creatingAgent.value = true;
  try {
    await props.createAgent({
      completionId,
      ...DEFAULT_NEW_AGENT
    });
  } catch (e) {
    createAgentError.value = e?.message || "创建 Agent 失败";
    console.error(e);
  } finally {
    creatingAgent.value = false;
  }
}
</script>

<style scoped>
.backdrop {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.35);
  z-index: 50;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  box-sizing: border-box;
}

.dialog {
  position: relative;
  width: min(920px, 100%);
  height: min(82vh, 720px);
  max-height: min(82vh, 720px);
  min-height: min(600px, min(82vh, 720px));
  background: #fff;
  border-radius: 12px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.18);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.confirm-overlay {
  position: absolute;
  inset: 0;
  z-index: 10;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  box-sizing: border-box;
  background: rgba(0, 0, 0, 0.35);
}

.confirm-panel {
  width: min(400px, 100%);
  padding: 20px 22px;
  border-radius: 12px;
  background: #fff;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.16);
}

.confirm-title {
  margin: 0 0 8px;
  font-size: 16px;
  font-weight: 600;
  color: #222;
}

.confirm-desc {
  margin: 0 0 20px;
  font-size: 14px;
  line-height: 1.5;
  color: #444;
}

.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.confirm-btn {
  border: none;
  border-radius: 8px;
  padding: 8px 16px;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
}

.confirm-btn:disabled {
  opacity: 0.55;
  cursor: default;
}

.confirm-btn--ghost {
  background: transparent;
  color: #444;
}

.confirm-btn--ghost:hover:not(:disabled) {
  background: #f3f3f5;
}

.confirm-btn--danger {
  color: #fff;
  background: #c62828;
}

.confirm-btn--danger:hover:not(:disabled) {
  background: #b71c1c;
}

.error {
  color: #c62828;
  padding: 12px 16px;
  margin: 0;
  font-size: 13px;
}

.settings-body {
  display: flex;
  flex-direction: row;
  flex: 1;
  min-height: 0;
}

.settings-nav {
  width: 28%;
  min-width: 168px;
  max-width: 240px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  padding: 12px 10px;
  gap: 4px;
  border-right: 1px solid rgba(0, 0, 0, 0.08);
  background: #f0f0f2;
  box-sizing: border-box;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 10px 12px;
  border: none;
  border-radius: 10px;
  background: transparent;
  color: #333;
  font-size: 14px;
  font-weight: 500;
  text-align: left;
  cursor: pointer;
  transition: background 0.12s ease;
}

.nav-item:hover {
  background: rgba(0, 0, 0, 0.05);
}

.nav-item.is-active {
  background: rgba(255, 255, 255, 0.55);
  color: #4a4a4f;
  font-weight: 600;
}

.nav-ico {
  flex-shrink: 0;
  width: 32px;
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #4a4a52;
}

.nav-ico-svg {
  width: 18px;
  height: 18px;
}

.nav-label {
  line-height: 1.2;
}

.settings-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: #fff;
}

.content-head {
  height: 44px;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  column-gap: 8px;
  padding: 0 16px;
  border-bottom: 1px solid #ececec;
  flex-shrink: 0;
}

.content-head--skills {
  grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
}

.content-head-title {
  margin: 0;
  padding: 8px 0;
  font-size: 16px;
  font-weight: 600;
  line-height: 1.25;
  color: #1a1a1e;
  min-width: 0;
  grid-column: 1;
  justify-self: start;
}

.segmented-switch {
  grid-column: 2;
  justify-self: center;
  align-self: center;
  margin: 6px 0;
  display: inline-flex;
  align-items: stretch;
  padding: 1px;
  border-radius: 6px;
  background: #ebebef;
  border: 1px solid #dcdce1;
}

.segmented-switch-btn {
  border: none;
  margin: 0;
  padding: 6px 12px;
  font-size: 12px;
  font-weight: 500;
  line-height: 1.2;
  color: #5c5c66;
  background: transparent;
  border-radius: 5px;
  cursor: pointer;
  white-space: nowrap;
  transition:
    color 0.12s ease,
    background 0.12s ease;
}

.segmented-switch-btn:hover:not(.is-active) {
  color: #333;
}

.segmented-switch-btn.is-active {
  background: #fff;
  color: #1a1a1e;
  font-weight: 600;
}

.segmented-switch-btn + .segmented-switch-btn {
  margin-left: 1px;
}

.content-head-title--skill {
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.close-btn {
  grid-column: -1;
  justify-self: end;
  align-self: center;
  border: 1px solid #d5d5d9;
  border-radius: 6px;
  padding: 4px 12px;
  background: #fff;
  font-size: 12px;
  line-height: 1.3;
  cursor: pointer;
  height: 32px;
}

.content-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 16px 18px 20px;
}

.agents-panel {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.agents-toolbar {
  display: flex;
  justify-content: flex-end;
}

.agents-create-btn {
  border: 1px solid #d8d8dc;
  background: #fff;
  border-radius: 8px;
  padding: 6px 12px;
  font-size: 13px;
  cursor: pointer;
}

.tile-row {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

@media (max-width: 640px) {
  .tile-row {
    grid-template-columns: 1fr;
  }
}

.agent-tile {
  text-align: left;
  border: 1px solid #e0e0e4;
  border-radius: 10px;
  padding: 14px 16px;
  background: #fafafa;
  cursor: pointer;
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 72px;
  transition:
    border-color 0.12s ease,
    background 0.12s ease;
}

.agent-tile:hover {
  background: #f3f3f5;
  border-color: #cfcfd4;
}

.agent-tile-name {
  font-weight: 600;
  font-size: 15px;
  color: #222;
}

.agent-tile-meta {
  font-size: 12px;
  color: #666;
}

.skill-list {
  margin: 0;
  padding: 0;
  list-style: none;
  border: 1px solid #e0e0e4;
  border-radius: 10px;
  overflow: hidden;
  background: #fafafa;
}

.skill-list > li {
  border-bottom: 1px solid #e8e8ec;
}

.skill-list > li:last-child {
  border-bottom: none;
}

.skill-list-row {
  display: flex;
  align-items: stretch;
  transition: background 0.12s ease;
}

.skill-list-row:hover {
  background: #f3f3f5;
}

.skill-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
  min-width: 0;
  padding: 12px 16px;
  border: none;
  background: transparent;
  text-align: left;
  cursor: pointer;
}

.skill-uninstall-btn {
  flex-shrink: 0;
  align-self: center;
  margin-right: 12px;
  border: 1px solid #d5d5d9;
  border-radius: 6px;
  padding: 4px 10px;
  background: transparent;
  font-size: 12px;
  line-height: 1.3;
  color: #8b2e2e;
  cursor: pointer;
}

.skill-uninstall-btn:hover:not(:disabled) {
  background: #fff5f5;
  border-color: #e0a0a0;
}

.skill-uninstall-btn:disabled {
  opacity: 0.55;
  cursor: default;
}

.skill-item-name {
  font-weight: 600;
  font-size: 14px;
  color: #222;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
}

.skill-item-meta {
  font-size: 12px;
  color: #666;
  line-height: 1.45;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.skills-panel {
  min-height: 0;
}

.hub-panel {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.hub-search-input {
  width: 100%;
  box-sizing: border-box;
  border: 1px solid #d5d5d9;
  border-radius: 8px;
  padding: 10px 12px;
  font-size: 14px;
  background-color: #fff;
}

.hub-error {
  margin: 0;
}

.hub-list {
  margin: 0;
  padding: 0;
  list-style: none;
  border: 1px solid #e0e0e4;
  border-radius: 10px;
  overflow: hidden;
}

.hub-item {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
  border-bottom: 1px solid #e8e8ec;
}

.hub-item:last-child {
  border-bottom: none;
}

.hub-item-main {
  min-width: 0;
  flex: 1;
}

.hub-item-name {
  font-weight: 600;
  font-size: 14px;
  color: #222;
}

.hub-item-version {
  margin-left: 8px;
  font-size: 12px;
  color: #888;
}

.hub-section-label {
  margin: 0 0 8px;
  font-size: 12px;
  font-weight: 600;
  color: #888;
}

.hub-item-summary {
  margin: 6px 0 0;
  font-size: 12px;
  color: #666;
  line-height: 1.45;
}

.hub-install-btn {
  flex-shrink: 0;
  border: 1px solid #4a5cff;
  border-radius: 6px;
  padding: 6px 12px;
  background: #fff;
  color: #3340cc;
  font-size: 12px;
  cursor: pointer;
}

.hub-install-btn:disabled {
  opacity: 0.55;
  cursor: default;
  border-color: #ccc;
  color: #888;
}

.hub-install-btn--installed:disabled {
  opacity: 1;
  border-color: #d0d0d0;
  background: #f5f5f5;
  color: #666;
}

.body-empty {
  margin: 0;
  font-size: 13px;
  color: #888;
}

.skill-detail {
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-height: 0;
}

.skill-meta {
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.skill-meta-row {
  display: flex;
  gap: 10px;
  font-size: 12px;
  color: #666;
}

.skill-meta-row dt {
  flex-shrink: 0;
  margin: 0;
  font-weight: 600;
  color: #888;
  min-width: 2.5em;
}

.skill-meta-row dd {
  margin: 0;
  min-width: 0;
}

.skill-meta-path {
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  word-break: break-all;
}

.skill-description {
  margin: 0;
  font-size: 13px;
  color: #555;
  line-height: 1.5;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.skill-detail-content {
  font-size: 14px;
  line-height: 1.55;
  color: #222;
  overflow-wrap: anywhere;
}

.skill-detail-content :deep(p) {
  margin: 0 0 0.75em;
}

.skill-detail-content :deep(p:last-child) {
  margin-bottom: 0;
}

.skill-detail-content :deep(code) {
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  font-size: 0.9em;
  background: #f0f0f2;
  padding: 0.1em 0.35em;
  border-radius: 4px;
}

.skill-detail-content :deep(pre) {
  margin: 0 0 1em;
  padding: 12px 14px;
  background: #f6f6f8;
  border-radius: 8px;
  overflow-x: auto;
}

.skill-detail-content :deep(pre code) {
  background: none;
  padding: 0;
}

.skill-detail-content :deep(h1),
.skill-detail-content :deep(h2),
.skill-detail-content :deep(h3) {
  margin: 1.2em 0 0.5em;
  line-height: 1.3;
}

.skill-detail-content :deep(h1:first-child),
.skill-detail-content :deep(h2:first-child),
.skill-detail-content :deep(h3:first-child) {
  margin-top: 0;
}

.skill-detail-content :deep(ul),
.skill-detail-content :deep(ol) {
  margin: 0 0 0.75em;
  padding-left: 1.4em;
}
</style>
