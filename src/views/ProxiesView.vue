<script setup lang="ts">
import { computed, ref } from "vue";
import {
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  Check,
  Gauge,
  GripVertical,
  Plus,
  Search,
  Settings,
  Trash2,
} from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { useI18n } from "@/i18n";
import type { ProxyConfig, ProxyHealthSnapshot, ProxyProtocol } from "@/types";
import HelpTooltip from "@/components/HelpTooltip.vue";
import ProxyFormModal from "@/components/ProxyFormModal.vue";

const store = useAppStore();
const { t } = useI18n();
const searchQuery = ref("");
const modalOpen = ref(false);
const editingProxy = ref<ProxyConfig | null>(null);
const reordering = ref(false);
const draftOrder = ref<ProxyConfig[]>([]);
const draggingId = ref<string | null>(null);

const protocolBadgeClass: Record<ProxyProtocol, string> = {
  socks5: "bg-purple-100/90 text-purple-700 dark:bg-purple-500/20 dark:text-purple-300",
  http: "bg-sky-100/90 text-sky-700 dark:bg-sky-500/20 dark:text-sky-300",
  https: "bg-emerald-100/90 text-emerald-700 dark:bg-emerald-500/20 dark:text-emerald-300",
  directInterface: "bg-amber-100/90 text-amber-700 dark:bg-amber-500/20 dark:text-amber-300",
};

function protocolLabel(protocol: ProxyProtocol): string {
  switch (protocol) {
    case "socks5":
      return "SOCKS5";
    case "http":
      return "HTTP";
    case "https":
      return "HTTPS";
    case "directInterface":
      return t("proxies.directInterfaceLabel");
  }
}

function healthOf(proxyId: string): ProxyHealthSnapshot | null {
  return store.proxyHealthMap[proxyId] ?? null;
}

function healthDotClass(proxyId: string): string {
  const health = healthOf(proxyId);
  if (health) {
    switch (health.status) {
      case "healthy":
        return "bg-emerald-500";
      case "degraded":
        return "bg-amber-500";
      case "unreachable":
        return "bg-red-500";
      case "stale":
        return "bg-gray-400";
      case "unknown":
        break;
    }
  }
  const test = store.proxyTestResults[proxyId];
  if (test) {
    if (test.stale) return "bg-gray-400";
    return test.reachable ? "bg-emerald-500" : "bg-red-500";
  }
  return "bg-gray-400";
}

function healthText(proxyId: string): string {
  const health = healthOf(proxyId);
  if (health && health.status !== "unknown") {
    return `${health.score}%`;
  }
  const test = store.proxyTestResults[proxyId];
  if (test) {
    return test.reachable ? "100%" : "0%";
  }
  return t("proxies.untested");
}

function healthTooltip(proxyId: string): string | undefined {
  const health = healthOf(proxyId);
  const test = store.proxyTestResults[proxyId];
  if (health && (health.lastErrorCode || health.lastError)) {
    return [health.lastErrorCode, health.lastError].filter(Boolean).join(": ");
  }
  if (test?.error) return test.error;
  if (test?.stale) return t("proxies.staleTooltip");
  return undefined;
}

const filteredProxies = computed(() => {
  if (reordering.value) return draftOrder.value;
  const q = searchQuery.value.trim().toLowerCase();
  if (!q) return store.config.proxies;
  return store.config.proxies.filter(
    (p) =>
      p.name.toLowerCase().includes(q) ||
      p.host.toLowerCase().includes(q) ||
      String(p.port).includes(q),
  );
});

function enterReorder() {
  searchQuery.value = "";
  draftOrder.value = store.config.proxies.slice();
  reordering.value = true;
}

async function finishReorder() {
  const next = JSON.parse(JSON.stringify(store.config));
  next.proxies = draftOrder.value;
  reordering.value = false;
  await store.saveConfig(next);
}

function moveProxy(idx: number, delta: number) {
  const target = idx + delta;
  if (target < 0 || target >= draftOrder.value.length) return;
  const arr = draftOrder.value.slice();
  const [item] = arr.splice(idx, 1);
  if (item) {
    arr.splice(target, 0, item);
    draftOrder.value = arr;
  }
}

function onDragStart(id: string) {
  draggingId.value = id;
}

function onDropProxy(targetId: string) {
  if (!draggingId.value || draggingId.value === targetId) return;
  const fromIdx = draftOrder.value.findIndex((p) => p.id === draggingId.value);
  const toIdx = draftOrder.value.findIndex((p) => p.id === targetId);
  if (fromIdx >= 0 && toIdx >= 0) {
    const arr = draftOrder.value.slice();
    const [item] = arr.splice(fromIdx, 1);
    if (item) {
      arr.splice(toIdx, 0, item);
      draftOrder.value = arr;
    }
  }
  draggingId.value = null;
}

function openCreate() {
  editingProxy.value = null;
  modalOpen.value = true;
}

function openEdit(proxy: ProxyConfig) {
  editingProxy.value = proxy;
  modalOpen.value = true;
}

async function handleSave(proxy: ProxyConfig) {
  await store.upsertProxy(proxy);
  modalOpen.value = false;
}
</script>

