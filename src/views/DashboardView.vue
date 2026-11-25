<script setup lang="ts">
import {
  Activity,
  AlertTriangle,
  ArrowDown,
  ArrowLeftRight,
  ArrowUp,
  CheckCircle2,
  Globe,
  Layers,
  ListChecks,
  Play,
  Plus,
  Power,
  Radio,
  RotateCcw,
  Server,
  Sparkles,
  Square,
} from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { useI18n } from "@/i18n";
import { formatBps, formatUptime } from "@/utils/format";
import OnboardingBanner from "@/components/OnboardingBanner.vue";

const store = useAppStore();
const { locale, t } = useI18n();

function actionLabel(action: "proxy" | "direct" | "block"): string {
  return t(`common.${action}`);
}
</script>

<template>
  <div class="space-y-4">
    <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
      {{ t("dashboard.title") }}
    </h1>

    <OnboardingBanner />

    <!-- Corrupt Config Alert Banner -->
    <div
      v-if="store.status.configError"
      class="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-red-300/90 bg-red-50 px-4 py-3 text-[12.5px] text-red-800 dark:border-red-500/35 dark:bg-red-950/35 dark:text-red-200"
    >
      <div class="flex min-w-0 items-start gap-2.5">
        <AlertTriangle class="mt-0.5 h-4.5 w-4.5 shrink-0 text-red-600 dark:text-red-400" />
        <div class="min-w-0">
          <div class="font-semibold">{{ t("dashboard.configErrorTitle") }}</div>
          <div class="mt-0.5 break-all text-[12px] opacity-90">
            {{ store.status.configError }}
          </div>
        </div>
      </div>
      <button
        type="button"
        :disabled="store.busySaving"
        class="inline-flex shrink-0 items-center gap-1.5 rounded-lg bg-red-600 px-3 py-1.5 text-[12px] font-medium text-white transition hover:bg-red-700 disabled:opacity-50"
        @click="store.restoreConfigBackup()"
      >
        <RotateCcw class="h-3.5 w-3.5" />
        {{ t("dashboard.restoreBackup") }}
      </button>
    </div>

    <!-- Top Status Banner -->
    <div
      class="flex items-center justify-between gap-3 rounded-2xl border px-5 py-4 transition-colors"
      :class="
        store.status.running
          ? 'border-emerald-200/80 bg-[#EAF9F0] dark:border-emerald-500/25 dark:bg-[#15241D]'
          : 'border-gray-200/80 bg-white dark:border-white/[0.07] dark:bg-[#1C1E24]'
      "
    >
      <div class="flex items-center gap-3.5">
        <div
          class="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl"
          :class="
            store.status.running
              ? 'bg-emerald-500/15 text-emerald-600 dark:bg-emerald-500/20 dark:text-emerald-400'
              : 'bg-gray-100 text-gray-400 dark:bg-[#252830] dark:text-gray-500'
          "
        >
          <CheckCircle2 v-if="store.status.running" class="h-6 w-6" />
          <Power v-else class="h-6 w-6" />
        </div>
        <div>
          <div class="text-[15px] font-semibold text-gray-900">
            {{ store.status.running ? t("dashboard.proxyActive") : t("dashboard.proxyInactive") }}
          </div>
          <div class="text-[12.5px] text-gray-500">
            {{
              store.status.running
                ? t("dashboard.proxyActiveDesc")
                : t("dashboard.proxyInactiveDesc")
            }}
          </div>
        </div>
      </div>

      <button
        type="button"
        :disabled="
          store.busyEngine ||
          !store.isServiceConnected ||
          Boolean(store.status.configError)
        "
        class="inline-flex shrink-0 items-center gap-2 rounded-full px-5 py-2 text-[13px] font-medium transition disabled:cursor-not-allowed disabled:opacity-50"
        :class="
          store.status.running
            ? 'bg-gray-200/90 text-gray-800 hover:bg-gray-300/80 dark:bg-[#2A2E38] dark:text-gray-100 dark:hover:bg-[#353A47]'
            : 'bg-[#0A7AFF] text-white shadow-sm hover:bg-blue-600'
        "
        @click="store.toggleEngine()"
      >
        <Square
          v-if="store.status.running"
          class="h-3 w-3 fill-current"
        />
        <Play v-else class="h-3.5 w-3.5 fill-current" />
        <span>{{ store.status.running ? t("dashboard.stop") : t("dashboard.start") }}</span>
      </button>
    </div>

    <!-- Middle 3-Column Feature Cards -->
    <div class="grid grid-cols-1 gap-4 md:grid-cols-3">
      <!-- General / Active Profile Card -->
      <div
        class="antify-card flex cursor-pointer flex-col justify-between p-4 transition hover:border-gray-300"
        @click="store.activeRoute = 'profiles'"
      >
        <div class="flex items-center gap-2.5">
          <div
            class="flex h-8 w-8 items-center justify-center rounded-lg bg-[#0A7AFF] text-white"
          >
            <Globe class="h-4.5 w-4.5" />
          </div>
          <div class="text-[14px] font-semibold text-gray-900">
            {{ store.activeProfile.name }}
          </div>
        </div>

        <div
          class="antify-inset mt-5 flex items-center justify-between px-3 py-2 text-[12.5px]"
        >
          <span class="text-gray-500">{{ t("dashboard.defaultPolicy") }}</span>
          <span class="font-medium text-gray-800">
            {{ actionLabel(store.activeProfile.defaultAction) }}
          </span>
        </div>
      </div>

      <!-- Automation Card -->
      <div
        class="antify-card flex cursor-pointer flex-col justify-between p-4 transition hover:border-gray-300"
        @click="store.activeRoute = 'automation'"
      >
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2.5">
            <div
              class="flex h-8 w-8 items-center justify-center rounded-lg bg-[#00B4D8] text-white"
            >
              <Radio class="h-4.5 w-4.5" />
            </div>
            <div class="text-[14px] font-semibold text-gray-900">
              {{ t("dashboard.automation") }}
            </div>
          </div>
          <span
            class="inline-flex items-center gap-1 rounded-full px-2.5 py-0.5 text-[11px] font-medium"
            :class="
              store.config.automation.enabled
                ? 'bg-emerald-100/80 text-emerald-700'
                : 'bg-gray-100 text-gray-500'
            "
          >
            {{
              store.config.automation.enabled
                ? `${t("dashboard.autoOn")} (${store.config.automation.wifiRules.filter((r) => r.enabled).length})`
                : t("dashboard.autoOff")
            }}
          </span>
        </div>

        <div
          class="antify-inset mt-5 flex items-center justify-between px-3 py-2 text-[12.5px]"
        >
          <span class="text-gray-500">{{ t("dashboard.wifiNetwork") }}</span>
          <span class="truncate font-medium text-gray-700">
            {{
              store.currentWifi.connected && store.currentWifi.ssid
                ? store.currentWifi.ssid
                : store.config.automation.wifiRules.length > 0
                  ? t("dashboard.wifiRuleCount", { count: store.config.automation.wifiRules.length })
                  : t("dashboard.noWifiRules")
            }}
          </span>
        </div>
      </div>

      <!-- Realtime Telemetry Card -->
      <div class="antify-card flex flex-col justify-between p-4">
        <div class="flex items-center justify-between gap-2">
          <div class="flex items-center gap-2.5">
            <div
              class="flex h-8 w-8 items-center justify-center rounded-lg bg-[#5E5CE6] text-white"
            >
              <Activity class="h-4.5 w-4.5" />
            </div>
            <div class="text-[14px] font-semibold text-gray-900">
              {{ t("dashboard.realtime") }}
            </div>
          </div>

          <span
            class="truncate rounded-full px-2.5 py-0.5 text-[11px] font-medium"
            :class="
              store.status.running
                ? 'bg-emerald-100/80 text-emerald-700'
                : 'bg-gray-100 text-gray-500'
            "
          >
            {{
              store.status.running
                ? t("dashboard.runningUptime", { uptime: formatUptime(store.stats.uptimeSecs, locale) })
                : t("dashboard.notRunning")
            }}
          </span>
        </div>

        <div class="mt-3 grid grid-cols-2 gap-2.5">
          <button
            type="button"
            class="antify-inset flex flex-col items-center justify-center py-2.5 transition hover:bg-gray-200/60"
            @click="store.activeRoute = 'connections'"
          >
            <span class="inline-flex items-center gap-1 text-[11.5px] text-gray-500">
              <ArrowLeftRight class="h-3 w-3 text-[#0A7AFF]" />
              {{ t("dashboard.connections") }}
            </span>
            <span class="mt-0.5 text-[20px] font-bold leading-tight text-[#0A7AFF]">
              {{ store.stats.activeConnections }}
            </span>
          </button>

          <button
            type="button"
            class="antify-inset flex flex-col items-center justify-center py-2.5 transition hover:bg-gray-200/60"
            @click="store.activeRoute = 'connections'"
          >
            <span class="inline-flex items-center gap-1 text-[11.5px] text-gray-500">
              <Sparkles class="h-3 w-3 text-[#A855F7]" />
              {{ t("dashboard.apps") }}
            </span>
            <span class="mt-0.5 text-[20px] font-bold leading-tight text-[#A855F7]">
              {{ store.stats.activeApps }}
            </span>
          </button>
        </div>

        <div class="mt-2 flex items-center justify-between px-1 text-[11px] text-gray-400">
          <span class="inline-flex items-center gap-1">
            <ArrowUp class="h-3 w-3 text-orange-500" />
            {{ formatBps(store.stats.uploadBps) }}
          </span>
          <span class="inline-flex items-center gap-1">
            <ArrowDown class="h-3 w-3 text-blue-500" />
            {{ formatBps(store.stats.downloadBps) }}
          </span>
        </div>
      </div>
    </div>

    <!-- Bottom 3-Column Summary Card -->
    <div class="antify-card grid grid-cols-3 divide-x divide-gray-100">
      <div
        class="flex cursor-pointer flex-col justify-between p-4 transition hover:bg-gray-50/60"
        @click="store.activeRoute = 'proxies'"
      >
        <div class="flex items-center justify-between">
          <div class="flex h-7 w-7 items-center justify-center rounded-lg bg-[#0A7AFF] text-white">
            <Server class="h-4 w-4" />
          </div>
          <button
            type="button"
            :title="t('dashboard.addProxyTitle')"
            class="flex h-6 w-6 items-center justify-center rounded-full bg-gray-100 text-gray-500 transition hover:bg-gray-200 hover:text-gray-800"
            @click.stop="store.quickAddProxyOpen = true"
          >
            <Plus class="h-3.5 w-3.5" />
          </button>
        </div>
        <div class="mt-4">
          <div class="text-[24px] font-bold leading-none text-gray-900">
            {{ store.config.proxies.length }}
          </div>
          <div class="mt-1.5 text-[12px] text-gray-500">{{ t("dashboard.proxies") }}</div>
        </div>
      </div>

      <div
        class="flex cursor-pointer flex-col justify-between p-4 transition hover:bg-gray-50/60"
        @click="store.activeRoute = 'rules'"
      >
        <div class="flex items-center justify-between">
          <div class="flex h-7 w-7 items-center justify-center rounded-lg bg-[#AF52DE] text-white">
            <ListChecks class="h-4 w-4" />
          </div>
          <button
            type="button"
            :title="t('dashboard.addRuleTitle')"
            class="flex h-6 w-6 items-center justify-center rounded-full bg-gray-100 text-gray-500 transition hover:bg-gray-200 hover:text-gray-800"
            @click.stop="store.quickAddRuleOpen = true"
          >
            <Plus class="h-3.5 w-3.5" />
          </button>
        </div>
        <div class="mt-4">
          <div class="text-[24px] font-bold leading-none text-gray-900">
            {{ store.activeProfileRules.length }}
          </div>
          <div class="mt-1.5 text-[12px] text-gray-500">{{ t("dashboard.rules") }}</div>
        </div>
      </div>

      <div
        class="flex cursor-pointer flex-col justify-between p-4 transition hover:bg-gray-50/60"
        @click="store.activeRoute = 'profiles'"
      >
        <div class="flex items-center justify-between">
          <div class="flex h-7 w-7 items-center justify-center rounded-lg bg-[#FF9500] text-white">
            <Layers class="h-4 w-4" />
          </div>
          <button
            type="button"
            :title="t('dashboard.addProfileTitle')"
            class="flex h-6 w-6 items-center justify-center rounded-full bg-gray-100 text-gray-500 transition hover:bg-gray-200 hover:text-gray-800"
            @click.stop="store.quickAddProfileOpen = true"
          >
            <Plus class="h-3.5 w-3.5" />
          </button>
        </div>
        <div class="mt-4">
          <div class="text-[24px] font-bold leading-none text-gray-900">
            {{ store.config.profiles.length }}
          </div>
          <div class="mt-1.5 text-[12px] text-gray-500">{{ t("dashboard.profiles") }}</div>
        </div>
      </div>
    </div>
  </div>
</template>
