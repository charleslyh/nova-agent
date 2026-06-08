import { invoke } from "@tauri-apps/api/core";

export function createChatClient(impl) {
  return {
    listSessions: impl.listSessions,
    createSession: impl.createSession,
    deleteSession: impl.deleteSession,
    postMessage: impl.postMessage,
    cancelTurn: impl.cancelTurn,
    subscribeEvents: impl.subscribeEvents,
    subscribeSondaState: impl.subscribeSondaState,
    replyToolAuth: impl.replyToolAuth,
    reset: impl.reset,
    getSettingsCatalog: impl.getSettingsCatalog,
    listSkills: impl.listSkills,
    getSkillDetail: impl.getSkillDetail,
    searchSkillHub: impl.searchSkillHub,
    installSkill: impl.installSkill,
    uninstallSkill: impl.uninstallSkill,
    getTools: impl.getTools,
    getSessionAgent: impl.getSessionAgent,
    setSessionAgent: impl.setSessionAgent,
    getSessionWorkspace: impl.getSessionWorkspace,
    getSessionWorkspacePath: impl.getSessionWorkspacePath,
    updateAgent: impl.updateAgent,
    listChannels: impl.listChannels,
    getChannelConfig: impl.getChannelConfig,
    saveChannelConfig: impl.saveChannelConfig,
    createChannel: impl.createChannel,
    deleteChannelConfig: impl.deleteChannelConfig
  };
}