<template>
  <div class="space-y-4">
    <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
      {{ t("proxies.title") }}
    </h1>

    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="flex items-center gap-2.5">
        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-full bg-gray-200/80 px-3 py-1.5 text-[12.5px] font-medium text-gray-700 transition hover:bg-gray-300/80"
          @click="store.testAllProxies()"
        >
          <Gauge class="h-3.5 w-3.5 text-gray-600" />
          {{ t("proxies.testAll") }}
        </button>

        <div v-if="!reordering" class="relative">
          <Search class="absolute left-3 top-2 h-3.5 w-3.5 text-gray-400" />
          <input
            v-model="searchQuery"
            type="text"
            :placeholder="t('proxies.searchPlaceholder')"
            class="w-44 rounded-full border border-gray-200/90 bg-white py-1.5 pl-8 pr-3 text-[12.5px] outline-none focus:border-[#0A7AFF]"
          />
        </div>
      </div>

      <div class="flex items-center gap-2">
        <button
          v-if="!reordering && store.config.proxies.length > 1"
          type="button"
          class="inline-flex h-7 items-center gap-1 rounded-lg border border-gray-200 bg-white px-2.5 text-[12px] font-medium text-gray-600 transition hover:bg-gray-50"
          @click="enterReorder"
        >
          <ArrowUpDown class="h-3.5 w-3.5" />
          {{ t("proxies.reorder") }}
        </button>
        <button
          v-else-if="reordering"
          type="button"
          class="inline-flex h-7 items-center gap-1 rounded-lg bg-[#0A7AFF] px-2.5 text-[12px] font-medium text-white transition hover:bg-blue-600"
          @click="finishReorder"
        >
          <Check class="h-3.5 w-3.5" />
          {{ t("proxies.doneReorder") }}
        </button>

        <button
          type="button"
          :title="t('proxies.addProxy')"
          class="inline-flex h-7 w-8 items-center justify-center rounded-lg border border-gray-200 bg-white text-gray-700 shadow-2xs transition hover:bg-gray-50"
          @click="openCreate"
        >
          <Plus class="h-4 w-4" />
        </button>
      </div>
    </div>

    <div
      v-if="reordering"
      class="rounded-xl border border-blue-200/80 bg-blue-50/70 px-3.5 py-2 text-[12px] text-blue-800 dark:border-blue-500/30 dark:bg-blue-950/30 dark:text-blue-200"
    >
      {{ t("proxies.reorderHint") }}
    </div>

    <!-- Table Header (responsive columns hide NIC / Auth / LAN on narrower viewports) -->
    <div
      class="grid grid-cols-[1.2fr_1.4fr_0.9fr_0.75fr_0.85fr_0.85fr] items-center gap-2 border-b border-gray-200/80 px-3 pb-2 text-[11.5px] font-medium text-gray-500 lg:grid-cols-[1.2fr_1.4fr_0.85fr_0.75fr_0.9fr_0.85fr_0.8fr] xl:grid-cols-[1.2fr_1.4fr_0.85fr_0.75fr_0.9fr_0.85fr_0.6fr_0.6fr_0.8fr]"
    >
      <span>{{ t("proxies.colName") }}</span>
      <span>{{ t("proxies.colServer") }}</span>
      <span>{{ t("proxies.colType") }}</span>
      <span>{{ t("proxies.colLatency") }}</span>
      <span class="hidden lg:block">{{ t("proxies.colInterface") }}</span>
      <span class="inline-flex items-center gap-1">
        <span>{{ t("proxies.colHealth") }}</span>
        <HelpTooltip :text="t('proxies.healthTooltip')" />
      </span>
      <span class="hidden xl:block">{{ t("proxies.colAuth") }}</span>
      <span class="hidden xl:block">{{ t("proxies.colLan") }}</span>
      <span class="text-right">{{ t("proxies.colActions") }}</span>
    </div>

    <!-- Rows -->
    <div class="space-y-1.5">
      <div
        v-for="(proxy, idx) in filteredProxies"
        :key="proxy.id"
        :draggable="reordering"
        class="grid grid-cols-[1.2fr_1.4fr_0.9fr_0.75fr_0.85fr_0.85fr] items-center gap-2 rounded-xl px-3 py-2.5 text-[12.5px] transition lg:grid-cols-[1.2fr_1.4fr_0.85fr_0.75fr_0.9fr_0.85fr_0.8fr] xl:grid-cols-[1.2fr_1.4fr_0.85fr_0.75fr_0.9fr_0.85fr_0.6fr_0.6fr_0.8fr]"
        :class="[
          idx % 2 === 0 ? 'bg-white' : 'bg-[#EDEEF2]/75',
          reordering ? 'cursor-grab ring-1 ring-blue-200/70' : '',
        ]"
        @dragstart="onDragStart(proxy.id)"
        @dragover.prevent
        @drop.prevent="onDropProxy(proxy.id)"
      >
        <div class="flex min-w-0 items-center gap-2 font-semibold text-gray-900">
          <GripVertical v-if="reordering" class="h-3.5 w-3.5 shrink-0 text-gray-400" />
          <span
            class="h-2 w-2 shrink-0 rounded-full"
            :class="proxy.enabled ? 'bg-emerald-500' : 'bg-gray-300'"
          />
          <span class="truncate">{{ proxy.name }}</span>
        </div>

        <span class="truncate font-mono text-[12px] text-gray-600">
          {{
            proxy.protocol === "directInterface"
              ? proxy.interfaceName ?? t("proxies.specifiedNic")
              : `${proxy.host}:${proxy.port}`
          }}
        </span>

        <div>
          <span
            class="inline-flex rounded-full px-2.5 py-0.5 text-[11px] font-semibold"
            :class="protocolBadgeClass[proxy.protocol]"
          >
            {{ protocolLabel(proxy.protocol) }}
          </span>
        </div>

        <span class="font-mono text-[12px]">
          <template v-if="store.testingProxyIds[proxy.id]">
            <span class="text-blue-500">{{ t("proxies.testing") }}</span>
          </template>
          <template v-else-if="store.proxyTestResults[proxy.id]?.reachable">
            <span
              class="font-medium"
              :class="
                store.proxyTestResults[proxy.id]?.stale
                  ? 'text-gray-400 line-through decoration-gray-400/70'
                  : 'text-emerald-600'
              "
            >
              {{ store.proxyTestResults[proxy.id].latencyMs }} ms
            </span>
            <span
              v-if="store.proxyTestResults[proxy.id]?.stale"
              class="ml-1 text-[10.5px] text-amber-600"
            >
              {{ t("proxies.stale") }}
            </span>
          </template>
          <template v-else-if="store.proxyTestResults[proxy.id]">
            <span
              :class="
                store.proxyTestResults[proxy.id]?.stale
                  ? 'text-gray-400'
                  : 'text-red-500'
              "
              :title="store.proxyTestResults[proxy.id]?.error ?? undefined"
            >
              {{
                store.proxyTestResults[proxy.id]?.stale
                  ? t("proxies.stale")
                  : t("proxies.unreachable")
              }}
            </span>
          </template>
          <template v-else>
            <span class="text-gray-400">—</span>
          </template>
        </span>

        <span class="hidden truncate text-gray-500 lg:block">
          {{ proxy.interfaceName ?? t("common.auto") }}
        </span>

        <div
          class="flex items-center gap-1.5 text-[12px]"
          :title="healthTooltip(proxy.id)"
        >
          <span class="h-2 w-2 rounded-full" :class="healthDotClass(proxy.id)" />
          <span class="text-gray-600">
            {{ healthText(proxy.id) }}
          </span>
        </div>

        <span class="hidden text-gray-500 xl:block">
          {{
            proxy.protocol === "directInterface"
              ? "—"
              : proxy.auth
                ? t("proxies.authCreds")
                : t("common.none")
          }}
        </span>

        <span class="hidden text-gray-500 xl:block">
          {{
            proxy.protocol === "directInterface"
              ? "—"
              : proxy.allowLan
                ? t("proxies.lanAllow")
                : t("proxies.lanDeny")
          }}
        </span>

        <div v-if="reordering" class="flex items-center justify-end gap-1">
          <button
            type="button"
            :title="t('proxies.moveUp')"
            :disabled="idx === 0"
            class="rounded-md p-1 text-gray-500 transition hover:bg-gray-100 disabled:opacity-30"
            @click="moveProxy(idx, -1)"
          >
            <ArrowUp class="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            :title="t('proxies.moveDown')"
            :disabled="idx === filteredProxies.length - 1"
            class="rounded-md p-1 text-gray-500 transition hover:bg-gray-100 disabled:opacity-30"
            @click="moveProxy(idx, 1)"
          >
            <ArrowDown class="h-3.5 w-3.5" />
          </button>
        </div>

        <div v-else class="flex items-center justify-end gap-2">
          <button
            type="button"
            :title="t('proxies.testAction')"
            class="rounded-md p-1 text-[#0A7AFF] transition hover:bg-blue-50"
            @click="store.testProxy(proxy.id)"
          >
            <Gauge class="h-4 w-4" />
          </button>
          <button
            type="button"
            :title="t('common.edit')"
            class="rounded-md p-1 text-gray-500 transition hover:bg-gray-100 hover:text-gray-800"
            @click="openEdit(proxy)"
          >
            <Settings class="h-4 w-4" />
          </button>
          <button
            type="button"
            :title="t('common.delete')"
            class="rounded-md p-1 text-red-400 transition hover:bg-red-50 hover:text-red-600"
            @click="store.deleteProxy(proxy.id)"
          >
            <Trash2 class="h-4 w-4" />
          </button>
        </div>
      </div>

      <div
        v-if="filteredProxies.length === 0"
        class="antify-card py-10 text-center text-[13px] text-gray-400"
      >
        {{ t("proxies.empty") }}
      </div>
    </div>

    <ProxyFormModal
      :open="modalOpen"
      :editing="editingProxy"
      @close="modalOpen = false"
      @save="handleSave"
    />
  </div>
</template>
