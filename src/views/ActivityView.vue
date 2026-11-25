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
import type { LogCategory, LogLevel } from "@/types";
import { formatDateTimeShort } from "@/utils/format";
import SelectMenu, { type SelectOption } from "@/components/SelectMenu.vue";

const store = useAppStore();
const searchQuery = ref("");
const levelFilter = ref<"all" | LogLevel>("all");
const copied = ref(false);

const levelOptions: SelectOption[] = [
  { value: "all", label: "全部" },
  { value: "info", label: "Info" },
  { value: "warn", label: "Warn" },
  { value: "error", label: "Error" },
];

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

const categoryLabel: Record<LogCategory, string> = {
  service: "引擎",
  proxy: "代理",
  rule: "规则",
  app: "应用",
};

const categoryClass: Record<LogCategory, string> = {
  service: "bg-purple-100/80 text-purple-700",
  proxy: "bg-sky-100/80 text-sky-700",
  rule: "bg-emerald-100/80 text-emerald-700",
  app: "bg-slate-200/80 text-slate-700",
};

async function copyLogs() {
  const lines = filteredLogs.value.map(
    (e) =>
      `[${formatDateTimeShort(e.timestampMs)}] [${e.level.toUpperCase()}] [${categoryLabel[e.category]}] ${e.title}${e.detail ? ` — ${e.detail}` : ""}`,
  );
  try {
    await navigator.clipboard.writeText(lines.join("\n"));
    copied.value = true;
    window.setTimeout(() => {
      copied.value = false;
    }, 1500);
  } catch {
    // Clipboard unavailable in restricted context.
  }
}
</script>

<template>
  <div class="space-y-4">
    <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
      活动
    </h1>

    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="relative w-72">
        <Search class="absolute left-3 top-2 h-3.5 w-3.5 text-gray-400" />
        <input
          v-model="searchQuery"
          type="text"
          placeholder="搜索日志..."
          class="w-full rounded-lg border border-gray-200 bg-white py-1.5 pl-8 pr-3 text-[12.5px] outline-none focus:border-[#0A7AFF]"
        />
      </div>

      <div class="flex items-center gap-2 text-[12.5px] text-gray-600">
        <span>级别</span>
        <SelectMenu
          :model-value="levelFilter"
          :options="levelOptions"
          variant="pill"
          @update:model-value="levelFilter = $event as 'all' | LogLevel"
        />

        <button
          type="button"
          title="复制当前日志"
          class="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-gray-200 bg-white text-gray-500 transition hover:bg-gray-50 hover:text-gray-800"
          @click="copyLogs"
        >
          <Check v-if="copied" class="h-4 w-4 text-emerald-600" />
          <Copy v-else class="h-4 w-4" />
        </button>

        <button
          type="button"
          title="刷新日志"
          class="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-gray-200 bg-white text-gray-500 transition hover:bg-gray-50 hover:text-gray-800"
          @click="store.refreshActivityLogs()"
        >
          <RefreshCw class="h-4 w-4" />
        </button>

        <button
          type="button"
          title="清空日志"
          class="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-gray-200 bg-white text-gray-500 transition hover:bg-gray-50 hover:text-red-600"
          @click="store.clearActivityLogs()"
        >
          <Trash2 class="h-4 w-4" />
        </button>
      </div>
    </div>

    <div class="antify-card divide-y divide-gray-100 overflow-hidden">
      <div
        v-for="entry in filteredLogs"
        :key="entry.id"
        class="flex items-center justify-between gap-3 px-4 py-2 text-[12.5px]"
      >
        <div class="flex min-w-0 items-center gap-2.5">
          <Info
            v-if="entry.level === 'info'"
            class="h-4 w-4 shrink-0 text-[#0A7AFF]"
          />
          <AlertTriangle
            v-else-if="entry.level === 'warn'"
            class="h-4 w-4 shrink-0 text-amber-500"
          />
          <AlertCircle v-else class="h-4 w-4 shrink-0 text-red-500" />

          <span class="shrink-0 font-mono text-[11.5px] text-gray-400">
            {{ formatDateTimeShort(entry.timestampMs) }}
          </span>

          <span
            class="shrink-0 rounded px-1.5 py-0.5 text-[10.5px] font-medium"
            :class="categoryClass[entry.category]"
          >
            {{ categoryLabel[entry.category] }}
          </span>

          <span class="truncate font-medium text-gray-900">
            {{ entry.title }}
          </span>
        </div>

        <span class="truncate text-[12px] text-gray-400">
          {{ entry.detail ?? "" }}
        </span>
      </div>

      <div
        v-if="filteredLogs.length === 0"
        class="py-10 text-center text-[13px] text-gray-400"
      >
        暂无活动日志
      </div>
    </div>
  </div>
</template>
