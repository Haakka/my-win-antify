<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { Info, Plus, RefreshCw, Search, Trash2 } from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import type { ConnectionSnapshot, ConnectionState } from "@/types";
import { formatBytes, formatTimeHms } from "@/utils/format";
import HelpTooltip from "@/components/HelpTooltip.vue";
import ModalDialog from "@/components/ModalDialog.vue";
import SelectMenu, { type SelectOption } from "@/components/SelectMenu.vue";
import ToggleSwitch from "@/components/ToggleSwitch.vue";

const store = useAppStore();

const filterMode = ref<"include" | "exclude">("include");
const searchQuery = ref("");
const inspectingId = ref<number | null>(null);

const filterOptions: SelectOption[] = [
  { value: "include", label: "包含" },
  { value: "exclude", label: "排除" },
];

const inspecting = computed<ConnectionSnapshot | null>(() => {
  if (inspectingId.value === null) return null;
  return store.connections.find((c) => c.id === inspectingId.value) ?? null;
});

onMounted(() => {
  void store.refreshConnections();
});

const filteredConnections = computed(() => {
  const q = searchQuery.value.trim().toLowerCase();
  return store.connections.filter((c) => {
    if (
      !store.config.settings.discoverUnmatched &&
      !c.ruleId &&
      c.action === "direct"
    ) {
      if (!q) return false;
    }
    if (!q) return true;
    const haystack = [
      c.appName,
      c.processName,
      c.domain ?? "",
      c.dstAddr,
      c.proxyName ?? "",
      c.ruleName ?? "",
    ]
      .join(" ")
      .toLowerCase();
    const matched = haystack.includes(q);
    return filterMode.value === "include" ? matched : !matched;
  });
});

function destinationText(conn: ConnectionSnapshot): string {
  const port = conn.dstAddr.split(":").pop() ?? "";
  if (conn.domain) {
    return port ? `${conn.domain}:${port}` : conn.domain;
  }
  return conn.dstAddr;
}

function stateLabel(state: ConnectionState): string {
  switch (state) {
    case "connecting":
      return "连接中";
    case "active":
      return "活跃";
    case "closed":
      return "已关闭";
    case "failed":
      return "失败";
  }
}

function createRuleFromConnection(conn: ConnectionSnapshot) {
  const path = conn.executablePath || conn.processName;
  const name = conn.appName || conn.processName.replace(/\.exe$/i, "");
  inspectingId.value = null;
  store.openQuickAddRule({ name, path });
}
</script>

