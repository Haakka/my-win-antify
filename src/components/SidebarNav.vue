<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, type Component } from "vue";
import {
  Activity,
  ArrowLeftRight,
  BarChart3,
  ChevronLeft,
  ChevronRight,
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
import { useI18n } from "@/i18n";
import type { NavRouteId, ThemeMode } from "@/types";
import ToggleSwitch from "./ToggleSwitch.vue";

const store = useAppStore();
const { skin, themeMode, setThemeMode, setSkin } = useTheme();
const { t } = useI18n();

const manualCollapsed = ref<boolean | null>(null);
const narrowViewport = ref<boolean>(
  typeof window !== "undefined" ? window.innerWidth < 1020 : false,
);

const collapsed = computed(() =>
  manualCollapsed.value !== null ? manualCollapsed.value : narrowViewport.value,
);

function handleResize() {
  narrowViewport.value = window.innerWidth < 1020;
}

onMounted(() => {
  window.addEventListener("resize", handleResize);
});

onBeforeUnmount(() => {
  window.removeEventListener("resize", handleResize);
});

function toggleCollapsed() {
  manualCollapsed.value = !collapsed.value;
}

const themeBadgeLabel = computed(() => {
  if (themeMode.value === "dark") return t("sidebar.themeDark");
  if (themeMode.value === "system") return t("sidebar.themeAuto");
  return t("sidebar.themeLight");
});

function cycleThemeMode() {
  const order: ThemeMode[] = ["light", "dark", "system"];
  const next = order[(order.indexOf(themeMode.value) + 1) % order.length] ?? "light";
  void setThemeMode(next);
}

interface NavItem {
  id: NavRouteId;
  labelKey: string;
  icon: Component;
}

interface NavSection {
  titleKey?: string;
  items: NavItem[];
}

const sections: NavSection[] = [
  {
    items: [{ id: "dashboard", labelKey: "nav.dashboard", icon: Gauge }],
  },
  {
    titleKey: "nav.groupConfig",
    items: [
      { id: "proxies", labelKey: "nav.proxies", icon: Server },
      { id: "rules", labelKey: "nav.rules", icon: ListChecks },
      { id: "profiles", labelKey: "nav.profiles", icon: Layers },
    ],
  },
  {
    titleKey: "nav.groupAutomation",
    items: [{ id: "automation", labelKey: "nav.automation", icon: Radio }],
  },
  {
    titleKey: "nav.groupMonitor",
    items: [
      { id: "connections", labelKey: "nav.connections", icon: ArrowLeftRight },
      { id: "traffic", labelKey: "nav.traffic", icon: BarChart3 },
      { id: "activity", labelKey: "nav.activity", icon: Activity },
    ],
  },
  {
    items: [{ id: "settings", labelKey: "nav.settings", icon: Settings }],
  },
];
</script>

<template>
  <aside
    class="flex h-full shrink-0 select-none flex-col justify-between border-r border-gray-200/80 bg-[#F2F3F6]/90 py-4 transition-all duration-200 dark:border-white/[0.06] dark:bg-[#16181D]"
    :class="collapsed ? 'w-[68px] px-2' : 'w-[200px] px-3'"
  >
    <div>
      <!-- Brand Header + Collapse Toggle -->
      <div
        class="mb-4 flex items-center"
        :class="collapsed ? 'justify-center gap-1' : 'justify-between px-2.5'"
      >
        <div class="flex min-w-0 items-center gap-2.5">
          <div
            class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-white shadow-sm"
            style="background: linear-gradient(135deg, var(--accent), var(--accent-hover))"
            title="MyWinAntify"
          >
            <Shield class="h-4 w-4" />
          </div>
          <div v-if="!collapsed" class="min-w-0">
            <div class="truncate text-[14px] font-semibold tracking-tight text-gray-900">
              MyWinAntify
            </div>
          </div>
        </div>

        <button
          type="button"
          :title="collapsed ? t('sidebar.expand') : t('sidebar.collapse')"
          class="rounded-md p-1 text-gray-400 transition hover:bg-black/5 hover:text-gray-700"
          @click="toggleCollapsed"
        >
          <ChevronRight v-if="collapsed" class="h-3.5 w-3.5" />
          <ChevronLeft v-else class="h-3.5 w-3.5" />
        </button>
      </div>

      <!-- Navigation Groups -->
      <nav class="space-y-3.5">
        <div v-for="(sec, sIdx) in sections" :key="sIdx">
          <div
            v-if="sec.titleKey && !collapsed"
            class="mb-1 px-2.5 text-[11px] font-medium text-gray-400"
          >
            {{ t(sec.titleKey) }}
          </div>
          <div class="space-y-0.5">
            <button
              v-for="item in sec.items"
              :key="item.id"
              type="button"
              :title="collapsed ? t(item.labelKey) : undefined"
              class="flex w-full items-center rounded-lg py-1.5 text-left text-[13px] font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-[#0A7AFF]/40"
              :class="[
                collapsed ? 'justify-center px-1.5' : 'gap-2.5 px-2.5',
                store.activeRoute === item.id
                  ? 'bg-[#0A7AFF] text-white shadow-xs'
                  : 'text-gray-700 hover:bg-black/[0.045]',
              ]"
              @click="store.activeRoute = item.id"
            >
              <component
                :is="item.icon"
                class="h-4 w-4 shrink-0"
                :class="
                  store.activeRoute === item.id ? 'text-white' : 'text-[#0A7AFF]'
                "
              />
              <span v-if="!collapsed" class="truncate">{{ t(item.labelKey) }}</span>
            </button>
          </div>
        </div>
      </nav>
    </div>

    <!-- Footer Status & Theme/Skin & Mock Preview -->
    <div class="space-y-2 border-t border-gray-200/70 pt-3 dark:border-white/[0.06]">
      <div
        class="flex items-center rounded-lg bg-white/75 px-2 py-1.5 text-[11px] text-gray-600 dark:bg-[#1C1E24] dark:text-gray-300"
        :class="collapsed ? 'justify-center' : 'justify-between'"
      >
        <button
          type="button"
          :title="t('sidebar.themeCycleTitle')"
          class="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 font-medium text-gray-700 transition hover:bg-gray-100 dark:text-gray-200 dark:hover:bg-[#252830]"
          @click="cycleThemeMode"
        >
          <Moon v-if="themeMode === 'dark'" class="h-3.5 w-3.5 text-[#0A7AFF]" />
          <Monitor
            v-else-if="themeMode === 'system'"
            class="h-3.5 w-3.5 text-[#0A7AFF]"
          />
          <Sun v-else class="h-3.5 w-3.5 text-amber-500" />
          <span v-if="!collapsed">{{ themeBadgeLabel }}</span>
        </button>

        <div v-if="!collapsed" class="flex items-center gap-1.5">
          <button
            v-for="item in SKINS"
            :key="item.id"
            type="button"
            :title="`${t('sidebar.skinPrefix')}${item.label}`"
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
        v-if="!collapsed"
        class="flex items-center justify-between rounded-lg bg-white/75 px-2.5 py-1.5 text-[11px] text-gray-600 dark:bg-[#1C1E24]"
      >
        <span class="font-medium">{{ t("sidebar.mockPreview") }}</span>
        <ToggleSwitch
          size="sm"
          :model-value="store.mockMode"
          @update:model-value="store.setMockMode"
        />
      </div>

      <div
        class="flex items-center px-2 text-[11px] text-gray-500"
        :class="collapsed ? 'justify-center' : 'justify-between'"
      >
        <span
          class="flex items-center gap-1.5"
          :title="
            store.mockMode
              ? t('sidebar.offlinePreview')
              : store.pipeState === 'connected'
                ? t('sidebar.serviceConnected')
                : t('sidebar.serviceDisconnected')
          "
        >
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
          <span v-if="!collapsed">
            {{
              store.mockMode
                ? t("sidebar.offlinePreview")
                : store.pipeState === "connected"
                  ? t("sidebar.serviceConnected")
                  : t("sidebar.serviceDisconnected")
            }}
          </span>
        </span>
        <span v-if="!collapsed" class="inline-flex items-center gap-1 text-gray-400">
          <Github class="h-3.5 w-3.5" />
          <span>v0.5</span>
        </span>
      </div>
    </div>
  </aside>
</template>
