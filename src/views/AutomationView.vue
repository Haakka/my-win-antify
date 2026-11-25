<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { Plus, Radio, RefreshCw, Trash2, Wifi } from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { useI18n } from "@/i18n";
import type { WifiProfileRule } from "@/types";
import { randomId } from "@/utils/format";
import SelectMenu, { type SelectOption } from "@/components/SelectMenu.vue";
import ToggleSwitch from "@/components/ToggleSwitch.vue";

const store = useAppStore();
const { t } = useI18n();

const newSsid = ref("");
const newProfileId = ref("");
const formError = ref<string | null>(null);

const profileOptions = computed<SelectOption[]>(() =>
  store.config.profiles.map((p) => ({
    value: p.id,
    label: p.name,
  })),
);

onMounted(() => {
  if (!newProfileId.value) {
    newProfileId.value = store.config.profiles[0]?.id ?? "default";
  }
  void store.refreshCurrentWifi();
});

function profileName(profileId: string): string {
  return (
    store.config.profiles.find((p) => p.id === profileId)?.name ?? profileId
  );
}

async function toggleAutomation(enabled: boolean) {
  await store.updateAutomation({
    ...store.config.automation,
    enabled,
  });
}

function useCurrentSsid() {
  if (store.currentWifi.ssid) {
    newSsid.value = store.currentWifi.ssid;
    formError.value = null;
  }
}

async function addWifiRule() {
  const ssid = newSsid.value.trim();
  const profileId = newProfileId.value || store.config.profiles[0]?.id;
  if (!ssid) {
    formError.value = t("automation.errSsid");
    return;
  }
  if (!profileId) {
    formError.value = t("automation.errProfile");
    return;
  }
  formError.value = null;
  const nextRules: WifiProfileRule[] = [
    ...store.config.automation.wifiRules,
    {
      id: randomId("wifi"),
      ssid,
      profileId,
      enabled: true,
    },
  ];
  await store.updateAutomation({
    ...store.config.automation,
    wifiRules: nextRules,
  });
  newSsid.value = "";
}

async function toggleWifiRule(ruleId: string, enabled: boolean) {
  await store.updateAutomation({
    ...store.config.automation,
    wifiRules: store.config.automation.wifiRules.map((r) =>
      r.id === ruleId ? { ...r, enabled } : r,
    ),
  });
}

async function updateRuleProfile(ruleId: string, profileId: string) {
  await store.updateAutomation({
    ...store.config.automation,
    wifiRules: store.config.automation.wifiRules.map((r) =>
      r.id === ruleId ? { ...r, profileId } : r,
    ),
  });
}

async function deleteWifiRule(ruleId: string) {
  await store.updateAutomation({
    ...store.config.automation,
    wifiRules: store.config.automation.wifiRules.filter((r) => r.id !== ruleId),
  });
}
</script>

