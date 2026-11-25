<script setup lang="ts">
import { onMounted, watch } from "vue";
import { useAppStore } from "@/stores/app";
import { useTheme } from "@/composables/useTheme";
import type { Profile, ProxyConfig, Rule } from "@/types";
import SidebarNav from "@/components/SidebarNav.vue";
import OfflineBanner from "@/components/OfflineBanner.vue";
import ProxyFormModal from "@/components/ProxyFormModal.vue";
import RuleFormModal from "@/components/RuleFormModal.vue";
import ProfileFormModal from "@/components/ProfileFormModal.vue";

import DashboardView from "@/views/DashboardView.vue";
import ProxiesView from "@/views/ProxiesView.vue";
import RulesView from "@/views/RulesView.vue";
import ProfilesView from "@/views/ProfilesView.vue";
import AutomationView from "@/views/AutomationView.vue";
import ConnectionsView from "@/views/ConnectionsView.vue";
import TrafficView from "@/views/TrafficView.vue";
import ActivityView from "@/views/ActivityView.vue";
import SettingsView from "@/views/SettingsView.vue";

const store = useAppStore();
const { applyThemeMode } = useTheme();

watch(
  () => store.config.settings.theme,
  (mode) => {
    applyThemeMode(mode);
  },
  { immediate: true },
);

onMounted(() => {
  void store.init();
});

async function handleQuickSaveProxy(proxy: ProxyConfig) {
  await store.upsertProxy(proxy);
  store.quickAddProxyOpen = false;
}

async function handleQuickSaveRule(rule: Rule) {
  await store.upsertRule(rule);
  store.quickAddRuleOpen = false;
  store.quickAddRulePrefill = null;
}

function handleQuickCloseRule() {
  store.quickAddRuleOpen = false;
  store.quickAddRulePrefill = null;
}

async function handleQuickSaveProfile(profile: Profile) {
  await store.upsertProfile(profile);
  store.quickAddProfileOpen = false;
}
</script>

<template>
  <div
    class="flex h-screen w-screen overflow-hidden bg-[var(--bg-canvas)] text-[var(--text-primary)] transition-colors duration-200"
  >
    <SidebarNav />

    <main class="flex-1 overflow-y-auto px-7 py-5">
      <div class="mx-auto max-w-[1040px]">
        <OfflineBanner />

        <div
          v-if="store.errorMessage"
          class="mb-4 rounded-xl border border-red-200 bg-red-50 px-4 py-2.5 text-[12.5px] text-red-700"
        >
          {{ store.errorMessage }}
        </div>

        <DashboardView v-if="store.activeRoute === 'dashboard'" />
        <ProxiesView v-else-if="store.activeRoute === 'proxies'" />
        <RulesView v-else-if="store.activeRoute === 'rules'" />
        <ProfilesView v-else-if="store.activeRoute === 'profiles'" />
        <AutomationView v-else-if="store.activeRoute === 'automation'" />
        <ConnectionsView v-else-if="store.activeRoute === 'connections'" />
        <TrafficView v-else-if="store.activeRoute === 'traffic'" />
        <ActivityView v-else-if="store.activeRoute === 'activity'" />
        <SettingsView v-else-if="store.activeRoute === 'settings'" />
      </div>
    </main>

    <!-- Quick-Add Modals from Dashboard -->
    <ProxyFormModal
      :open="store.quickAddProxyOpen"
      :editing="null"
      @close="store.quickAddProxyOpen = false"
      @save="handleQuickSaveProxy"
    />
    <RuleFormModal
      :open="store.quickAddRuleOpen"
      :editing="null"
      :prefill="store.quickAddRulePrefill"
      @close="handleQuickCloseRule"
      @save="handleQuickSaveRule"
    />
    <ProfileFormModal
      :open="store.quickAddProfileOpen"
      :editing="null"
      @close="store.quickAddProfileOpen = false"
      @save="handleQuickSaveProfile"
    />
  </div>
</template>
