import { computed, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauriRuntime } from "@/api/coreClient";
import { useAppStore } from "@/stores/app";
import type { ThemeMode } from "@/types";

export type SkinId = "flclash" | "indigo" | "emerald" | "amber" | "rose";

export interface SkinMeta {
  id: SkinId;
  label: string;
  preview: string;
}

export const SKINS: SkinMeta[] = [
  { id: "flclash", label: "FlClash 蓝", preview: "#2F7CF6" },
  { id: "indigo", label: "极客紫", preview: "#6E78FF" },
  { id: "emerald", label: "翡翠绿", preview: "#22C55E" },
  { id: "amber", label: "暖琥珀", preview: "#F59E0B" },
  { id: "rose", label: "绯红粉", preview: "#F43F5E" },
];

const THEME_STORAGE_KEY = "my-win-antify:theme";
const SKIN_STORAGE_KEY = "my-win-antify:skin";

function readInitialSkin(): SkinId {
  try {
    const raw = localStorage.getItem(SKIN_STORAGE_KEY);
    if (raw && SKINS.some((s) => s.id === raw)) {
      return raw as SkinId;
    }
  } catch {
    // Ignore storage access errors in restricted environments.
  }
  return "flclash";
}

function readInitialThemeMode(): ThemeMode {
  try {
    const raw = localStorage.getItem(THEME_STORAGE_KEY);
    if (raw === "light" || raw === "dark" || raw === "system") {
      return raw;
    }
  } catch {
    // Ignore storage access errors.
  }
  return "light";
}

const skin = ref<SkinId>(readInitialSkin());
const activeMode = ref<ThemeMode>(readInitialThemeMode());
const mediaQuery =
  typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-color-scheme: dark)")
    : null;
const systemPrefersDark = ref<boolean>(mediaQuery?.matches ?? false);

const resolvedTheme = computed<"light" | "dark">(() => {
  if (activeMode.value === "dark") return "dark";
  if (activeMode.value === "light") return "light";
  return systemPrefersDark.value ? "dark" : "light";
});

function syncDom(mode: ThemeMode, nextSkin: SkinId) {
  if (typeof document === "undefined") return;
  const isDark =
    mode === "dark" || (mode === "system" && systemPrefersDark.value);
  const root = document.documentElement;
  root.classList.toggle("dark", isDark);
  root.dataset.theme = isDark ? "dark" : "light";
  root.dataset.skin = nextSkin;
  if (isTauriRuntime()) {
    try {
      void getCurrentWindow().setTheme(mode === "system" ? null : mode);
    } catch {
      // Window API may be unavailable during early mount or browser preview.
    }
  }
}

let listenerBound = false;
function ensureMediaListener() {
  if (listenerBound || !mediaQuery) return;
  listenerBound = true;
  mediaQuery.addEventListener("change", (event) => {
    systemPrefersDark.value = event.matches;
    syncDom(activeMode.value, skin.value);
  });
}

export function useTheme() {
  ensureMediaListener();
  const store = useAppStore();

  function applyThemeMode(mode: ThemeMode) {
    activeMode.value = mode;
    try {
      localStorage.setItem(THEME_STORAGE_KEY, mode);
    } catch {
      // Ignore storage write errors.
    }
    syncDom(mode, skin.value);
  }

  async function setThemeMode(mode: ThemeMode) {
    applyThemeMode(mode);
    store.config.settings.theme = mode;
    if (store.isServiceConnected) {
      try {
        await store.updateSettings({ theme: mode });
      } catch {
        // Keep local theme active even if backend save hits a transient error.
      }
    }
  }

  function setSkin(nextSkin: SkinId) {
    skin.value = nextSkin;
    try {
      localStorage.setItem(SKIN_STORAGE_KEY, nextSkin);
    } catch {
      // Ignore storage write errors.
    }
    syncDom(activeMode.value, nextSkin);
  }

  return {
    skin,
    themeMode: activeMode,
    resolvedTheme,
    applyThemeMode,
    setThemeMode,
    setSkin,
  };
}
