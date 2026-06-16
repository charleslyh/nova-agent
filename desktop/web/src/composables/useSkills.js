import { ref } from "vue";

/**
 * Skills UI state for Settings → Skills (local list + hub search/install).
 * @param {{ local: { list: () => Promise<{ id: string, slug?: string }[]>, detail?: (id: string) => Promise<unknown> }, hub: { search: (q: string) => Promise<{ entries?: unknown[], from_remote?: boolean }> }, install: (slug: string, opts?: { force?: boolean }) => Promise<unknown>, uninstall: (skillId: string) => Promise<unknown> }} skills
 */
export function useSkills(skills) {
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
    searchTimer = setTimeout(() => {
      runHubSearch(query.trim());
    }, query.trim() ? 300 : 0);
  }

  function loadHubRecommendations() {
    return runHubSearch("");
  }

  async function runHubSearch(query) {
    hubLoading.value = true;
    hubError.value = "";
    try {
      const body = await skills.hub.search(query);
      hubResults.value = Array.isArray(body?.entries) ? body.entries : [];
    } catch (e) {
      hubError.value = e?.message || "搜索失败";
      hubResults.value = [];
    } finally {
      hubLoading.value = false;
    }
  }

  function normalizeSkillKey(value) {
    return String(value || "")
      .trim()
      .toLowerCase()
      .replace(/[\s_]+/g, "-");
  }

  function isInstalledSlug(slug, installedSkills) {
    const hubKey = normalizeSkillKey(slug);
    if (!hubKey) return false;
    return (installedSkills || []).some((s) => {
      const keys = [s?.slug, s?.id].map(normalizeSkillKey).filter(Boolean);
      return keys.includes(hubKey);
    });
  }

  async function installFromHub(slug, { force = false, onInstalled } = {}) {
    if (!slug || installingSlug.value) return;
    installingSlug.value = slug;
    hubError.value = "";
    try {
      await skills.install(slug, { force });
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
    loadHubRecommendations,
    isInstalledSlug,
    installFromHub,
    resetHub,
    clearHubError
  };
}