<template>
  <div class="space-y-4">
    <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
      连接
    </h1>

    <!-- Top Filter Bar -->
    <div class="flex flex-wrap items-center gap-3">
      <SelectMenu
        :model-value="filterMode"
        :options="filterOptions"
        variant="pill"
        @update:model-value="filterMode = $event as 'include' | 'exclude'"
      />

      <div class="relative min-w-[240px] flex-1">
        <Search class="absolute left-3 top-2 h-3.5 w-3.5 text-gray-400" />
        <input
          v-model="searchQuery"
          type="text"
          placeholder="搜索应用、进程、域名、IP 或代理..."
          class="w-full rounded-full border border-gray-200 bg-white py-1.5 pl-8 pr-3 text-[12.5px] outline-none focus:border-[#0A7AFF]"
        />
      </div>

      <div class="flex items-center gap-1.5 text-[12.5px] text-gray-600">
        <span>记录中</span>
        <HelpTooltip
          text="开启后实时记录并展示应用的连接历史；关闭仅暂停列表记录，不影响后台代理路由。"
        />
        <ToggleSwitch
          size="sm"
          :model-value="store.config.settings.recordConnections"
          @update:model-value="
            (val) => store.updateSettings({ recordConnections: val })
          "
        />
      </div>

      <div class="flex items-center gap-1.5 text-[12.5px] text-gray-600">
        <span>探索</span>
        <HelpTooltip
          text="开启后同时展示未命中任何规则的直连流量，便于发现后台子进程或新应用并一键创建规则。"
        />
        <ToggleSwitch
          size="sm"
          :model-value="store.config.settings.discoverUnmatched"
          @update:model-value="
            (val) => store.updateSettings({ discoverUnmatched: val })
          "
        />
      </div>

      <button
        type="button"
        title="刷新连接列表"
        class="rounded-lg border border-gray-200 bg-white p-1.5 text-gray-500 transition hover:bg-gray-50 hover:text-gray-800"
        @click="store.refreshConnections()"
      >
        <RefreshCw class="h-4 w-4" />
      </button>

      <button
        type="button"
        title="清空连接列表"
        class="rounded-lg border border-gray-200 bg-white p-1.5 text-gray-500 transition hover:bg-gray-50 hover:text-gray-800"
        @click="store.clearConnections()"
      >
        <Trash2 class="h-4 w-4" />
      </button>
    </div>

    <!-- Connection Stream -->
    <div class="antify-card divide-y divide-gray-100 overflow-hidden">
      <div
        v-for="conn in filteredConnections"
        :key="conn.id"
        class="flex items-center justify-between gap-3 px-4 py-2 text-[12.5px] transition hover:bg-gray-50/80"
      >
        <div class="flex min-w-0 items-center gap-2.5">
          <span class="shrink-0 font-mono text-[11.5px] text-gray-400">
            {{ formatTimeHms(conn.timestampMs) }}
          </span>

          <span
            class="shrink-0 rounded px-1.5 py-0.5 font-mono text-[10.5px] font-semibold uppercase"
            :class="
              conn.protocol === 'tcp'
                ? 'bg-sky-100 text-sky-700'
                : 'bg-orange-100 text-orange-700'
            "
          >
            {{ conn.protocol }}
          </span>

          <div
            class="flex h-5 w-5 shrink-0 items-center justify-center rounded bg-slate-800 text-[10px] font-bold text-white"
          >
            {{ (conn.appName || conn.processName).slice(0, 1).toUpperCase() }}
          </div>

          <span class="shrink-0 font-medium text-gray-900">
            {{ conn.appName || conn.processName }}
          </span>

          <span
            v-if="conn.processName"
            class="hidden truncate text-[12px] text-gray-400 sm:inline"
          >
            · {{ conn.processName }}
          </span>

          <span class="text-gray-300">→</span>

          <span class="truncate font-mono text-[12px] text-[#D97706]">
            {{ destinationText(conn) }}
          </span>
        </div>

        <div class="flex shrink-0 items-center gap-2">
          <span
            v-if="conn.ruleId"
            title="已命中规则"
            class="rounded bg-emerald-100 px-1.5 py-0.5 text-[10px] font-semibold text-emerald-700"
          >
            R
          </span>

          <span
            class="text-[12px] font-medium"
            :class="
              conn.action === 'block'
                ? 'text-red-500'
                : conn.action === 'proxy'
                  ? 'text-[#0A7AFF]'
                  : 'text-gray-500'
            "
          >
            {{
              conn.action === "block"
                ? "已阻断"
                : conn.action === "proxy"
                  ? conn.proxyName ?? "代理"
                  : "直连"
            }}
          </span>

          <span
            v-if="conn.action !== 'block'"
            :title="conn.state === 'failed' ? (conn.error ?? undefined) : undefined"
            class="hidden rounded px-1.5 py-0.5 text-[10px] font-medium md:inline"
            :class="
              conn.state === 'active'
                ? 'bg-emerald-50 text-emerald-700'
                : conn.state === 'connecting'
                  ? 'bg-amber-50 text-amber-700'
                  : conn.state === 'failed'
                    ? 'bg-red-50 text-red-600'
                    : 'bg-gray-100 text-gray-500'
            "
          >
            {{ stateLabel(conn.state) }}
          </span>

          <span
            class="hidden rounded bg-blue-50 px-1.5 py-0.5 font-mono text-[10px] text-blue-600 md:inline"
          >
            {{ formatBytes(conn.bytesUp + conn.bytesDown) }}
          </span>

          <button
            type="button"
            title="查看连接详情"
            class="rounded p-0.5 text-gray-400 transition hover:bg-gray-100 hover:text-gray-700"
            @click="inspectingId = conn.id"
          >
            <Info class="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      <div
        v-if="filteredConnections.length === 0"
        class="py-12 text-center text-[13px] text-gray-400"
      >
        暂无匹配的连接记录。开启「探索」可观察未命中规则的直连流量。
      </div>
    </div>

    <!-- Connection Detail Modal -->
    <ModalDialog
      :open="inspecting !== null"
      title="连接详情"
      @close="inspectingId = null"
    >
      <div v-if="inspecting" class="space-y-2.5 text-[12.5px]">
        <div class="grid grid-cols-3 gap-2 border-b border-gray-100 pb-2">
          <span class="text-gray-500">应用 / 进程</span>
          <span class="col-span-2 font-medium text-gray-900">
            {{ inspecting.appName }} ({{ inspecting.processName }} / PID {{ inspecting.pid }})
          </span>
        </div>
        <div class="grid grid-cols-3 gap-2 border-b border-gray-100 pb-2">
          <span class="text-gray-500">可执行路径</span>
          <span class="col-span-2 break-all font-mono text-[11.5px] text-gray-700">
            {{ inspecting.executablePath || "—" }}
          </span>
        </div>
        <div class="grid grid-cols-3 gap-2 border-b border-gray-100 pb-2">
          <span class="text-gray-500">源地址</span>
          <span class="col-span-2 font-mono text-gray-700">{{ inspecting.srcAddr }}</span>
        </div>
        <div class="grid grid-cols-3 gap-2 border-b border-gray-100 pb-2">
          <span class="text-gray-500">目标地址</span>
          <span class="col-span-2 font-mono text-gray-700">
            {{ inspecting.dstAddr }}
            <span v-if="inspecting.domain" class="text-[#D97706]">
              ({{ inspecting.domain }})
            </span>
          </span>
        </div>
        <div class="grid grid-cols-3 gap-2 border-b border-gray-100 pb-2">
          <span class="text-gray-500">匹配规则与路由</span>
          <span class="col-span-2 text-gray-800">
            {{ inspecting.ruleName ?? "默认策略" }} · {{ inspecting.action.toUpperCase() }}
            <span v-if="inspecting.proxyName"> → {{ inspecting.proxyName }}</span>
          </span>
        </div>
        <div class="grid grid-cols-3 gap-2">
          <span class="text-gray-500">累计流量 / 状态</span>
          <span class="col-span-2 text-gray-800">
            ↑ {{ formatBytes(inspecting.bytesUp) }} / ↓ {{ formatBytes(inspecting.bytesDown) }} ({{ stateLabel(inspecting.state) }})
          </span>
        </div>
        <div
          v-if="inspecting.error"
          class="rounded-lg bg-red-50 px-3 py-2 text-[12px] text-red-600"
        >
          {{ inspecting.error }}
        </div>
      </div>

      <template v-if="inspecting" #footer>
        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-lg bg-[#0A7AFF] px-3.5 py-1.5 text-[12.5px] font-medium text-white transition hover:bg-blue-600"
          @click="createRuleFromConnection(inspecting)"
        >
          <Plus class="h-3.5 w-3.5" />
          为此应用创建规则
        </button>
      </template>
    </ModalDialog>
  </div>
</template>