export async function createHttpChatClient() {
  const baseUrl = await invoke("get_server_url");
  const channelPath = (channelId) =>
    `${baseUrl}/channels/${encodeURIComponent(channelId)}`;

  return createChatClient({
    async listSessions() {
      const list = await request(`${baseUrl}/sessions`, { method: "GET" });
      if (!Array.isArray(list)) {
        throw new Error("invalid sessions list payload");
      }
      return list;
    },

    async createSession(name) {
      const body = await request(`${baseUrl}/sessions`, {
        method: "POST",
        body: JSON.stringify({ name })
      });
      const sessionId = body?.session_id;
      if (typeof sessionId !== "string" || !sessionId) {
        throw new Error("invalid create session payload");
      }
      return sessionId;
    },

    async deleteSession(sessionId) {
      const sid = encodeURIComponent(sessionId);
      await request(`${baseUrl}/sessions/${sid}`, { method: "DELETE" });
    },

    async postMessage(sessionId, input) {
      const sessionBase = `${baseUrl}/sessions/${encodeURIComponent(sessionId)}`;
      const body =
        typeof input === "string"
          ? { text: input, resources: [] }
          : {
              text: input?.text ?? "",
              resources: Array.isArray(input?.resources) ? input.resources : []
            };
      await request(`${sessionBase}/submit`, {
        method: "POST",
        body: JSON.stringify(body)
      });
    },

    async cancelTurn(sessionId) {
      const sessionBase = `${baseUrl}/sessions/${encodeURIComponent(sessionId)}`;
      await request(`${sessionBase}/cancel`, { method: "POST" });
    },

    async subscribeEvents(sessionId, { fromSeq, onEvent }) {
      const sessionBase = `${baseUrl}/sessions/${encodeURIComponent(sessionId)}`;
      const source = new EventSource(
        `${sessionBase}/events?from_seq=${fromSeq ?? 0}`
      );
      source.addEventListener("session", (event) => onEvent(JSON.parse(event.data)));
      return () => source.close();
    },

    async subscribeSondaState({ onEvent }) {
      const source = new EventSource(`${baseUrl}/sonda/events`);
      source.addEventListener("sonda", (event) => onEvent(JSON.parse(event.data)));
      return () => source.close();
    },

    async replyToolAuth(callId, data) {
      await request(`${baseUrl}/tool-auth/${encodeURIComponent(callId)}`, {
        method: "POST",
        body: JSON.stringify(data)
      });
    },

    async reset(sessionId) {
      const sessionBase = `${baseUrl}/sessions/${encodeURIComponent(sessionId)}`;
      await request(`${sessionBase}/reset`, { method: "POST" });
    },

    async getSettingsCatalog() {
      return request(`${baseUrl}/settings/catalog`, { method: "GET" });
    },

    async listSkills() {
      const body = await request(`${baseUrl}/skills`, { method: "GET" });
      return Array.isArray(body?.skills) ? body.skills : [];
    },

    async getSkillDetail(skillId) {
      const sid = encodeURIComponent(skillId);
      return request(`${baseUrl}/skills/${sid}`, { method: "GET" });
    },

    async searchSkillHub(query) {
      const trimmed = String(query ?? "").trim();
      const url = trimmed
        ? `${baseUrl}/skills/search?q=${encodeURIComponent(trimmed)}`
        : `${baseUrl}/skills/search`;
      return request(url, { method: "GET" });
    },

    async installSkill(slug, { force = false } = {}) {
      return request(`${baseUrl}/skills/install`, {
        method: "POST",
        body: JSON.stringify({ slug, force })
      });
    },

    async uninstallSkill(skillId) {
      const sid = encodeURIComponent(skillId);
      return request(`${baseUrl}/skills/${sid}`, { method: "DELETE" });
    },

    async getTools() {
      const body = await request(`${baseUrl}/tools`, { method: "GET" });
      return Array.isArray(body?.tools) ? body.tools : [];
    },

    async getSessionAgent(sessionId) {
      const sid = encodeURIComponent(sessionId);
      return request(`${baseUrl}/sessions/${sid}/agent`, { method: "GET" });
    },

    async setSessionAgent(sessionId, agentId) {
      const sid = encodeURIComponent(sessionId);
      await request(`${baseUrl}/sessions/${sid}/agent`, {
        method: "PUT",
        body: JSON.stringify({ agent_id: agentId })
      });
    },

    async getSessionWorkspace(sessionId) {
      const sid = encodeURIComponent(sessionId);
      return request(`${baseUrl}/sessions/${sid}/workspace`, { method: "GET" });
    },

    async getSessionWorkspacePath(sessionId) {
      const sid = encodeURIComponent(sessionId);
      return request(`${baseUrl}/sessions/${sid}/workspace/path`, { method: "GET" });
    },

    async updateAgent(agentId, { name, completion_id: completionId, allowed_tools: allowedTools, character }) {
      const aid = encodeURIComponent(agentId);
      await request(`${baseUrl}/settings/agents/${aid}`, {
        method: "PATCH",
        body: JSON.stringify({
          name,
          completion_id: completionId,
          allowed_tools: allowedTools ?? [],
          character: character ?? null
        })
      });
    },

    async listChannels() {
      const list = await request(`${baseUrl}/channels`, { method: "GET" });
      return Array.isArray(list) ? list : [];
    },

    async getChannelConfig(channelId) {
      return request(channelPath(channelId), { method: "GET" });
    },

    async saveChannelConfig(channelId, { data }) {
      await request(channelPath(channelId), {
        method: "PUT",
        body: JSON.stringify({ data })
      });
    },

    async createChannel({ name, type, data }) {
      const body = await request(`${baseUrl}/channels`, {
        method: "POST",
        body: JSON.stringify({ name, type, data })
      });
      if (!body?.channel_id || !body?.session_id) {
        throw new Error("invalid create channel payload");
      }
      return body;
    },

    async deleteChannelConfig(channelId) {
      await request(channelPath(channelId), { method: "DELETE" });
    }
  });
}

async function request(url, init = {}) {
  const headers = { ...(init.headers || {}) };
  if (init.body != null) {
    headers["content-type"] = "application/json";
  }
  const res = await fetch(url, {
    ...init,
    headers
  });
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.message || `request failed: ${res.status}`);
  }
  if (res.status === 204) {
    return null;
  }
  const ct = res.headers.get("content-type") || "";
  if (ct.includes("application/json")) {
    return res.json();
  }
  return null;
}
