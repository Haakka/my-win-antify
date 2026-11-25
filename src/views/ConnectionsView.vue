<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  Check,
  Copy,
  Info,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  X,
} from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { useI18n } from "@/i18n";
import type { ConnectionSnapshot, ConnectionState } from "@/types";
import { formatBytes, formatTimeHms } from "@/utils/format";
import HelpTooltip from "@/components/HelpTooltip.vue";
import ModalDialog from "@/components/ModalDialog.vue";
import SelectMenu, { type SelectOption } from "@/components/SelectMenu.vue";
import ToggleSwitch from "@/components/ToggleSwitch.vue";

const store = useAppStore();
const { t } = useI18n();

const filterMode = ref<"include" | "exclude">("include");
const searchQuery = ref("");
const inspectingId = ref<number | null>(null);
const expandedId = ref<number | null>(null);
const copiedDiagId = ref<number | null>(null);

const filterOptions = computed<SelectOption[]>(() => [
  { value: "include", label: t("connections.include") },
  { value: "exclude", label: t("connections.exclude") },
]);

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
      c.matchReason ?? "",
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
      return t("connections.stateConnecting");
    case "active":
      return t("connections.stateActive");
    case "closed":
      return t("connections.stateClosed");
    case "failed":
      return t("connections.stateFailed");
  }
}

function failureLabel(code: string | null | undefined): string {
  if (!code) return t("connections.stateFailed");
  switch (code) {
    case "timeout":
      return t("connections.failTimeout");
    case "auth_failed":
      return t("connections.failAuth");
    case "tls_cert_error":
      return t("connections.failCert");
    case "protocol_error":
      return t("connections.failProtocol");
    case "dest_refused":
      return t("connections.failRefused");
    case "network_lock":
      return t("connections.failLock");
    default:
      return code;
  }
}

const discoverCountdownText = computed(() => {
  const secs = store.discoverRemainingSecs;
  if (secs === null) return "";
  const m = Math.floor(secs / 60);
  const s = String(secs % 60).padStart(2, "0");
  return t("connections.discoverCountdown", { time: `${m}:${s}` });
});

function toggleExpand(id: number) {
  expandedId.value = expandedId.value === id ? null : id;
}