<template>
  <div class="space-y-4">
    <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
      {{ t("automation.title") }}
    </h1>

    <div class="antify-card flex items-center justify-between px-5 py-3.5">
      <div>
        <div class="text-[13.5px] font-medium text-gray-900">
          {{ t("automation.autoSwitch") }}
        </div>
        <div class="text-[11.5px] text-gray-500">
          {{ t("automation.autoSwitchDesc") }}
        </div>
      </div>
      <ToggleSwitch
        :model-value="store.config.automation.enabled"
        size="sm"
        @update:model-value="toggleAutomation"
      />
    </div>

    <div class="antify-card flex flex-wrap items-center justify-between gap-3 px-5 py-3.5">
      <div class="flex items-center gap-3">
        <div
          class="flex h-9 w-9 items-center justify-center rounded-xl bg-sky-50 text-[#0A7AFF] dark:bg-sky-500/15"
        >
          <Wifi class="h-4.5 w-4.5" />
        </div>
        <div>
          <div class="text-[13px] font-semibold text-gray-900">
            {{ t("automation.currentWifi") }}
            <span
              v-if="store.currentWifi.connected && store.currentWifi.ssid"
              class="text-[#0A7AFF]"
            >
              {{ store.currentWifi.ssid }}
            </span>
            <span v-else-if="!store.currentWifi.supported" class="text-gray-400">
              {{ t("automation.wlanUnsupported") }}
            </span>
            <span v-else class="text-gray-500">
              {{ t("automation.wlanDisconnected") }}
            </span>
          </div>
          <div class="text-[11.5px] text-gray-500">
            {{
              store.currentWifi.interfaceName
                ? t("automation.ifacePrefix", { name: store.currentWifi.interfaceName })
                : t("automation.wlanHint")
            }}
          </div>
        </div>
      </div>

      <div class="flex items-center gap-2">
        <button
          v-if="store.currentWifi.connected && store.currentWifi.ssid"
          type="button"
          class="rounded-lg bg-blue-50 px-2.5 py-1 text-[12px] font-medium text-[#0A7AFF] transition hover:bg-blue-100 dark:bg-blue-500/15"
          @click="useCurrentSsid"
        >
          {{ t("automation.fillSsid") }}
        </button>
        <button
          type="button"
          :title="t('automation.refreshWifi')"
          class="rounded-lg border border-gray-200 bg-white p-1.5 text-gray-500 transition hover:bg-gray-50 hover:text-gray-800"
          @click="store.refreshCurrentWifi()"
        >
          <RefreshCw class="h-3.5 w-3.5" />
        </button>
      </div>
    </div>

    <div class="antify-card space-y-3 p-5">
      <div class="flex items-center gap-2 text-[13px] font-semibold text-gray-900">
        <Radio class="h-4 w-4 text-[#00B4D8]" />
        <span>{{ t("automation.addTitle") }}</span>
      </div>

      <div class="flex flex-wrap items-center gap-2.5">
        <input
          v-model="newSsid"
          type="text"
          :placeholder="t('automation.ssidPlaceholder')"
          class="h-8.5 min-w-[220px] flex-1 rounded-lg border border-gray-200 bg-white px-3 text-[12.5px] outline-none focus:border-[#0A7AFF]"
          @keydown.enter="addWifiRule"
        />
        <SelectMenu
          v-model="newProfileId"
          :options="profileOptions"
          variant="pill"
          :placeholder="t('automation.selectProfile')"
        />
        <button
          type="button"
          :disabled="store.busySaving"
          class="inline-flex items-center gap-1.5 rounded-lg bg-[#0A7AFF] px-3.5 py-1.5 text-[12.5px] font-medium text-white transition hover:bg-blue-600 disabled:opacity-50"
          @click="addWifiRule"
        >
          <Plus class="h-3.5 w-3.5" />
          {{ t("automation.addRule") }}
        </button>
      </div>

      <div v-if="formError" class="text-[12px] text-red-600">
        {{ formError }}
      </div>
    </div>

    <div class="antify-card divide-y divide-gray-100 overflow-hidden">
      <div
        v-for="rule in store.config.automation.wifiRules"
        :key="rule.id"
        class="flex items-center justify-between gap-3 px-5 py-3 text-[12.5px]"
      >
        <div class="flex min-w-0 items-center gap-3">
          <ToggleSwitch
            size="sm"
            :model-value="rule.enabled"
            @update:model-value="(val) => toggleWifiRule(rule.id, val)"
          />
          <div class="min-w-0">
            <div class="truncate font-semibold text-gray-900">{{ rule.ssid }}</div>
            <div class="text-[11.5px] text-gray-400">
              {{ t("automation.targetProfile", { name: profileName(rule.profileId) }) }}
            </div>
          </div>
        </div>

        <div class="flex items-center gap-2.5">
          <SelectMenu
            :model-value="rule.profileId"
            :options="profileOptions"
            variant="pill"
            @update:model-value="(pid) => updateRuleProfile(rule.id, String(pid))"
          />
          <button
            type="button"
            :title="t('automation.deleteRule')"
            class="rounded-md p-1.5 text-red-400 transition hover:bg-red-50 hover:text-red-600"
            @click="deleteWifiRule(rule.id)"
          >
            <Trash2 class="h-4 w-4" />
          </button>
        </div>
      </div>

      <div
        v-if="store.config.automation.wifiRules.length === 0"
        class="py-10 text-center text-[13px] text-gray-400"
      >
        {{ t("automation.empty") }}
      </div>
    </div>
  </div>
</template>
