<script setup lang="ts">
import { reactive, ref, watch } from "vue";
import { useAppStore } from "@/stores/app";
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

const form = reactive({
  name: "",
  defaultAction: "direct" as RuleAction,
  defaultProxyId: "",
});
const validationError = ref<string | null>(null);

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
    validationError.value = "请输入配置名称";
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
    :title="editing ? '编辑配置' : '新增配置'"
    @close="emit('close')"
  >
    <div class="space-y-4">
      <div
        v-if="validationError"
        class="rounded-lg bg-red-50 px-3 py-2 text-[12px] text-red-600"
      >
        {{ validationError }}
      </div>

      <div>
        <label class="mb-1 block text-[12px] font-medium text-gray-600">
          配置名称
        </label>
        <input
          v-model="form.name"
          type="text"
          placeholder="例如：Work / General"
          class="w-full rounded-lg border border-gray-200 px-3 py-2 text-[13px] outline-none focus:border-[#0A7AFF]"
        />
      </div>

      <div>
        <label class="mb-1 block text-[12px] font-medium text-gray-600">
          未命中规则时的默认策略
        </label>
        <select
          v-model="form.defaultAction"
          class="w-full rounded-lg border border-gray-200 bg-white px-3 py-2 text-[13px] outline-none focus:border-[#0A7AFF]"
        >
          <option value="direct">直连 (Direct)</option>
          <option value="proxy">代理 (Proxy)</option>
          <option value="block">阻断 (TCP RST)</option>
        </select>
      </div>

      <div v-if="form.defaultAction === 'proxy'">
        <label class="mb-1 block text-[12px] font-medium text-gray-600">
          默认代理节点
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
            {{ proxy.name }} ({{ proxy.host }}:{{ proxy.port }})
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
        取消
      </button>
      <button
        type="button"
        class="rounded-lg bg-[#0A7AFF] px-4 py-1.5 text-[12.5px] font-medium text-white hover:bg-blue-600"
        @click="submit"
      >
        保存
      </button>
    </template>
  </ModalDialog>
</template>
