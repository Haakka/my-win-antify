<script setup lang="ts">
import {
  Activity,
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
  Server,
  Sparkles,
  Square,
} from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { formatBps, formatUptime } from "@/utils/format";

const store = useAppStore();

const actionLabelMap = {
  proxy: "代理",
  direct: "直连",
  block: "阻断",
} as const;
</script>

<template>
  <div class="space-y-4">
    <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
      仪表盘
    </h1>

    <!-- Top Status Banner -->
    <div
      class="flex items-center justify-between rounded-2xl border px-5 py-4 transition-colors"
      :class="
        store.status.running
          ? 'border-emerald-200/80 bg-[#EAF9F0] dark:border-emerald-500/25 dark:bg-[#15241D]'
          : 'border-gray-200/80 bg-white dark:border-white/[0.07] dark:bg-[#1C1E24]'
      "
    >
      <div class="flex items-center gap-3.5">
        <div
          class="flex h-11 w-11 items-center justify-center rounded-xl"
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
            {{ store.status.running ? "代理已启用" : "代理未启用" }}
          </div>
          <div class="text-[12.5px] text-gray-500">
            {{
              store.status.running
                ? "已配置应用的流量通过 WinDivert 透明反射路由。"
                : "点击右侧启动按钮开始接管配置应用的出站流量。"
            }}
          </div>
        </div>
      </div>

      <button
        type="button"
        :disabled="store.busyEngine || !store.isServiceConnected"
        class="inline-flex items-center gap-2 rounded-full px-5 py-2 text-[13px] font-medium transition disabled:cursor-not-allowed disabled:opacity-50"
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
        <span>{{ store.status.running ? "停止" : "启动" }}</span>
      </button>
    </div>

    <!-- Middle 3-Column Feature Cards -->
    <div class="grid grid-cols-3 gap-4">
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
          <span class="text-gray-500">默认</span>
          <span class="font-medium text-gray-800">
            {{ actionLabelMap[store.activeProfile.defaultAction] }}
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
            <div class="text-[14px] font-semibold text-gray-900">自动化</div>
          </div>
          <span
            class="inline-flex items-center gap-1 rounded-full bg-gray-100 px-2.5 py-0.5 text-[11px] font-medium text-gray-500"
          >
            M2 规划
          </span>
        </div>

        <div
          class="antify-inset mt-5 flex items-center justify-between px-3 py-2 text-[12.5px]"
        >
          <span class="text-gray-500">Wi-Fi 网络</span>
          <span class="font-medium text-gray-700">无 Wi-Fi 规则</span>
        </div>
      </div>

      <!-- Realtime Telemetry Card -->
      <div
        class="antify-card flex flex-col justify-between p-4"
      >
        <div class="flex items-center justify-between gap-2">
          <div class="flex items-center gap-2.5">
            <div
              class="flex h-8 w-8 items-center justify-center rounded-lg bg-[#5E5CE6] text-white"
            >
              <Activity class="h-4.5 w-4.5" />
            </div>
            <div class="text-[14px] font-semibold text-gray-900">实时</div>
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
                ? `运行 ${formatUptime(store.stats.uptimeSecs)}`
                : "未运行"
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
              连接
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
              应用
            </span>
            <span class="mt-0.5 text-[20px] font-bold leading-tight text-[#A855F7]">
              {{ store.stats.activeApps }}
            </span>
          </button>
        </div>

        <div
          class="mt-2 flex items-center justify-between px-1 text-[11px] text-gray-400"
        >
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
          <div
            class="flex h-7 w-7 items-center justify-center rounded-lg bg-[#0A7AFF] text-white"
          >
            <Server class="h-4 w-4" />
          </div>
          <button
            type="button"
            title="新增代理"
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
          <div class="mt-1.5 text-[12px] text-gray-500">代理</div>
        </div>
      </div>

      <div
        class="flex cursor-pointer flex-col justify-between p-4 transition hover:bg-gray-50/60"
        @click="store.activeRoute = 'rules'"
      >
        <div class="flex items-center justify-between">
          <div
            class="flex h-7 w-7 items-center justify-center rounded-lg bg-[#AF52DE] text-white"
          >
            <ListChecks class="h-4 w-4" />
          </div>
          <button
            type="button"
            title="新增规则"
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
          <div class="mt-1.5 text-[12px] text-gray-500">规则</div>
        </div>
      </div>

      <div
        class="flex cursor-pointer flex-col justify-between p-4 transition hover:bg-gray-50/60"
        @click="store.activeRoute = 'profiles'"
      >
        <div class="flex items-center justify-between">
          <div
            class="flex h-7 w-7 items-center justify-center rounded-lg bg-[#FF9500] text-white"
          >
            <Layers class="h-4 w-4" />
          </div>
          <button
            type="button"
            title="新增配置"
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
          <div class="mt-1.5 text-[12px] text-gray-500">配置</div>
        </div>
      </div>
    </div>
  </div>
</template>
