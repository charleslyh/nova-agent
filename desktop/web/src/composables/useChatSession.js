import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { createHttpChatClient } from "@/chat-client";
import { previewSrcForPickerPath } from "@/lib/userImages.js";
import {
  channelTypeBySessionIdFromInstances,
  mapChannelInstance,
} from "@/channelInstances.js";

export const MAX_COMPOSER_IMAGE_ATTACHMENTS = 3;

export function useChatSession() {
  const transcript = ref([]);
  const draft = ref("");
  const composerAttachments = ref([]);
  const status = ref("idle");
  const activeSessionId = ref(null);
  const sessionWorkspaceDir = ref(null);
  const sessions = ref([]);
  const channelInstances = ref([]);
  const channelEditing = ref(null);
  const channelDeleteTarget = ref(null);
  const channelDeleting = ref(false);
  const channelDeleteError = ref("");
  const normalSessionDeleteTarget = ref(null);
  const normalSessionDeleting = ref(false);
  const normalSessionDeleteError = ref("");
  const activeThinkId = ref(null);
  const thinkCardIdsByRound = new Map();
  const currentAgentRound = ref(0);
  const toolCardsByCallId = new Map();
  const activeAssistantByStream = new Map();
  let client;
  let unsubscribeEvents = null;
  let unsubscribeSondaState = null;

  const settingsOpen = ref(false);
  const settingsCatalog = ref({ agents: [], completions: [], skills: [] });
  const availableTools = ref([]);
  const currentAgentId = ref("");

  const isWelcome = () => activeSessionId.value == null;

  function sessionNameFromInput(input) {
    const trimmed = input.trim();
    if (!trimmed) return "图片会话";
    return [...trimmed].slice(0, 16).join("");
  }

  function sessionNameFromAttachment(sourcePath) {
    const base =
      typeof sourcePath === "string"
        ? sourcePath.split(/[/\\]/).pop() || ""
        : "";
    const stem = base.replace(/\.[^.]+$/, "").trim();
    return sessionNameFromInput(stem || "图片会话");
  }

  function normalizeTurnResources(resources) {
    if (!Array.isArray(resources)) return [];
    return resources
      .map((r) => {
        if (r?.kind === "image" && typeof r.path === "string") {
          return { kind: "image", path: r.path };
        }
        return null;
      })
      .filter(Boolean);
  }

  function addComposerAttachments(paths) {
    const remaining =
      MAX_COMPOSER_IMAGE_ATTACHMENTS - composerAttachments.value.length;
    if (remaining <= 0) return;

    let added = 0;
    for (const sourcePath of paths) {
      if (added >= remaining) break;
      if (typeof sourcePath !== "string" || !sourcePath) continue;
      composerAttachments.value.push({
        id: `${Date.now()}-${Math.random()}`,
        sourcePath,
        previewUrl: previewSrcForPickerPath(sourcePath)
      });
      added += 1;
    }
  }

  function removeComposerAttachment(id) {
    composerAttachments.value = composerAttachments.value.filter((a) => a.id !== id);
  }

  function push(role, text, extra = {}) {
    const item = {
      id: `${Date.now()}-${Math.random()}`,
      role,
      text,
      ...extra
    };
    transcript.value.push(item);
    return item;
  }

  function assistantStreamKey(agentRole, agentId) {
    return `${agentRole ?? "leader"}:${agentId ?? ""}`;
  }

  function appendAssistantText(text, { agentId, agentRole } = {}) {
    if (text == null || text === "") return;
    const streamKey = assistantStreamKey(agentRole, agentId);
    let activeId = activeAssistantByStream.get(streamKey);
    if (!activeId) {
      if (!isNonEmptyString(text)) return;
      const item = push("assistant", "", { agentId, agentRole });
      activeId = item.id;
      activeAssistantByStream.set(streamKey, activeId);
    }
    const target = transcript.value.find((it) => it.id === activeId);
    if (target) {
      target.text += text;
    }
  }

  function appendAssistantError(reason) {
    const message = isNonEmptyString(reason) ? reason.trim() : "agent response failed";
    push("error", `错误：${message}`);
  }

  function isFinishedCanceled(kind) {
    if (kind === "canceled") return true;
    if (kind && typeof kind === "object" && kind.type === "canceled") return true;
    return false;
  }

  function getFinishedFailedReason(kind) {
    if (kind && typeof kind === "object") {
      const failed = kind.failed;
      if (failed && typeof failed === "object" && isNonEmptyString(failed.reason)) {
        return failed.reason.trim();
      }
      if (kind.type === "failed" && isNonEmptyString(kind.reason)) {
        return kind.reason.trim();
      }
    }
    return null;
  }

  function collapseAllThinkCardsExcept(keepId = null) {
    for (const item of transcript.value) {
      if (item.role !== "think") continue;
      item.expanded = item.id === keepId;
    }
  }

  function ensureThinkCardForCurrentRound() {
    let thinkId = thinkCardIdsByRound.get(currentAgentRound.value);
    if (!thinkId) {
      const leaderStreamId = [...activeAssistantByStream.entries()].find(([key]) =>
        key.startsWith("leader:")
      )?.[1];
      const assistantIndex = leaderStreamId
        ? transcript.value.findIndex((it) => it.id === leaderStreamId)
        : -1;
      const item = {
        id: `${Date.now()}-${Math.random()}`,
        role: "think",
        text: "",
        roundId: currentAgentRound.value,
        done: false,
        expanded: true
      };
      if (assistantIndex >= 0) {
        transcript.value.splice(assistantIndex, 0, item);
      } else {
        transcript.value.push(item);
      }
      thinkId = item.id;
      thinkCardIdsByRound.set(currentAgentRound.value, thinkId);
    }
    activeThinkId.value = thinkId;
    collapseAllThinkCardsExcept(thinkId);
    return transcript.value.find((it) => it.id === thinkId) ?? null;
  }

  function appendThinkText(text) {
    if (text == null || text === "") return;
    const card = ensureThinkCardForCurrentRound();
    if (card) {
      card.text += text;
    }
  }

  function finishThinkChunkStream() {
    if (!activeThinkId.value) return;
    const card = transcript.value.find((it) => it.id === activeThinkId.value);
    if (card) {
      card.done = true;
      card.expanded = false;
    }
    activeThinkId.value = null;
  }

  function pruneEmptyAssistantMessages() {
    transcript.value = transcript.value.filter(
      (it) => it.role !== "assistant" || isNonEmptyString(it.text)
    );
  }

  function finishAssistantChunkStream({ agentId, agentRole } = {}) {
    if (agentId != null && agentRole != null) {
      activeAssistantByStream.delete(assistantStreamKey(agentRole, agentId));
    } else {
      activeAssistantByStream.clear();
    }
    pruneEmptyAssistantMessages();
  }

  function isNonEmptyString(value) {
    return typeof value === "string" && value.trim().length > 0;
  }

  function ensureToolCard(callId, seed = {}) {
    if (!isNonEmptyString(callId)) return null;
    let item = toolCardsByCallId.get(callId);
    if (item?.id) {
      const reactiveItem = transcript.value.find((it) => it.id === item.id);
      if (reactiveItem) {
        item = reactiveItem;
        toolCardsByCallId.set(callId, reactiveItem);
      }
    }
    if (!item) {
      const rawItem = push("tool", "", {
        callId,
        toolName: seed.toolName ?? "(tool)",
        arguments: seed.arguments ?? "",
        agentId: seed.agentId,
        agentRole: seed.agentRole,
        authState: "unknown",
        authDecision: null,
        awaitAuthAction: false,
        status: "pending",
        result: ""
      });
      item = transcript.value.find((it) => it.id === rawItem.id) ?? rawItem;
      toolCardsByCallId.set(callId, item);
    } else {
      if (seed.toolName && (!item.toolName || item.toolName === "(tool)")) {
        item.toolName = seed.toolName;
      }
      if (seed.arguments && !item.arguments) {
        item.arguments = seed.arguments;
      }
      if (seed.agentId && !item.agentId) {
        item.agentId = seed.agentId;
        item.agentRole = seed.agentRole;
      }
    }
    return item;
  }

  function handleToolCallAgentEvent(agentEv, streamCtx = {}) {
    if (agentEv?.type !== "tool_call" || !agentEv.event) return;
    const ev = agentEv.event;
    const callId = ev.call_id;
    const phase = ev.type;
    if (!isNonEmptyString(callId) || !phase) return;

    switch (phase) {
      case "requested": {
        ensureToolCard(callId, {
          toolName: ev.name,
          arguments: ev.arguments ?? "",
          agentId: streamCtx.agentId,
          agentRole: streamCtx.agentRole
        });
        break;
      }
      case "extra": {
        const card = ensureToolCard(callId);
        if (card) {
          card.authState = "blocked";
          card.awaitAuthAction = true;
        }
        break;
      }
      case "started": {
        const card = ensureToolCard(callId);
        if (card) {
          card.authState = card.authState === "blocked" ? "granted" : "not_required";
          card.awaitAuthAction = false;
          card.status = "running";
        }
        break;
      }
      case "payload": {
        const card = ensureToolCard(callId);
        if (card && ev.text != null && ev.text !== "") {
          card.result = (card.result ?? "") + ev.text;
        }
        break;
      }
      case "finished": {
        const card = ensureToolCard(callId);
        if (!card) break;
        card.awaitAuthAction = false;
        const priorStatus = card.status;
        if (ev.status === "canceled") {
          card.status = "canceled";
        } else if (ev.status === "error") {
          card.status = "error";
          if (
            card.authState === "denied" ||
            card.authDecision === false ||
            card.authState === "blocked" ||
            (card.authState === "unknown" && priorStatus === "pending")
          ) {
            card.authState = "denied";
            card.authDecision = false;
          }
        } else {
          card.status = "success";
        }
        break;
      }
      default:
        break;
    }
  }

  function markInFlightToolCardsCanceled() {
    for (const card of toolCardsByCallId.values()) {
      if (!card || card.status === "success" || card.status === "error") continue;
      card.awaitAuthAction = false;
      card.status = "canceled";
    }
  }

  function clearToolCardState() {
    toolCardsByCallId.clear();
  }

  function clearThinkCardState() {
    activeThinkId.value = null;
    thinkCardIdsByRound.clear();
  }

  function clearConversationState() {
    transcript.value = [];
    draft.value = "";
    composerAttachments.value = [];
    status.value = "idle";
    finishAssistantChunkStream();
    finishThinkChunkStream();
    clearToolCardState();
    clearThinkCardState();
    currentAgentRound.value = 0;
  }

  function stopEventSubscription() {
    if (unsubscribeEvents) {
      unsubscribeEvents();
      unsubscribeEvents = null;
    }
  }

  function handleSessionEvent(event) {
    const kind = event?.kind?.type;
    if (kind === "agent_response") {
      const agentEv = event.kind.event;
      const agentId = event.kind.agent_id;
      const role = event.kind.role ?? "leader";
      const streamCtx = { agentId, agentRole: role };

      if (agentEv?.type === "started") {
        if (role === "leader") {
          currentAgentRound.value += 1;
          activeThinkId.value = null;
        }
        activeAssistantByStream.delete(assistantStreamKey(role, agentId));
      }
      if (role === "leader" && agentEv?.type === "completion_response" && agentEv.chunk?.type === "think") {
        const thinkText = agentEv.chunk.content ?? "";
        appendThinkText(thinkText);
      }
      if (role === "leader" && agentEv?.type === "completion_response" && agentEv.chunk?.type === "think_done") {
        finishThinkChunkStream();
      }
      if (
        role !== "sub" &&
        agentEv?.type === "completion_response" &&
        agentEv.chunk?.type === "text_block"
      ) {
        const text = agentEv.chunk.content ?? agentEv.chunk.text ?? "";
        appendAssistantText(text, streamCtx);
      }
      if (
        role !== "sub" &&
        agentEv?.type === "completion_response" &&
        agentEv.chunk?.type === "text_done"
      ) {
        finishAssistantChunkStream(streamCtx);
      }
      if (agentEv?.type === "completion_response" && agentEv.chunk?.type === "tool_call") {
        ensureToolCard(agentEv.chunk.call_id, {
          toolName: agentEv.chunk.name,
          arguments: agentEv.chunk.arguments,
          agentId,
          agentRole: role
        });
      }
      handleToolCallAgentEvent(agentEv, streamCtx);
      if (agentEv?.type === "finished") {
        if (role === "sub") {
          finishAssistantChunkStream(streamCtx);
        } else if (isFinishedCanceled(agentEv.kind)) {
          markInFlightToolCardsCanceled();
          push("assistant", "（已停止生成）", streamCtx);
          finishAssistantChunkStream(streamCtx);
        } else {
          const failedReason = getFinishedFailedReason(agentEv.kind);
          if (failedReason) {
            appendAssistantError(failedReason);
          }
          finishAssistantChunkStream(streamCtx);
          finishThinkChunkStream();
        }
      }
    } else if (kind === "turn_accepted") {
      status.value = "running";
      finishAssistantChunkStream();
      finishThinkChunkStream();
      clearToolCardState();
      clearThinkCardState();
      push("user", event.kind.input?.text ?? "", {
        resources: normalizeTurnResources(event.kind.input?.resources)
      });
    } else if (kind === "reset") {
      clearConversationState();
    } else if (kind === "turn_finish") {
      status.value = "idle";
      finishAssistantChunkStream();
      finishThinkChunkStream();
    } else if (!kind) {
      console.debug("unknown session event payload", event);
    }
  }

  function mapCoreEntry(entry) {
    return {
      sessionId: entry.session_id,
      name:
        typeof entry.name === "string" && entry.name ? entry.name : entry.session_id,
      workStatus: entry.work_status === "running" ? "running" : "idle",
    };
  }

  function channelSessionIdSet() {
    return new Set(channelInstances.value.map((c) => c.sessionId));
  }

  /** Platform for sidebar icon: from GET /channels, or in-flight create dialog. */
  function resolveChannelType(sessionId) {
    if (!sessionId) return null;
    const fromConfig = channelTypeBySessionIdFromInstances(channelInstances.value).get(
      sessionId
    );
    if (fromConfig) return fromConfig;
    const editing = channelEditing.value;
    if (editing?.sessionId === sessionId && editing?.type) {
      return editing.type;
    }
    return null;
  }

  const channelSessions = computed(() => {
    const byId = new Map(sessions.value.map((s) => [s.sessionId, s]));
    return channelInstances.value.map((inst) => {
      const row = byId.get(inst.sessionId);
      return {
        sessionId: inst.sessionId,
        sessionKind: "channel",
        name: row?.name ?? inst.platformName ?? inst.sessionId,
        workStatus: row?.workStatus ?? "idle",
        channelType: inst.type,
        platformName: inst.platformName,
      };
    });
  });

  const normalSessions = computed(() => {
    const channelIds = channelSessionIdSet();
    return sessions.value.filter((s) => !channelIds.has(s.sessionId));
  });

  const isChannelSession = computed(() => {
    const sid = activeSessionId.value;
    return sid != null && channelSessionIdSet().has(sid);
  });

  const activeChannelPlatform = computed(() => {
    const sid = activeSessionId.value;
    if (!sid || !isChannelSession.value) return null;
    return resolveChannelType(sid);
  });

  const activeChannelSessionIdForSettings = computed(() => {
    if (!isChannelSession.value) return null;
    return activeSessionId.value;
  });

  async function refreshChannelInstances() {
    const list = await client.listChannels();
    channelInstances.value = (list ?? [])
      .map(mapChannelInstance)
      .filter(Boolean);
  }

  function applySessionsFromCore(snapshotSessions) {
    const fromServer = (snapshotSessions ?? [])
      .filter((e) => typeof e?.session_id === "string" && e.session_id)
      .map(mapCoreEntry);
    // Catalog appends oldest-first; show newest session at the top.
    sessions.value = [...fromServer].reverse();
  }

  function handleSondaStateEvent(event) {
    if (!event || typeof event.type !== "string") return;
    switch (event.type) {
      case "snapshot":
        applySessionsFromCore(event.sessions);
        break;
      case "session_added": {
        const raw = event.session;
        if (!raw?.session_id) break;
        const item = mapCoreEntry(raw);
        if (!sessions.value.some((s) => s.sessionId === item.sessionId)) {
          sessions.value = [item, ...sessions.value];
        }
        break;
      }
      case "session_removed":
        if (event.session_id) {
          sessions.value = sessions.value.filter((s) => s.sessionId !== event.session_id);
          void refreshChannelInstances();
        }
        break;
      case "work_status_changed": {
        if (!event.session_id) break;
        const workStatus = event.work_status === "running" ? "running" : "idle";
        const row = sessions.value.find((s) => s.sessionId === event.session_id);
        if (row) row.workStatus = workStatus;
        break;
      }
      case "session_updated": {
        const raw = event.session;
        if (!raw?.session_id) break;
        const item = mapCoreEntry(raw);
        const idx = sessions.value.findIndex((s) => s.sessionId === item.sessionId);
        if (idx >= 0) {
          sessions.value[idx] = item;
        }
        break;
      }
      default:
        console.debug("unknown sonda state event", event);
    }
  }

  async function startSondaStateSubscription() {
    stopSondaStateSubscription();
    unsubscribeSondaState = await client.subscribeSondaState({
      onEvent: handleSondaStateEvent
    });
  }

  function stopSondaStateSubscription() {
    if (unsubscribeSondaState) {
      unsubscribeSondaState();
      unsubscribeSondaState = null;
    }
  }

  function defaultAgentIdFromCatalog() {
    const agents = settingsCatalog.value.agents;
    if (!agents.length) return "";
    return agents[0].id;
  }

  async function refreshAgentSelectionState(sessionId) {
    const catalog = await client.getSettingsCatalog();
    settingsCatalog.value = catalog;
    if (sessionId) {
      const cur = await client.getSessionAgent(sessionId);
      currentAgentId.value = cur.agent_id;
    } else {
      currentAgentId.value = defaultAgentIdFromCatalog();
    }
  }

  async function init() {
    client = await createHttpChatClient();
    await refreshAgentSelectionState(null);
    await refreshChannelInstances();
    await startSondaStateSubscription();
    openWelcome();
  }

  async function refreshSessionWorkspaceDir(sessionId) {
    if (!sessionId) {
      sessionWorkspaceDir.value = null;
      return;
    }
    try {
      const body = await client.getSessionWorkspacePath(sessionId);
      const path = body?.path;
      sessionWorkspaceDir.value =
        typeof path === "string" && path.length > 0 ? path : null;
    } catch {
      sessionWorkspaceDir.value = null;
    }
  }

  function openWelcome() {
    stopEventSubscription();
    activeSessionId.value = null;
    sessionWorkspaceDir.value = null;
    clearConversationState();
    currentAgentId.value = defaultAgentIdFromCatalog();
  }

  async function activateSession(sessionId) {
    if (!sessionId || sessionId === activeSessionId.value) return;
    stopEventSubscription();
    clearConversationState();
    sessionWorkspaceDir.value = null;
    activeSessionId.value = sessionId;
    await refreshSessionWorkspaceDir(sessionId);
    await refreshAgentSelectionState(sessionId);
    unsubscribeEvents = await client.subscribeEvents(sessionId, {
      fromSeq: 0,
      onEvent: (event) => handleSessionEvent(event)
    });
  }

  async function applyComposerAgentToSession(sessionId) {
    const agentId = currentAgentId.value;
    const defaultId = defaultAgentIdFromCatalog();
    if (agentId && agentId !== defaultId) {
      await client.setSessionAgent(sessionId, agentId);
      currentAgentId.value = agentId;
    }
  }

  async function createAndActivateSession(initialText) {
    const pendingAgentId = currentAgentId.value;
    const name = sessionNameFromInput(initialText);
    const sessionId = await client.createSession(name);
    await activateSession(sessionId);
    if (pendingAgentId) {
      currentAgentId.value = pendingAgentId;
      await applyComposerAgentToSession(sessionId);
    }
    return sessionId;
  }

  async function deleteSession(sessionId) {
    await client.deleteSession(sessionId);
    if (activeSessionId.value === sessionId) {
      openWelcome();
    }
  }

  function channelInstBySessionId(sessionId) {
    return channelInstances.value.find((c) => c.sessionId === sessionId) ?? null;
  }

  function channelDeleteTargetForSession(sessionId) {
    const inst = channelInstBySessionId(sessionId);
    if (!inst) return null;
    const row = channelSessions.value.find((s) => s.sessionId === sessionId);
    return {
      ...inst,
      name: row?.name ?? inst.platformName ?? inst.sessionId
    };
  }

  function normalSessionDeleteTargetForSession(sessionId) {
    const row = sessions.value.find((s) => s.sessionId === sessionId);
    if (!row) return null;
    return {
      sessionId,
      name: row.name || sessionId
    };
  }

  /** Channel rows: open confirm dialog (actual delete on second confirm). */
  function requestSessionDelete(sessionId) {
    if (channelSessionIdSet().has(sessionId)) {
      const target = channelDeleteTargetForSession(sessionId);
      if (target) {
        requestChannelDelete(target);
        return Promise.resolve();
      }
    }
    const target = normalSessionDeleteTargetForSession(sessionId);
    if (!target || normalSessionDeleting.value) {
      return Promise.resolve();
    }
    normalSessionDeleteError.value = "";
    normalSessionDeleteTarget.value = target;
    return Promise.resolve();
  }

  function cancelNormalSessionDelete() {
    if (normalSessionDeleting.value) return;
    normalSessionDeleteTarget.value = null;
    normalSessionDeleteError.value = "";
  }

  async function confirmNormalSessionDelete() {
    const target = normalSessionDeleteTarget.value;
    if (!target?.sessionId || normalSessionDeleting.value) return;
    normalSessionDeleteError.value = "";
    normalSessionDeleting.value = true;
    try {
      await deleteSession(target.sessionId);
      normalSessionDeleteTarget.value = null;
    } catch (e) {
      normalSessionDeleteError.value = e?.message || "删除失败";
    } finally {
      normalSessionDeleting.value = false;
    }
  }

  function isSessionBusyError(error) {
    const msg = error instanceof Error ? error.message : String(error);
    return msg.includes("409") || msg.toLowerCase().includes("busy");
  }

  async function cancelTurn() {
    const sessionId = activeSessionId.value;
    if (!sessionId) return;
    try {
      await client.cancelTurn(sessionId);
    } catch (error) {
      appendAssistantError(error instanceof Error ? error.message : String(error));
      status.value = "idle";
    }
  }

  async function submitDraft() {
    if (isChannelSession.value) return;
    if (status.value === "running") return;
    const text = draft.value.trim();
    const pending = [...composerAttachments.value];
    if (!text && pending.length === 0) return;

    try {
      let sessionId = activeSessionId.value;
      if (!sessionId) {
        const nameSeed = text || sessionNameFromAttachment(pending[0]?.sourcePath);
        sessionId = await createAndActivateSession(nameSeed);
      }
      const sourcePaths = pending.map((a) => a.sourcePath);
      const resources = sourcePaths.length
        ? await invoke("stage_session_images", { sessionId, sourcePaths })
        : [];
      await client.postMessage(sessionId, { text, resources });
      draft.value = "";
      composerAttachments.value = [];
      // Optimistic until SSE turn_accepted; createAndActivateSession must not leave this as idle.
      status.value = "running";
    } catch (error) {
      if (isSessionBusyError(error)) {
        status.value = "running";
      } else {
        status.value = "idle";
      }
      appendAssistantError(error instanceof Error ? error.message : String(error));
    }
  }

  async function resetSession() {
    const sessionId = activeSessionId.value;
    if (!sessionId) return;
    await client.reset(sessionId);
    clearConversationState();
  }

  async function refreshAvailableTools() {
    availableTools.value = await client.getTools();
  }

  async function refreshSkillsList() {
    const skills = await client.listSkills();
    settingsCatalog.value = {
      ...settingsCatalog.value,
      skills
    };
  }

  async function refreshSettingsDialogState(sessionId) {
    const [catalog, skills] = await Promise.all([
      client.getSettingsCatalog(),
      client.listSkills()
    ]);
    settingsCatalog.value = {
      agents: catalog.agents,
      completions: catalog.completions,
      skills
    };
    if (sessionId) {
      const cur = await client.getSessionAgent(sessionId);
      currentAgentId.value = cur.agent_id;
    } else {
      currentAgentId.value = defaultAgentIdFromCatalog();
    }
  }

  function openSettings() {
    settingsOpen.value = true;
    Promise.all([
      refreshSettingsDialogState(activeSessionId.value),
      refreshAvailableTools()
    ]).catch(() => {});
  }

  function closeSettings() {
    settingsOpen.value = false;
  }

  async function selectComposerAgent(agentId) {
    if (!agentId || agentId === currentAgentId.value) return;
    currentAgentId.value = agentId;
    const sessionId = activeSessionId.value;
    if (sessionId) {
      await client.setSessionAgent(sessionId, agentId);
    }
  }

  async function getSkillDetail(skillId) {
    return client.getSkillDetail(skillId);
  }

  async function searchSkillHub(query) {
    return client.searchSkillHub(query);
  }

  async function installSkill(slug, options) {
    return client.installSkill(slug, options);
  }

  async function uninstallSkill(skillId) {
    return client.uninstallSkill(skillId);
  }

  async function saveAgentFromSettings({ agentId, name, completionId, allowedTools, character, desc }) {
    await client.updateAgent(agentId, {
      name,
      completion_id: completionId,
      allowed_tools: allowedTools,
      character: character ?? null,
      desc: desc ?? null
    });
    await refreshAgentSelectionState(activeSessionId.value);
  }

  async function createAgentFromSettings({ name, completionId, allowedTools, character, desc }) {
    const body = await client.createAgent({
      name,
      completion_id: completionId,
      allowed_tools: allowedTools,
      character: character ?? null,
      desc: desc ?? null
    });
    await refreshAgentSelectionState(activeSessionId.value);
    return body?.id ?? null;
  }

  async function deleteAgentFromSettings(agentId) {
    await client.deleteAgent(agentId);
    await refreshAgentSelectionState(activeSessionId.value);
  }

  async function getSessionAgentsConfig(sessionId) {
    return client.getSessionAgents(sessionId);
  }

  async function saveSessionAgentsConfig(sessionId, { leaderAgentId, subAgents }) {
    await client.setSessionAgents(sessionId, {
      leader_agent_id: leaderAgentId,
      sub_agents: subAgents
    });
    if (activeSessionId.value === sessionId) {
      currentAgentId.value = leaderAgentId;
    }
  }

  function startChannelEdit(inst) {
    channelEditing.value = {
      type: inst.type,
      channelId: inst.channelId,
      sessionId: inst.sessionId,
      isNew: false
    };
  }

  function startChannelCreate(type) {
    channelEditing.value = { type, channelId: "", sessionId: "", isNew: true };
  }

  function closeChannelEdit() {
    channelEditing.value = null;
  }

  async function onChannelConfigSaved(result = {}) {
    const editing = channelEditing.value;
    const sessionId = editing?.sessionId;
    const wasNew = editing?.isNew;
    const channelChanged = result?.channelChanged !== false;
    closeChannelEdit();
    if (!channelChanged && !wasNew) {
      if (sessionId) {
        await activateSession(sessionId);
      }
      return;
    }
    await refreshChannelInstances();
    if (wasNew) {
      const latest = channelInstances.value[channelInstances.value.length - 1];
      if (latest?.sessionId) {
        await activateSession(latest.sessionId);
      }
    } else if (sessionId) {
      await activateSession(sessionId);
    }
  }

  function requestChannelDelete(target) {
    if (!target?.channelId || channelDeleting.value) return;
    channelDeleteError.value = "";
    channelDeleteTarget.value = target;
  }

  function cancelChannelDelete() {
    if (channelDeleting.value) return;
    channelDeleteTarget.value = null;
    channelDeleteError.value = "";
  }

  async function confirmChannelDelete() {
    const target = channelDeleteTarget.value;
    if (!target?.channelId || channelDeleting.value) return;

    channelDeleteError.value = "";
    channelDeleting.value = true;
    try {
      const { channelId, sessionId } = target;
      await client.deleteChannelConfig(channelId);
      if (activeSessionId.value === sessionId) {
        openWelcome();
      }
      if (channelEditing.value?.channelId === channelId) {
        closeChannelEdit();
      }
      channelDeleteTarget.value = null;
      await refreshChannelInstances();
    } catch (e) {
      channelDeleteError.value = e?.message || "删除失败";
    } finally {
      channelDeleting.value = false;
    }
  }

  function openChannelSettings() {
    const sessionId = activeChannelSessionIdForSettings.value;
    if (!sessionId) return;
    const inst = channelInstances.value.find((c) => c.sessionId === sessionId);
    if (inst) {
      startChannelEdit(inst);
      return;
    }
    const platform = activeChannelPlatform.value;
    if (platform === "qq" || platform === "wecom") {
      channelEditing.value = { type: platform, sessionId, isNew: false };
    }
  }

  async function replyToolAuth(callId, decision) {
    if (!isNonEmptyString(callId)) return;
    await client.replyToolAuth(callId, decision);
    const card = ensureToolCard(callId);
    if (card) {
      card.authDecision = decision;
      card.authState = decision ? "granted" : "denied";
      card.awaitAuthAction = false;
      if (!decision && card.status === "pending") {
        card.status = "error";
      }
    }
  }

  return {
    transcript,
    draft,
    composerAttachments,
    addComposerAttachments,
    removeComposerAttachment,
    status,
    activeSessionId,
    sessionWorkspaceDir,
    sessions,
    channelSessions,
    normalSessions,
    isChannelSession,
    activeChannelPlatform,
    activeChannelSessionIdForSettings,
    channelEditing,
    channelDeleteTarget,
    channelDeleting,
    channelDeleteError,
    normalSessionDeleteTarget,
    normalSessionDeleting,
    normalSessionDeleteError,
    isWelcome,
    init,
    openWelcome,
    startChannelCreate,
    closeChannelEdit,
    onChannelConfigSaved,
    requestChannelDelete,
    cancelChannelDelete,
    confirmChannelDelete,
    openChannelSettings,
    activateSession,
    createAndActivateSession,
    deleteSession,
    requestSessionDelete,
    cancelNormalSessionDelete,
    confirmNormalSessionDelete,
    submitDraft,
    cancelTurn,
    resetSession,
    replyToolAuth,
    settingsOpen,
    settingsCatalog,
    availableTools,
    currentAgentId,
    openSettings,
    closeSettings,
    selectComposerAgent,
    saveAgentFromSettings,
    createAgentFromSettings,
    deleteAgentFromSettings,
    getSessionAgentsConfig,
    saveSessionAgentsConfig,
    getSkillDetail,
    searchSkillHub,
    installSkill,
    uninstallSkill,
    refreshSkillsList,
    getChannelConfig: (channelId) => client.getChannelConfig(channelId),
    saveChannelConfig: (channelId, payload) => client.saveChannelConfig(channelId, payload),
    createChannel: (payload) => client.createChannel(payload),
    getSettingsCatalog: () => client.getSettingsCatalog(),
    getSessionAgent: (sessionId) => client.getSessionAgent(sessionId),
    setSessionAgent: (sessionId, agentId) => client.setSessionAgent(sessionId, agentId),
    getSessionWorkspace: (sessionId) => client.getSessionWorkspace(sessionId),
    getSessionWorkspacePath: (sessionId) => client.getSessionWorkspacePath(sessionId)
  };
}
