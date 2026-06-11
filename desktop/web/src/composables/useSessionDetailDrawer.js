import { ref } from "vue";

const drawerOpen = ref(false);

export function useSessionDetailDrawer() {
  function toggleDrawer() {
    drawerOpen.value = !drawerOpen.value;
  }

  return {
    drawerOpen,
    toggleDrawer
  };
}
