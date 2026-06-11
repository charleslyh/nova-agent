<template>
  <main class="chat-page">
    <AppSidebar
      :channel-sessions="channelSessions"
      :normal-sessions="normalSessions"
      :active-session-id="activeSessionId"
      :welcome-active="isWelcome()"
      @open-settings="openSettings"
      @pick-channel-type="startChannelCreate"
      @select-session="onSelectSession"
      @delete-session="onDeleteSession"
      @new-session="onNewSession"
    />

    <div
      class="main-and-drawer"
      :class="{ 'drawer-open': drawerOpen }"
    >
      <div class="chat-main">
        <AppTitlebar
          :show-actions="!isWelcome()"
          :show-session-detail="!!activeSessionId"
          :session-detail-open="drawerOpen"
          @reset="onResetSession"
          @toggle-session-detail="toggleDrawer"
        />

        <section class="content-layout">
        <div v-if="isWelcome()" class="welcome-prompt content-lane">
          <h1 class="welcome-prompt__title">我们聊些什么？</h1>
        </div>

        <div v-show="!isWelcome()" class="stream-column">
          <div v-if="isChannelSession" class="stream-lane">
            <ChannelSessionNotice
              :platform="activeChannelPlatform"
            />
          </div>
          <Stream
            :key="activeSessionId"
            class="stream-panel"
            :messages="transcript"
            :status="status"
            :session-dir="sessionWorkspaceDir"
            :agents="settingsCatalog.agents"
            :read-only="isChannelSession"
            read-only-hint="IM 频道"
            @tool-auth-approve="replyToolAuth($event, true)"
            @tool-auth-deny="replyToolAuth($event, false)"
          />
        </div>

        <div v-if="!isChannelSession" class="content-lane">
          <Composer
            :draft="draft"
            :status="status"
            :attachments="composerAttachments"
            @update:draft="draft = $event"
            @add-attachments="addComposerAttachments"
            @remove-attachment="removeComposerAttachment"
            @submit="submitDraft"
            @cancel="onCancelTurn"
          />
        </div>
        </section>
      </div>

      <div
        v-if="drawerOpen && activeSessionId"
        class="session-detail-drawer"
      >
        <SessionDetailPanel
          :session-id="activeSessionId"
          :open="drawerOpen"
          :status="status"
          :agents="settingsCatalog.agents"
          :fetch-workspace="getSessionWorkspace"
          :get-session-agents="getSessionAgentsConfig"
          :save-session-agents="saveSessionAgentsConfig"
          :channel-type="isChannelSession ? activeChannelPlatform : null"
          :channel-id="activeChannelId"
          :get-channel-config="getChannelConfig"
          :save-channel-config="saveChannelConfig"
          :create-channel="createChannel"
          @channel-config-saved="onChannelConfigSaved"
        />
      </div>
    </div>

    <SettingsDialog
      :open="settingsOpen"
      :settings-catalog="settingsCatalog"
      :available-tools="availableTools"
      :save-agent="saveAgentFromSettings"
      :create-agent="createAgentFromSettings"
      :delete-agent="deleteAgentFromSettings"
      :get-skill-detail="getSkillDetail"
      :search-skill-hub="searchSkillHub"
      :install-skill="installSkill"
      :refresh-skills-list="refreshSkillsList"
      :uninstall-skill="uninstallSkill"
      @close="closeSettings"
    />

    <ChannelConfigModals
      :editing="channelEditing"
      :delete-target="channelDeleteTarget"
      :deleting="channelDeleting"
      :delete-error="channelDeleteError"
      :get-channel-config="getChannelConfig"
      :save-channel-config="saveChannelConfig"
      :create-channel="createChannel"
      :get-settings-catalog="getSettingsCatalog"
      :get-session-agent="getSessionAgent"
      :set-session-agent="setSessionAgent"
      @close-edit="closeChannelEdit"
      @saved="onChannelConfigSaved"
      @cancel-delete="cancelChannelDelete"
      @confirm-delete="confirmChannelDelete"
    />

    <div
      v-if="normalSessionDeleteTarget"
      class="confirm-overlay"
      role="alertdialog"
      aria-labelledby="session-delete-title"
      aria-describedby="session-delete-desc"
      @click.stop
    >
      <div class="confirm-panel">
        <p id="session-delete-title" class="confirm-title">删除会话</p>
        <p id="session-delete-desc" class="confirm-desc">
          确定删除会话「{{ normalSessionDeleteTarget.name }}」？此操作不可恢复。
        </p>
        <p v-if="normalSessionDeleteError" class="confirm-error">{{ normalSessionDeleteError }}</p>
        <div class="confirm-actions">
          <button
            type="button"
            class="confirm-btn confirm-btn--ghost"
            :disabled="normalSessionDeleting"
            @click="cancelNormalSessionDelete"
          >
            取消
          </button>
          <button
            type="button"
            class="confirm-btn confirm-btn--danger"
            :disabled="normalSessionDeleting"
            @click="confirmNormalSessionDelete"
          >
            {{ normalSessionDeleting ? "删除中…" : "删除" }}
          </button>
        </div>
      </div>
    </div>
  </main>
