<script setup lang="ts">
import { computed } from "vue";
import { AlertCircle, AlertTriangle, RefreshCw, Sparkles, X } from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { useI18n } from "@/i18n";
import type { CoexistenceConflict } from "@/types";

const store = useAppStore();
const { t } = useI18n();

const visibleConflicts = computed(() =>
  store.coexistenceConflicts.filter((c) => !c.dismissed),
);

function conflictTitle(conflict: CoexistenceConflict): string {
  if (conflict.kind === "systemProxy") {
    return t("banner.conflictSystemProxyTitle");
  }
  if (conflict.kind === "tunAdapter") {
    const prefix = "Active virtual network adapter:";
    const name = conflict.title.startsWith(prefix)
      ? conflict.title.slice(prefix.length).trim()
      : conflict.title;
    return t("banner.conflictTunTitle", { name });
  }
  return conflict.title;
}

function conflictDetail(conflict: CoexistenceConflict): string {
  if (conflict.kind === "systemProxy") {
    return t("banner.conflictSystemProxyDetail");
  }
  if (conflict.kind === "tunAdapter") {
    return t("banner.conflictTunDetail", { detail: conflict.detail });
  }
  return conflict.detail;
}
</script>

<template>
  <div
    v-if="(!store.mockMode && store.pipeState !== 'connected') || visibleConflicts.length > 0"
    class="mb-3 space-y-2.5"
  >
    <div
      v-if="!store.mockMode && store.pipeState !== 'connected'"
      class="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-amber-200/90 bg-amber-50/90 px-4 py-2.5 text-[12.5px] text-amber-900"
    >
      <div class="flex items-center gap-2.5">
        <AlertCircle class="h-4 w-4 shrink-0 text-amber-600" />
        <span>{{ t("banner.serviceOffline") }}</span>
      </div>
      <div class="flex items-center gap-2">
        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-lg border border-amber-300 bg-white px-2.5 py-1 text-[12px] font-medium text-amber-800 transition hover:bg-amber-100/60 dark:border-amber-500/30 dark:bg-[#252830] dark:hover:bg-[#2C303A]"
          @click="store.reconnectService()"
        >
          <RefreshCw class="h-3.5 w-3.5" />
          {{ t("banner.startService") }}
        </button>
        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-lg bg-[#0A7AFF] px-2.5 py-1 text-[12px] font-medium text-white transition hover:bg-blue-600"
          @click="store.setMockMode(true)"
        >
          <Sparkles class="h-3.5 w-3.5" />
          {{ t("banner.enableMock") }}
        </button>
      </div>
    </div>

    <div
      v-for="conflict in visibleConflicts"
      :key="conflict.id"
      class="flex items-start justify-between gap-3 rounded-xl border px-4 py-2.5 text-[12px]"
      :class="
        conflict.severity === 'high'
          ? 'border-orange-300/90 bg-orange-50/90 text-orange-900 dark:border-orange-500/35 dark:bg-orange-950/30 dark:text-orange-200'
          : 'border-amber-200/90 bg-amber-50/75 text-amber-900 dark:border-amber-500/30 dark:bg-amber-950/25 dark:text-amber-200'
      "
    >
      <div class="flex items-start gap-2.5">
        <AlertTriangle class="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
        <div>
          <div class="font-semibold">{{ conflictTitle(conflict) }}</div>
          <div class="mt-0.5 opacity-85">{{ conflictDetail(conflict) }}</div>
        </div>
      </div>
      <button
        type="button"
        :title="t('banner.dismissConflict')"
        class="rounded-md p-1 opacity-60 transition hover:bg-black/5 hover:opacity-100"
        @click="store.dismissCoexistenceConflict(conflict.id)"
      >
        <X class="h-3.5 w-3.5" />
      </button>
    </div>
  </div>
</template>
