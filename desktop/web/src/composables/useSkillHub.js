import { ref } from "vue";

/**
 * SkillHub search/install state for Settings → Skills hub tab.
 * @param {{ searchSkillHub: (q: string) => Promise<{ entries?: unknown[], from_remote?: boolean }>, installSkill: (slug: string, opts?: { force?: boolean }) => Promise<unknown>, listSkills: () => Promise<{ id: string }[]> }} client
 */
export function useSkillHub(client) {
  const hubQuery = ref("");
  const hubResults = ref([]);
  const hubLoading = ref(false);
  const hubError = ref("");
  const installingSlug = ref(null);

  let searchTimer = null;

  function clearHubError() {
    hubError.value = "";
  }

  function scheduleHubSearch(query) {
    if (searchTimer) clearTimeout(searchTimer);
    const trimmed = query.trim();
    if (!trimmed) {
      hubResults.value = [];
      hubLoading.value = false;
      hubError.value = "";
      return;
    }
    searchTimer = setTimeout(() => {
      runHubSearch(trimmed);
    }, 300);
  }

  async function runHubSearch(query) {
    hubLoading.value = true;
    hubError.value = "";
    try {
      const body = await client.searchSkillHub(query);
      hubResults.value = Array.isArray(body?.entries) ? body.entries : [];
    } catch (e) {
      hubError.value = e?.message || "搜索失败";
      hubResults.value = [];
    } finally {
      hubLoading.value = false;
    }
  }

  function isInstalledSlug(slug, installedSkills) {
    const ids = new Set((installedSkills || []).map((s) => s.id));
    return ids.has(slug);
  }

  async function installFromHub(slug, { force = false, onInstalled } = {}) {
    if (!slug || installingSlug.value) return;
    installingSlug.value = slug;
    hubError.value = "";
    try {
      await client.installSkill(slug, { force });
      if (typeof onInstalled === "function") {
        await onInstalled();
      }
    } catch (e) {
      hubError.value = e?.message || "安装失败";
      throw e;
    } finally {
      installingSlug.value = null;
    }
  }

  function resetHub() {
    hubQuery.value = "";
    hubResults.value = [];
    hubLoading.value = false;
    hubError.value = "";
    installingSlug.value = null;
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = null;
  }

  return {
    hubQuery,
    hubResults,
    hubLoading,
    hubError,
    installingSlug,
    scheduleHubSearch,
    runHubSearch,
    isInstalledSlug,
    installFromHub,
    resetHub,
    clearHubError
  };
}
