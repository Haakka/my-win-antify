<script setup lang="ts">
import { computed, ref } from "vue";
import { CheckCircle2, ListChecks, Plus, Server, ShieldCheck, X } from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { useI18n } from "@/i18n";

const DISMISS_KEY = "my-win-antify:onboarding-dismissed";

const store = useAppStore();
const { t } = useI18n();
const dismissed = ref(
  typeof localStorage !== "undefined" && localStorage.getItem(DISMISS_KEY) === "1",
);

const shouldShow = computed(
  () =>
    !dismissed.value &&
    store.config.proxies.length === 0 &&
    store.config.rules.length === 0,
);

function dismiss() {
  dismissed.value = true;
  try {
    localStorage.setItem(DISMISS_KEY, "1");
  } catch {
    // Ignore storage errors.
  }
}

const driverOk = computed(
  () => store.isServiceConnected && (store.mockMode || store.status.driverReady || store.status.servicePid > 0),
);
</script>

<template>
  <div
    v-if="shouldShow"
    class="antify-card border border-[#0A7AFF]/25 bg-blue-50/40 p-4 dark:bg-blue-950/20"
  >
    <div class="flex items-start justify-between gap-3">
      <div>
        <div class="text-[14px] font-semibold text-gray-900">
          {{ t("onboarding.title") }}
        </div>
        <div class="mt-0.5 text-[12px] text-gray-500">
          {{ t("onboarding.subtitle") }}
        </div>
      </div>
      <button
        type="button"
        class="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-[11.5px] text-gray-500 transition hover:bg-black/5 hover:text-gray-800"
        @click="dismiss"
      >
        <X class="h-3.5 w-3.5" />
        {{ t("onboarding.skip") }}
      </button>
    </div>

    <div class="mt-3 grid grid-cols-1 gap-2.5 md:grid-cols-3">
      <div class="rounded-xl bg-white/85 p-3 dark:bg-[#1C1E24]">
        <div class="flex items-center gap-2 text-[12.5px] font-semibold text-gray-900">
          <CheckCircle2
            v-if="driverOk"
            class="h-4 w-4 text-emerald-500"
          />
          <ShieldCheck v-else class="h-4 w-4 text-amber-500" />
          <span>{{ t("onboarding.step1Title") }}</span>
        </div>
        <div class="mt-1 text-[11.5px] text-gray-500">
          {{ driverOk ? t("onboarding.step1Done") : t("onboarding.step1Pending") }}
        </div>
      </div>

      <div class="flex flex-col justify-between rounded-xl bg-white/85 p-3 dark:bg-[#1C1E24]">
        <div>
          <div class="flex items-center gap-2 text-[12.5px] font-semibold text-gray-900">
            <Server class="h-4 w-4 text-[#0A7AFF]" />
            <span>{{ t("onboarding.step2Title") }}</span>
          </div>
          <div class="mt-1 text-[11.5px] text-gray-500">
            {{ t("onboarding.step2Desc") }}
          </div>
        </div>
        <button
          type="button"
          class="mt-2.5 inline-flex w-fit items-center gap-1 rounded-lg bg-[#0A7AFF] px-2.5 py-1 text-[11.5px] font-medium text-white transition hover:bg-blue-600"
          @click="store.quickAddProxyOpen = true"
        >
          <Plus class="h-3.5 w-3.5" />
          {{ t("onboarding.step2Action") }}
        </button>
      </div>

      <div class="flex flex-col justify-between rounded-xl bg-white/85 p-3 dark:bg-[#1C1E24]">
        <div>
          <div class="flex items-center gap-2 text-[12.5px] font-semibold text-gray-900">
            <ListChecks class="h-4 w-4 text-[#AF52DE]" />
            <span>{{ t("onboarding.step3Title") }}</span>
          </div>
          <div class="mt-1 text-[11.5px] text-gray-500">
            {{ t("onboarding.step3Desc") }}
          </div>
        </div>
        <button
          type="button"
          class="mt-2.5 inline-flex w-fit items-center gap-1 rounded-lg bg-[#0A7AFF] px-2.5 py-1 text-[11.5px] font-medium text-white transition hover:bg-blue-600"
          @click="store.quickAddRuleOpen = true"
        >
          <Plus class="h-3.5 w-3.5" />
          {{ t("onboarding.step3Action") }}
        </button>
      </div>
    </div>
  </div>
</template>