async function copyDiagnostics(conn: ConnectionSnapshot) {
  const text = [
    `App: ${conn.appName || conn.processName} (PID ${conn.pid})`,
    `Protocol: ${conn.protocol.toUpperCase()}`,
    `Source: ${conn.srcAddr}`,
    `Destination: ${destinationText(conn)} (${conn.dstAddr})`,
    `Route: ${conn.action.toUpperCase()}${conn.proxyName ? ` -> ${conn.proxyName}` : ""}`,
    `Rule: ${conn.ruleName ?? t("connections.detailDefault")}`,
    `Reason: ${conn.matchReason ?? t("connections.diagDefaultPolicy")}`,
    conn.failureCode ? `FailureCode: ${conn.failureCode}` : null,
    conn.error ? `Error: ${conn.error}` : null,
  ]
    .filter(Boolean)
    .join("\n");
  try {
    await navigator.clipboard.writeText(text);
    copiedDiagId.value = conn.id;
    window.setTimeout(() => {
      if (copiedDiagId.value === conn.id) copiedDiagId.value = null;
    }, 1500);
  } catch {
    // Clipboard unavailable.
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
      {{ t("connections.title") }}
    </h1>

    <div
      v-if="store.discoverAutoOffNotice"
      class="flex items-center justify-between gap-2 rounded-xl border border-amber-200 bg-amber-50 px-3.5 py-2 text-[12px] text-amber-800"
    >
      <span>{{ t("connections.discoverAutoOffBanner") }}</span>
      <button
        type="button"
        class="rounded p-0.5 opacity-60 hover:opacity-100"
        @click="store.discoverAutoOffNotice = false"
      >
        <X class="h-3.5 w-3.5" />
      </button>
    </div>

    <div class="flex flex-wrap items-center gap-3">
      <SelectMenu
        :model-value="filterMode"
        :options="filterOptions"
        variant="pill"
        @update:model-value="filterMode = $event as 'include' | 'exclude'"
      />

      <div class="relative min-w-[210px] flex-1">
        <Search class="absolute left-3 top-2 h-3.5 w-3.5 text-gray-400" />
        <input
          v-model="searchQuery"
          type="text"
          :placeholder="t('connections.searchPlaceholder')"
          class="w-full rounded-full border border-gray-200 bg-white py-1.5 pl-8 pr-3 text-[12.5px] outline-none focus:border-[#0A7AFF]"
        />
      </div>

      <div class="flex items-center gap-1.5 text-[12.5px] text-gray-600">
        <span>{{ t("connections.recording") }}</span>
        <HelpTooltip :text="t('connections.recordingHelp')" />
        <ToggleSwitch
          size="sm"
          :model-value="store.config.settings.recordConnections"
          @update:model-value="
            (val) => store.updateSettings({ recordConnections: val })
          "
        />
      </div>

      <div class="flex items-center gap-1.5 text-[12.5px] text-gray-600">
        <span>{{ t("connections.discover") }}</span>
        <HelpTooltip :text="t('connections.discoverHelp')" />
        <ToggleSwitch
          size="sm"
          :model-value="store.config.settings.discoverUnmatched"
          @update:model-value="
            (val) => store.updateSettings({ discoverUnmatched: val })
          "
        />
        <span
          v-if="store.config.settings.discoverUnmatched && store.discoverRemainingSecs !== null"
          class="rounded-full bg-amber-100 px-2 py-0.5 font-mono text-[10.5px] text-amber-800"
        >
          {{ discoverCountdownText }}
        </span>
      </div>

      <button
        type="button"
        :title="t('connections.refreshTitle')"
        class="rounded-lg border border-gray-200 bg-white p-1.5 text-gray-500 transition hover:bg-gray-50 hover:text-gray-800"
        @click="store.refreshConnections()"
      >
        <RefreshCw class="h-4 w-4" />
      </button>

      <button
        type="button"
        :title="t('connections.clearTitle')"
        class="rounded-lg border border-gray-200 bg-white p-1.5 text-gray-500 transition hover:bg-gray-50 hover:text-gray-800"
        @click="store.clearConnections()"
      >
        <Trash2 class="h-4 w-4" />
      </button>
    </div>

    <div class="antify-card divide-y divide-gray-100 overflow-hidden">
      <div v-for="conn in filteredConnections" :key="conn.id">
        <div
          class="flex cursor-pointer items-center justify-between gap-3 px-4 py-2 text-[12.5px] transition hover:bg-gray-50/80"
          @click="toggleExpand(conn.id)"
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
              :title="t('connections.matchedRuleBadge')"
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
                  ? t("connections.blocked")
                  : conn.action === "proxy"
                    ? conn.proxyName ?? t("common.proxy")
                    : t("common.direct")
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
              {{
                conn.state === "failed" && conn.failureCode
                  ? failureLabel(conn.failureCode)
                  : stateLabel(conn.state)
              }}
            </span>

            <span
              class="hidden rounded bg-blue-50 px-1.5 py-0.5 font-mono text-[10px] text-blue-600 md:inline"
            >
              {{ formatBytes(conn.bytesUp + conn.bytesDown) }}
            </span>

            <button
              type="button"
              :title="t('connections.detailTitle')"
              class="rounded p-0.5 text-gray-400 transition hover:bg-gray-100 hover:text-gray-700"
              @click.stop="inspectingId = conn.id"
            >
              <Info class="h-3.5 w-3.5" />
            </button>
          </div>
        </div>

        <!-- Inline Routing Diagnostics Drawer -->
        <div
          v-if="expandedId === conn.id"
          class="border-t border-gray-100 bg-gray-50/80 px-4 py-2.5 text-[12px] dark:bg-[#22252D]/60"
        >
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div class="space-y-1">
              <div class="font-semibold text-gray-800">
                {{ t("connections.diagTitle") }}
              </div>
              <div class="text-gray-600">
                <span class="text-gray-400">{{ t("connections.diagReason") }}:</span>
                {{ conn.matchReason ?? t("connections.diagDefaultPolicy") }}
              </div>
              <div class="flex flex-wrap items-center gap-3 text-[11.5px] text-gray-500">
                <span>
                  {{ t("connections.diagRule") }}:
                  <strong>{{ conn.ruleName ?? t("connections.detailDefault") }}</strong>
                </span>
                <span v-if="conn.failureCode" class="text-red-600">
                  {{ t("connections.diagFailureCode") }}:
                  <code class="rounded bg-red-100 px-1 py-0.5 font-mono">{{ conn.failureCode }}</code>
                  ({{ failureLabel(conn.failureCode) }})
                </span>
                <span v-if="conn.error" class="text-red-600">{{ conn.error }}</span>
              </div>
            </div>

            <div class="flex items-center gap-2">
              <button
                type="button"
                class="inline-flex items-center gap-1 rounded-lg border border-gray-200 bg-white px-2.5 py-1 text-[11.5px] font-medium text-gray-700 transition hover:bg-gray-50"
                @click.stop="copyDiagnostics(conn)"
              >
                <Check v-if="copiedDiagId === conn.id" class="h-3.5 w-3.5 text-emerald-600" />
                <Copy v-else class="h-3.5 w-3.5" />
                {{ copiedDiagId === conn.id ? t("common.copied") : t("connections.diagCopy") }}
              </button>
              <button
                type="button"
                class="inline-flex items-center gap-1 rounded-lg bg-[#0A7AFF] px-2.5 py-1 text-[11.5px] font-medium text-white transition hover:bg-blue-600"
                @click.stop="createRuleFromConnection(conn)"
              >
                <Plus class="h-3.5 w-3.5" />
                {{ t("connections.createRuleBtn") }}
              </button>
            </div>
          </div>
        </div>
      </div>

      <div
        v-if="filteredConnections.length === 0"
        class="py-12 text-center text-[13px] text-gray-400"
      >
        {{ t("connections.empty") }}
      </div>
    </div>

    <ModalDialog
      :open="inspecting !== null"
      :title="t('connections.detailTitle')"
      @close="inspectingId = null"
    >
      <div v-if="inspecting" class="space-y-2.5 text-[12.5px]">
        <div class="grid grid-cols-3 gap-2 border-b border-gray-100 pb-2">
          <span class="text-gray-500">{{ t("connections.detailApp") }}</span>
          <span class="col-span-2 font-medium text-gray-900">
            {{ inspecting.appName }} ({{ inspecting.processName }} / PID {{ inspecting.pid }})
          </span>
        </div>
        <div class="grid grid-cols-3 gap-2 border-b border-gray-100 pb-2">
          <span class="text-gray-500">{{ t("connections.detailExe") }}</span>
          <span class="col-span-2 break-all font-mono text-[11.5px] text-gray-700">
            {{ inspecting.executablePath || "—" }}
          </span>
        </div>
        <div class="grid grid-cols-3 gap-2 border-b border-gray-100 pb-2">
          <span class="text-gray-500">{{ t("connections.detailSrc") }}</span>
          <span class="col-span-2 font-mono text-gray-700">{{ inspecting.srcAddr }}</span>
        </div>
        <div class="grid grid-cols-3 gap-2 border-b border-gray-100 pb-2">
          <span class="text-gray-500">{{ t("connections.detailDst") }}</span>
          <span class="col-span-2 font-mono text-gray-700">
            {{ inspecting.dstAddr }}
            <span v-if="inspecting.domain" class="text-[#D97706]">
              ({{ inspecting.domain }})
            </span>
          </span>
        </div>
        <div class="grid grid-cols-3 gap-2 border-b border-gray-100 pb-2">
          <span class="text-gray-500">{{ t("connections.detailRoute") }}</span>
          <span class="col-span-2 text-gray-800">
            {{ inspecting.ruleName ?? t("connections.detailDefault") }} · {{ inspecting.action.toUpperCase() }}
            <span v-if="inspecting.proxyName"> → {{ inspecting.proxyName }}</span>
            <div v-if="inspecting.matchReason" class="mt-0.5 text-[11.5px] text-gray-500">
              {{ inspecting.matchReason }}
            </div>
          </span>
        </div>
        <div class="grid grid-cols-3 gap-2">
          <span class="text-gray-500">{{ t("connections.detailTraffic") }}</span>
          <span class="col-span-2 text-gray-800">
            ↑ {{ formatBytes(inspecting.bytesUp) }} / ↓ {{ formatBytes(inspecting.bytesDown) }} ({{ stateLabel(inspecting.state) }})
          </span>
        </div>
        <div
          v-if="inspecting.error || inspecting.failureCode"
          class="rounded-lg bg-red-50 px-3 py-2 text-[12px] text-red-600"
        >
          <span v-if="inspecting.failureCode" class="mr-1.5 font-semibold">
            [{{ failureLabel(inspecting.failureCode) }}]
          </span>
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
          {{ t("connections.createRuleBtn") }}
        </button>
      </template>
    </ModalDialog>
  </div>
</template>
