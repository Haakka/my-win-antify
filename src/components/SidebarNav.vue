<script setup lang="ts">
import { computed, type Component } from "vue";
import {
  Activity,
  ArrowLeftRight,
  BarChart3,
  Gauge,
  Github,
  Layers,
  ListChecks,
  Monitor,
  Moon,
  Radio,
  Server,
  Settings,
  Shield,
  Sun,
} from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { SKINS, useTheme } from "@/composables/useTheme";
import type { NavRouteId, ThemeMode } from "@/types";
import ToggleSwitch from "./ToggleSwitch.vue";

const store = useAppStore();
const { skin, themeMode, setThemeMode, setSkin } = useTheme();

const themeBadgeLabel = computed(() => {
  if (themeMode.value === "dark") return "深色";
  if (themeMode.value === "system") return "自动";
  return "浅色";
});

function cycleThemeMode() {
  const order: ThemeMode[] = ["light", "dark", "system"];
  const next = order[(order.indexOf(themeMode.value) + 1) % order.length] ?? "light";
  void setThemeMode(next);
}

interface NavItem {
  id: NavRouteId;
  label: string;
  icon: Component;
}

interface NavSection {
  title?: string;
  items: NavItem[];
}

const sections: NavSection[] = [
  {
    items: [{ id: "dashboard", label: "仪表盘", icon: Gauge }],
  },
  {
    title: "配置",
    items: [
      { id: "proxies", label: "代理", icon: Server },
      { id: "rules", label: "规则", icon: ListChecks },
      { id: "profiles", label: "配置", icon: Layers },
    ],
  },
  {
    title: "自动化",
    items: [{ id: "automation", label: "自动化", icon: Radio }],
  },
  {
    title: "监控",
    items: [
      { id: "connections", label: "连接", icon: ArrowLeftRight },
      { id: "traffic", label: "流量", icon: BarChart3 },
      { id: "activity", label: "活动", icon: Activity },
    ],
  },
  {
    items: [{ id: "settings", label: "设置", icon: Settings }],
  },
];
</script>

<template>
  <aside
    class="flex h-full w-[200px] shrink-0 select-none flex-col justify-between border-r border-gray-200/80 bg-[#F2F3F6]/90 px-3 py-4 transition-colors duration-200 dark:border-white/[0.06] dark:bg-[#16181D]"
  >
    <div>
      <!-- Brand Header -->
      <div class="mb-4 flex items-center gap-2.5 px-2.5">
        <div
          class="flex h-7 w-7 items-center justify-center rounded-lg text-white shadow-sm"
          style="background: linear-gradient(135deg, var(--accent), var(--accent-hover))"
        >
          <Shield class="h-4 w-4" />
        </div>
        <div class="min-w-0">
          <div class="truncate text-[14px] font-semibold tracking-tight text-gray-900">
            MyWinAntify
          </div>
        </div>
      </div>

      <!-- Navigation Groups -->
      <nav class="space-y-3.5">
        <div v-for="(sec, sIdx) in sections" :key="sIdx">
          <div
            v-if="sec.title"
            class="mb-1 px-2.5 text-[11px] font-medium text-gray-400"
          >
            {{ sec.title }}
          </div>
          <div class="space-y-0.5">
            <button
              v-for="item in sec.items"
              :key="item.id"
              type="button"
              class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-[13px] font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-[#0A7AFF]/40"
              :class="
                store.activeRoute === item.id
                  ? 'bg-[#0A7AFF] text-white shadow-xs'
                  : 'text-gray-700 hover:bg-black/[0.045]'
              "
              @click="store.activeRoute = item.id"
            >
              <component
                :is="item.icon"
                class="h-4 w-4 shrink-0"
                :class="
                  store.activeRoute === item.id ? 'text-white' : 'text-[#0A7AFF]'
                "
              />
              <span class="truncate">{{ item.label }}</span>
            </button>
          </div>
        </div>
      </nav>
    </div>

    <!-- Footer Status & Theme/Skin & Mock Preview -->
    <div class="space-y-2 border-t border-gray-200/70 pt-3 dark:border-white/[0.06]">
      <div
        class="flex items-center justify-between rounded-lg bg-white/75 px-2 py-1.5 text-[11px] text-gray-600 dark:bg-[#1C1E24] dark:text-gray-300"
      >
        <button
          type="button"
          title="切换浅色 / 深色 / 跟随系统"
          class="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 font-medium text-gray-700 transition hover:bg-gray-100 dark:text-gray-200 dark:hover:bg-[#252830]"
          @click="cycleThemeMode"
        >
          <Moon v-if="themeMode === 'dark'" class="h-3.5 w-3.5 text-[#0A7AFF]" />
          <Monitor
            v-else-if="themeMode === 'system'"
            class="h-3.5 w-3.5 text-[#0A7AFF]"
          />
          <Sun v-else class="h-3.5 w-3.5 text-amber-500" />
          <span>{{ themeBadgeLabel }}</span>
        </button>

        <div class="flex items-center gap-1.5">
          <button
            v-for="item in SKINS"
            :key="item.id"
            type="button"
            :title="`切换皮肤：${item.label}`"
            class="h-3 w-3 rounded-full transition-transform hover:scale-110 focus:outline-none"
            :class="
              skin === item.id
                ? 'scale-110 ring-2 ring-gray-900/40 ring-offset-1 ring-offset-white dark:ring-white/80 dark:ring-offset-[#1C1E24]'
                : 'opacity-70 hover:opacity-100'
            "
            :style="{ backgroundColor: item.preview }"
            @click="setSkin(item.id)"
          />
        </div>
      </div>

      <div
        class="flex items-center justify-between rounded-lg bg-white/75 px-2.5 py-1.5 text-[11px] text-gray-600 dark:bg-[#1C1E24]"
      >
        <span class="font-medium">Mock 预览</span>
        <ToggleSwitch
          size="sm"
          :model-value="store.mockMode"
          @update:model-value="store.setMockMode"
        />
      </div>

      <div class="flex items-center justify-between px-2 text-[11px] text-gray-500">
        <span class="flex items-center gap-1.5">
          <span
            class="h-2 w-2 rounded-full"
            :class="
              store.mockMode
                ? 'bg-amber-400'
                : store.pipeState === 'connected'
                  ? 'bg-emerald-500'
                  : 'bg-gray-300'
            "
          />
          <span>
            {{
              store.mockMode
                ? "离线预览"
                : store.pipeState === "connected"
                  ? "服务已连接"
                  : "未连接服务"
            }}
          </span>
        </span>
        <span class="inline-flex items-center gap-1 text-gray-400">
          <Github class="h-3.5 w-3.5" />
          <span>v0.1</span>
        </span>
      </div>
    </div>
  </aside>
</template>
