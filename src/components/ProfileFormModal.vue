<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { AlertTriangle } from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { useI18n } from "@/i18n";
import type { Profile, RuleAction } from "@/types";
import { randomId } from "@/utils/format";
import ModalDialog from "./ModalDialog.vue";

const props = defineProps<{
  open: boolean;
  editing: Profile | null;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "save", profile: Profile): void;
}>();

const store = useAppStore();
const { t } = useI18n();

const form = reactive({
  name: "",
  defaultAction: "direct" as RuleAction,
  defaultProxyId: "",
});
const validationError = ref<string | null>(null);

const duplicateNameWarn = computed(() => {
  const nameLower = form.name.trim().toLowerCase();
  if (!nameLower) return false;
  return store.config.profiles.some(
    (p) => p.id !== props.editing?.id && p.name.trim().toLowerCase() === nameLower,
  );
});

watch(
  () => [props.open, props.editing] as const,
  ([isOpen, item]) => {
    if (!isOpen) return;
    validationError.value = null;
    if (item) {
      form.name = item.name;
      form.defaultAction = item.defaultAction;
      form.defaultProxyId = item.defaultProxyId ?? store.config.proxies[0]?.id ?? "";
    } else {
      form.name = "";
      form.defaultAction = "direct";
      form.defaultProxyId = store.config.proxies[0]?.id ?? "";
    }
  },
  { immediate: true },
);

function submit() {
  if (!form.name.trim()) {
    validationError.value = t("profiles.errName");
    return;
  }
  emit("save", {
    id: props.editing?.id ?? randomId("profile"),
    name: form.name.trim(),
    defaultAction: form.defaultAction,
    defaultProxyId:
      form.defaultAction === "proxy" && form.defaultProxyId
        ? form.defaultProxyId
        : null,
  });
}
</script>

<template>
  <ModalDialog
    :open="open"
    :title="editing ? t('profiles.modalEdit') : t('profiles.modalAdd')"
    @close="emit('close')"
  >
    <div class="space-y-4">
      <div
        v-if="validationError"
        class="rounded-lg bg-red-50 px-3 py-2 text-[12px] text-red-600"
      >
        {{ validationError }}
      </div>

      <div
        v-if="duplicateNameWarn"
        class="flex items-center gap-2 rounded-lg bg-amber-50 px-3 py-2 text-[12px] text-amber-800"
      >
        <AlertTriangle class="h-3.5 w-3.5 shrink-0" />
        <span>{{ t("profiles.dupNameWarn") }}</span>
      </div>

      <div>
        <label class="mb-1 block text-[12px] font-medium text-gray-600">
          {{ t("profiles.nameLabel") }}
        </label>
        <input
          v-model="form.name"
          type="text"
          :placeholder="t('profiles.namePlaceholder')"
          class="w-full rounded-lg border border-gray-200 px-3 py-2 text-[13px] outline-none focus:border-[#0A7AFF]"
        />
      </div>

      <div>
        <label class="mb-1 block text-[12px] font-medium text-gray-600">
          {{ t("profiles.defaultStrategy") }}
        </label>
        <select
          v-model="form.defaultAction"
          class="w-full rounded-lg border border-gray-200 bg-white px-3 py-2 text-[13px] outline-none focus:border-[#0A7AFF]"
        >
          <option value="direct">{{ t("profiles.strategyDirect") }}</option>
          <option value="proxy">{{ t("profiles.strategyProxy") }}</option>
          <option value="block">{{ t("profiles.strategyBlock") }}</option>
        </select>
      </div>

      <div v-if="form.defaultAction === 'proxy'">
        <label class="mb-1 block text-[12px] font-medium text-gray-600">
          {{ t("profiles.defaultProxy") }}
        </label>
        <select
          v-model="form.defaultProxyId"
          class="w-full rounded-lg border border-gray-200 bg-white px-3 py-2 text-[13px] outline-none focus:border-[#0A7AFF]"
        >
          <option
            v-for="proxy in store.config.proxies"
            :key="proxy.id"
            :value="proxy.id"
          >
            {{
              proxy.protocol === "directInterface"
                ? `${proxy.name} (${proxy.interfaceName ?? "NIC"})`
                : `${proxy.name} (${proxy.host}:${proxy.port})`
            }}
          </option>
        </select>
      </div>
    </div>

    <template #footer>
      <button
        type="button"
        class="rounded-lg border border-gray-200 bg-white px-3.5 py-1.5 text-[12.5px] font-medium text-gray-700 hover:bg-gray-50"
        @click="emit('close')"
      >
        {{ t("common.cancel") }}
      </button>
      <button
        type="button"
        class="rounded-lg bg-[#0A7AFF] px-4 py-1.5 text-[12.5px] font-medium text-white hover:bg-blue-600"
        @click="submit"
      >
        {{ t("common.save") }}
      </button>
    </template>
  </ModalDialog>
</template>
