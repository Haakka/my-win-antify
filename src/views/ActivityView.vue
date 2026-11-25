<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  AlertCircle,
  AlertTriangle,
  Check,
  Copy,
  Info,
  RefreshCw,
  Search,
  Trash2,
} from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { useI18n } from "@/i18n";
import type { ActivityLogEntry, LogCategory, LogLevel } from "@/types";
import { formatDateTimeShort } from "@/utils/format";
import SelectMenu, { type SelectOption } from "@/components/SelectMenu.vue";

const store = useAppStore();
const { locale, t } = useI18n();
const searchQuery = ref("");
const levelFilter = ref<"all" | LogLevel>("all");
const copied = ref(false);

const levelOptions = computed<SelectOption[]>(() => [
  { value: "all", label: t("activity.all") },
  { value: "info", label: "Info" },
  { value: "warn", label: "Warn" },
  { value: "error", label: "Error" },
]);

onMounted(() => {
  void store.refreshActivityLogs();
});

const filteredLogs = computed(() => {
  const q = searchQuery.value.trim().toLowerCase();
  return store.activityLogs.filter((entry) => {
    if (levelFilter.value !== "all" && entry.level !== levelFilter.value) {
      return false;
    }
    if (!q) return true;
    return (
      entry.title.toLowerCase().includes(q) ||
      (entry.detail ?? "").toLowerCase().includes(q)
    );
  });
});

interface FoldedLogEntry {
  entry: ActivityLogEntry;
  count: number;
}

const foldedLogs = computed<FoldedLogEntry[]>(() => {
  const groups: FoldedLogEntry[] = [];
  for (const entry of filteredLogs.value) {
    const sec = Math.floor(entry.timestampMs / 1000);
    const prev = groups[groups.length - 1];
    if (
      prev &&
      Math.floor(prev.entry.timestampMs / 1000) === sec &&
      prev.entry.level === entry.level &&
      prev.entry.category === entry.category &&
      prev.entry.title === entry.title &&
      (prev.entry.detail ?? "") === (entry.detail ?? "")
    ) {
      prev.count += 1;
    } else {
      groups.push({ entry, count: 1 });
    }
  }
  return groups;
});

function categoryLabel(category: LogCategory): string {
  switch (category) {
    case "service":
      return t("activity.catService");
    case "proxy":
      return t("activity.catProxy");
    case "rule":
      return t("activity.catRule");
    case "app":
      return t("activity.catApp");
  }
}

const categoryClass: Record<LogCategory, string> = {
  service: "bg-purple-100/80 text-purple-700",
  proxy: "bg-sky-100/80 text-sky-700",
  rule: "bg-emerald-100/80 text-emerald-700",
  app: "bg-slate-200/80 text-slate-700",
};

async function copyLogs() {
  const lines = foldedLogs.value.map(
    ({ entry, count }) =>
      `[${formatDateTimeShort(entry.timestampMs, locale.value)}] [${entry.level.toUpperCase()}] [${categoryLabel(entry.category)}] ${entry.title}${entry.detail ? ` — ${entry.detail}` : ""}${count > 1 ? ` (×${count})` : ""}`,
  );
  try {
    await navigator.clipboard.writeText(lines.join("\n"));
    copied.value = true;
    window.setTimeout(() => {
      copied.value = false;
    }, 1500);
  } catch {
    // Clipboard unavailable.
  }
}
</script>

<template>
  <div class="space-y-4">
    <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
      {{ t("activity.title") }}
    </h1>

    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="relative w-72">
        <Search class="absolute left-3 top-2 h-3.5 w-3.5 text-gray-400" />
        <input
          v-model="searchQuery"
          type="text"
          :placeholder="t('activity.searchPlaceholder')"
          class="w-full rounded-lg border border-gray-200 bg-white py-1.5 pl-8 pr-3 text-[12.5px] outline-none focus:border-[#0A7AFF]"
        />
      </div>

      <div class="flex items-center gap-2 text-[12.5px] text-gray-600">
        <span>{{ t("activity.level") }}</span>
        <SelectMenu
          :model-value="levelFilter"
          :options="levelOptions"
          variant="pill"
          @update:model-value="levelFilter = $event as 'all' | LogLevel"
        />

        <button
          type="button"
          :title="t('activity.copyLogs')"
          class="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-gray-200 bg-white text-gray-500 transition hover:bg-gray-50 hover:text-gray-800"
          @click="copyLogs"
        >
          <Check v-if="copied" class="h-4 w-4 text-emerald-600" />
          <Copy v-else class="h-4 w-4" />
        </button>

        <button
          type="button"
          :title="t('activity.refreshLogs')"
          class="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-gray-200 bg-white text-gray-500 transition hover:bg-gray-50 hover:text-gray-800"
          @click="store.refreshActivityLogs()"
        >
          <RefreshCw class="h-4 w-4" />
        </button>

        <button
          type="button"
          :title="t('activity.clearLogs')"
          class="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-gray-200 bg-white text-gray-500 transition hover:bg-gray-50 hover:text-red-600"
          @click="store.clearActivityLogs()"
        >
          <Trash2 class="h-4 w-4" />
        </button>
      </div>
    </div>

    <div class="antify-card divide-y divide-gray-100 overflow-hidden select-text">
      <div
        v-for="item in foldedLogs"
        :key="item.entry.id"
        class="flex items-center justify-between gap-3 px-4 py-2 text-[12.5px]"
      >
        <div class="flex min-w-0 items-center gap-2.5">
          <Info
            v-if="item.entry.level === 'info'"
            class="h-4 w-4 shrink-0 text-[#0A7AFF]"
          />
          <AlertTriangle
            v-else-if="item.entry.level === 'warn'"
            class="h-4 w-4 shrink-0 text-amber-500"
          />
          <AlertCircle v-else class="h-4 w-4 shrink-0 text-red-500" />

          <span class="shrink-0 font-mono text-[11.5px] text-gray-400">
            {{ formatDateTimeShort(item.entry.timestampMs, locale) }}
          </span>

          <span
            class="shrink-0 rounded px-1.5 py-0.5 text-[10.5px] font-medium"
            :class="categoryClass[item.entry.category]"
          >
            {{ categoryLabel(item.entry.category) }}
          </span>

          <span class="truncate font-medium text-gray-900">
            {{ item.entry.title }}
          </span>

          <span
            v-if="item.count > 1"
            class="shrink-0 rounded-full bg-gray-200/80 px-1.5 py-0.5 font-mono text-[10.5px] font-semibold text-gray-700"
          >
            ×{{ item.count }}
          </span>
        </div>

        <span class="truncate text-[12px] text-gray-400">
          {{ item.entry.detail ?? "" }}
        </span>
      </div>

      <div
        v-if="foldedLogs.length === 0"
        class="py-10 text-center text-[13px] text-gray-400"
      >
        {{ t("activity.empty") }}
      </div>
    </div>
  </div>
</template>
