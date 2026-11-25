import { computed } from "vue";
import { useAppStore } from "@/stores/app";
import { zhCN } from "./zh-CN";
import { enUS } from "./en-US";

export type AppLocale = "zh-CN" | "en-US";

export function normalizeLocale(lang?: string | null): AppLocale {
  return lang?.toLowerCase().startsWith("en") ? "en-US" : "zh-CN";
}

function resolvePath(obj: Record<string, unknown>, path: string): string | undefined {
  let cur: unknown = obj;
  for (const part of path.split(".")) {
    if (!cur || typeof cur !== "object") return undefined;
    cur = (cur as Record<string, unknown>)[part];
  }
  return typeof cur === "string" ? cur : undefined;
}

export function useI18n() {
  const store = useAppStore();
  const locale = computed<AppLocale>(() =>
    normalizeLocale(store.config.settings.language),
  );
  const dict = computed(() => (locale.value === "en-US" ? enUS : zhCN));

  function t(key: string, params?: Record<string, string | number>): string {
    const raw =
      resolvePath(dict.value as unknown as Record<string, unknown>, key) ??
      resolvePath(zhCN as unknown as Record<string, unknown>, key) ??
      key;
    if (!params) return raw;
    return raw.replace(/\{(\w+)\}/g, (_, k) =>
      k in params ? String(params[k]) : `{${k}}`,
    );
  }

  return { locale, t };
}