</template>

<script setup>
import { onMounted } from "vue";
import SettingsDialog from "@/components/settings/SettingsDialog.vue";
import AppSidebar from "@/components/app/AppSidebar.vue";
import AppTitlebar from "@/components/app/AppTitlebar.vue";
import SessionDetailPanel from "@/components/app/SessionDetailPanel.vue";
import ChannelConfigModals from "@/components/channels/ChannelConfigModals.vue";
import ChannelSessionNotice from "@/components/chat/ChannelSessionNotice.vue";
import Composer from "@/components/chat/Composer.vue";
import Stream from "@/components/chat/Stream.vue";
import { useChatSession } from "@/composables/useChatSession";
import { useSessionDetailDrawer } from "@/composables/useSessionDetailDrawer";

const {
  transcript,
  draft,
  composerAttachments,
  addComposerAttachments,
  removeComposerAttachment,
  status,
  activeSessionId,
  sessionWorkspaceDir,
  channelSessions,
  normalSessions,
  isChannelSession,
  activeChannelPlatform,
  activeChannelId,
  channelEditing,
  channelDeleteTarget,
  channelDeleting,
  channelDeleteError,
  normalSessionDeleteTarget,
  normalSessionDeleting,
  normalSessionDeleteError,
  isWelcome,
  init,
  submitDraft,
  cancelTurn,
  resetSession,
  replyToolAuth,
  settingsOpen,
  settingsCatalog,
  availableTools,
  openSettings,
  closeSettings,
  saveAgentFromSettings,
  createAgentFromSettings,
  deleteAgentFromSettings,
  getSessionAgentsConfig,
  saveSessionAgentsConfig,
  getSkillDetail,
  searchSkillHub,
  installSkill,
  refreshSkillsList,
  uninstallSkill,
  startChannelCreate,
  closeChannelEdit,
  onChannelConfigSaved,
  cancelChannelDelete,
  confirmChannelDelete,
  cancelNormalSessionDelete,
  confirmNormalSessionDelete,
  getChannelConfig,
  saveChannelConfig,
  createChannel,
  getSettingsCatalog,
  getSessionAgent,
  setSessionAgent,
  openWelcome,
  activateSession,
  requestSessionDelete,
  getSessionWorkspace
} = useChatSession();

const { drawerOpen, toggleDrawer } = useSessionDetailDrawer();

async function onSelectSession(sessionId) {
  try {
    await activateSession(sessionId);
  } catch (e) {
    console.error(e);
  }
}

async function onDeleteSession(sessionId) {
  try {
    await requestSessionDelete(sessionId);
  } catch (e) {
    console.error(e);
  }
}

function onNewSession() {
  openWelcome();
}

async function onCancelTurn() {
  try {
    await cancelTurn();
  } catch (e) {
    console.error(e);
  }
}

async function onResetSession() {
  try {
    await resetSession();
  } catch (e) {
    console.error(e);
    const msg = e instanceof Error ? e.message : String(e);
    window.alert(`重置会话失败：${msg}`);
  }
}

onMounted(async () => {
  await init();
});
</script>

<style>
html,
body,
#app {
  margin: 0;
  padding: 0;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: #fcfcfc;
}
</style>

<style scoped>
.chat-page {
  height: 100vh;
  display: flex;
  flex-direction: row;
  padding: 0;
  box-sizing: border-box;
  color: #212121;
  font-family: -apple-system, BlinkMacSystemFont, "PingFang SC", "Helvetica Neue", sans-serif;
  overflow: hidden;
  background: #fcfcfc;
}

.main-and-drawer {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: row;
  overflow: hidden;
}

.main-and-drawer.drawer-open .chat-main {
  min-width: 0;
}

.chat-main {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.session-detail-drawer {
  width: 320px;
  flex-shrink: 0;
  min-height: 0;
  border-left: 1px solid rgba(0, 0, 0, 0.08);
  overflow: hidden;
}

.content-layout {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 8px 0 14px;
  box-sizing: border-box;
  overflow: hidden;
  background: #fcfcfc;
}

.content-lane {
  width: min(100%, 920px);
  margin: 0 auto;
  padding: 0 16px;
  box-sizing: border-box;
}

.welcome-prompt {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  text-align: center;
}

.welcome-prompt__title {
  margin: 0;
  font-size: 3.5em;
  font-weight: 600;
  letter-spacing: -0.02em;
  color: #212121;
  line-height: 1.25;
}

.stream-column {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  width: 100%;
}

.stream-lane {
  width: min(100%, 920px);
  margin: 0 auto;
  padding: 0 16px;
  box-sizing: border-box;
  flex-shrink: 0;
}

.stream-panel {
  flex: 1;
  min-height: 0;
}

.confirm-overlay {
  position: fixed;
  inset: 0;
  z-index: 2000;
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

.confirm-error {
  margin: -12px 0 16px;
  font-size: 13px;
  line-height: 1.4;
  color: #c62828;
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
</style>
