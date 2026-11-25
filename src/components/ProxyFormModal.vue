<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch } from "vue";
import { AlertTriangle, Eye, EyeOff, Globe } from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { useI18n } from "@/i18n";
import type { ProxyConfig, ProxyProtocol } from "@/types";
import { randomId } from "@/utils/format";
import HelpTooltip from "./HelpTooltip.vue";
import SelectMenu, { type SelectOption } from "./SelectMenu.vue";
import ToggleSwitch from "./ToggleSwitch.vue";

const props = defineProps<{
  open: boolean;
  editing: ProxyConfig | null;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "save", proxy: ProxyConfig): void;
}>();

const store = useAppStore();
const { t } = useI18n();

const form = reactive({
  name: "",
  protocol: "socks5" as ProxyProtocol,
  host: "",
  port: "",
  username: "",
  password: "",
  udpRelay: true,
  fallbackDirect: false,
  allowLan: false,
  skipTlsVerify: false,
  interfaceName: "",
});

const showPassword = ref(false);
const validationError = ref<string | null>(null);

const isDirectInterface = computed(() => form.protocol === "directInterface");
const isHttps = computed(() => form.protocol === "https");
const isSocks5 = computed(() => form.protocol === "socks5");

const protocolOptions = computed<SelectOption[]>(() => [
  { value: "socks5", label: "SOCKS5", description: "TCP + UDP ASSOCIATE" },
  { value: "http", label: "HTTP", description: "HTTP CONNECT" },
  { value: "https", label: "HTTPS", description: "TLS + HTTP CONNECT" },
  {
    value: "directInterface",
    label: t("proxies.directInterfaceLabel"),
    description: t("proxyModal.interfaceDirectHelp"),
  },
]);

const interfaceOptions = computed<SelectOption[]>(() => {
  const options: SelectOption[] = isDirectInterface.value
    ? []
    : [{ value: "", label: t("common.auto") }];
  const seen = new Set<string>();
  for (const iface of store.networkInterfaces) {
    seen.add(iface.name);
    options.push({
      value: iface.name,
      label: iface.name,
      description: `${iface.ipv4}${iface.isUp ? "" : " (offline)"} · ${iface.description}`,
    });
  }
  if (form.interfaceName && !seen.has(form.interfaceName)) {
    options.push({
      value: form.interfaceName,
      label: form.interfaceName,
    });
  }
  return options;
});

const duplicateWarnings = computed(() => {
  const warnings: string[] = [];
  const nameLower = form.name.trim().toLowerCase();
  const hostLower = form.host.trim().toLowerCase();
  const portNum = Number(form.port);
  const others = store.config.proxies.filter((p) => p.id !== props.editing?.id);

  if (nameLower && others.some((p) => p.name.trim().toLowerCase() === nameLower)) {
    warnings.push(t("proxyModal.dupNameWarn"));
  }
  if (isDirectInterface.value) {
    if (
      form.interfaceName &&
      others.some(
        (p) =>
          p.protocol === "directInterface" &&
          p.interfaceName === form.interfaceName,
      )
    ) {
      warnings.push(t("proxyModal.dupEndpointWarn"));
    }
  } else if (
    hostLower &&
    Number.isInteger(portNum) &&
    portNum > 0 &&
    others.some(
      (p) =>
        p.protocol !== "directInterface" &&
        p.host.trim().toLowerCase() === hostLower &&
        p.port === portNum,
    )
  ) {
    warnings.push(t("proxyModal.dupEndpointWarn"));
  }
  return warnings;
});

const canSubmit = computed(() => {
  if (!form.name.trim()) return false;
  if (isDirectInterface.value) {
    return form.interfaceName.trim().length > 0;
  }
  const portNum = Number(form.port);
  return (
    form.host.trim().length > 0 &&
    Number.isInteger(portNum) &&
    portNum >= 1 &&
    portNum <= 65535
  );
});

watch(
  () => [props.open, props.editing] as const,
  ([isOpen, item]) => {
    if (!isOpen) return;
    validationError.value = null;
    showPassword.value = false;
    void store.refreshNetworkInterfaces();
    if (item) {
      form.name = item.name;
      form.protocol = item.protocol;
      form.host = item.host;
      form.port = item.port ? String(item.port) : "";
      form.username = item.auth?.username ?? "";
      form.password = item.auth?.password ?? "";
      form.udpRelay = item.udpRelay;
      form.fallbackDirect = item.fallbackDirect;
      form.allowLan = item.allowLan;
      form.skipTlsVerify = item.skipTlsVerify ?? false;
      form.interfaceName = item.interfaceName ?? "";
    } else {
      form.name = "";
      form.protocol = "socks5";
      form.host = "";
      form.port = "";
      form.username = "";
      form.password = "";
      form.udpRelay = true;
      form.fallbackDirect = false;
      form.allowLan = false;
      form.skipTlsVerify = false;
      form.interfaceName = "";
    }
  },
  { immediate: true },
);

watch(
  () => form.protocol,
  (proto) => {
    if (proto === "directInterface" && !form.interfaceName) {
      form.interfaceName = store.networkInterfaces[0]?.name ?? "";
    }
  },
);

function onKeyDown(ev: KeyboardEvent) {
  if (props.open && ev.key === "Escape") {
    emit("close");
  }
}

window.addEventListener("keydown", onKeyDown);
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeyDown);
});

function submit() {
  const name = form.name.trim();
  const host = isDirectInterface.value ? "" : form.host.trim();
  const port = isDirectInterface.value ? 0 : Number(form.port);

  if (!name) {
    validationError.value = t("proxyModal.errName");
    return;
  }
  if (isDirectInterface.value) {
    if (!form.interfaceName.trim()) {
      validationError.value = t("proxyModal.errNic");
      return;
    }
  } else {
    if (!host) {
      validationError.value = t("proxyModal.errHost");
      return;
    }
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      validationError.value = t("proxyModal.errPort");
      return;
    }
  }

  const username = isDirectInterface.value ? "" : form.username.trim();
  const password = isDirectInterface.value ? "" : form.password;
  if ((username.length > 0) !== (password.length > 0)) {
    validationError.value = t("proxyModal.errAuth");
    return;
  }

  emit("save", {
    id: props.editing?.id ?? randomId("proxy"),
    name,
    protocol: form.protocol,
    host,
    port,
    auth: username.length > 0 ? { username, password } : null,
    interfaceName: form.interfaceName.trim() || null,
    allowLan: isDirectInterface.value ? false : form.allowLan,
    udpRelay: isSocks5.value ? form.udpRelay : false,
    fallbackDirect: isDirectInterface.value ? false : form.fallbackDirect,
    skipTlsVerify: isHttps.value ? form.skipTlsVerify : false,
    enabled: props.editing?.enabled ?? true,
  });
}
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/20 p-4 backdrop-blur-[2px]"
      @mousedown.self="emit('close')"
    >
      <div
        class="flex max-h-[88vh] w-full max-w-[440px] flex-col overflow-hidden rounded-[18px] border border-gray-200/90 bg-white shadow-2xl"
      >
        <div class="border-b border-gray-200/75 px-5 py-3.5">
          <h3 class="text-[15px] font-semibold text-gray-900">
            {{ editing ? t("proxyModal.editTitle") : t("proxyModal.addTitle") }}
          </h3>
        </div>

        <div class="space-y-3.5 overflow-y-auto px-5 py-4">
          <div
            v-if="validationError"
            class="rounded-xl bg-red-50 px-3.5 py-2 text-[12px] text-red-600"
          >
            {{ validationError }}
          </div>

          <div
            v-for="warn in duplicateWarnings"
            :key="warn"
            class="flex items-center gap-2 rounded-xl bg-amber-50 px-3.5 py-2 text-[12px] text-amber-800"
          >
            <AlertTriangle class="h-3.5 w-3.5 shrink-0" />
            <span>{{ warn }}</span>
          </div>

          <div class="divide-y divide-gray-200/75 rounded-2xl bg-[#F5F5F7] px-4">
            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <label class="shrink-0 text-[13px] font-medium text-gray-800">
                {{ t("proxyModal.name") }}
              </label>
              <input
                v-model="form.name"
                type="text"
                autofocus
                :placeholder="t('proxyModal.namePlaceholder')"
                class="h-8 w-[220px] rounded-lg border border-gray-200/90 bg-white px-3 text-[13px] text-gray-900 outline-none transition focus:border-[#0A7AFF] focus:ring-[3px] focus:ring-[#0A7AFF]/25"
              />
            </div>

            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <label class="shrink-0 text-[13px] font-medium text-gray-800">
                {{ t("proxyModal.type") }}
              </label>
              <SelectMenu
                :model-value="form.protocol"
                :options="protocolOptions"
                variant="inline"
                @update:model-value="form.protocol = $event as ProxyProtocol"
              >
                <template #icon>
                  <Globe class="h-4 w-4 text-gray-600" />
                </template>
              </SelectMenu>
            </div>
          </div>

          <div
            v-if="!isDirectInterface"
            class="divide-y divide-gray-200/75 rounded-2xl bg-[#F5F5F7] px-4"
          >
            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <label class="shrink-0 text-[13px] font-medium text-gray-800">
                {{ t("proxyModal.host") }}
              </label>
              <input
                v-model="form.host"
                type="text"
                placeholder="127.0.0.1"
                class="h-8 w-[220px] rounded-lg border border-gray-200/90 bg-white px-3 text-[13px] text-gray-900 outline-none transition focus:border-[#0A7AFF] focus:ring-[3px] focus:ring-[#0A7AFF]/25"
              />
            </div>

            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <label class="shrink-0 text-[13px] font-medium text-gray-800">
                {{ t("proxyModal.port") }}
              </label>
              <input
                v-model="form.port"
                type="number"
                min="1"
                max="65535"
                placeholder="7890"
                class="h-8 w-[220px] rounded-lg border border-gray-200/90 bg-white px-3 text-[13px] text-gray-900 outline-none transition focus:border-[#0A7AFF] focus:ring-[3px] focus:ring-[#0A7AFF]/25"
              />
            </div>
          </div>

          <div
            v-if="!isDirectInterface"
            class="divide-y divide-gray-200/75 rounded-2xl bg-[#F5F5F7] px-4"
          >
            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <label class="shrink-0 text-[13px] font-medium text-gray-800">
                {{ t("proxyModal.username") }}
              </label>
              <input
                v-model="form.username"
                type="text"
                autocomplete="off"
                class="h-8 w-[220px] rounded-lg border border-gray-200/90 bg-white px-3 text-[13px] text-gray-900 outline-none transition focus:border-[#0A7AFF] focus:ring-[3px] focus:ring-[#0A7AFF]/25"
              />
            </div>

            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <label class="shrink-0 text-[13px] font-medium text-gray-800">
                {{ t("proxyModal.password") }}
              </label>
              <div class="flex items-center gap-2">
                <input
                  v-model="form.password"
                  :type="showPassword ? 'text' : 'password'"
                  autocomplete="new-password"
                  class="h-8 w-[190px] rounded-lg border border-gray-200/90 bg-white px-3 text-[13px] text-gray-900 outline-none transition focus:border-[#0A7AFF] focus:ring-[3px] focus:ring-[#0A7AFF]/25"
                />
                <button
                  type="button"
                  :title="showPassword ? t('proxyModal.hidePassword') : t('proxyModal.showPassword')"
                  class="flex h-7 w-7 items-center justify-center rounded-md text-gray-500 transition hover:bg-gray-200/70 hover:text-gray-800"
                  @click="showPassword = !showPassword"
                >
                  <EyeOff v-if="showPassword" class="h-4 w-4" />
                  <Eye v-else class="h-4 w-4" />
                </button>
              </div>
            </div>
          </div>

          <div class="divide-y divide-gray-200/75 rounded-2xl bg-[#F5F5F7] px-4">
            <div
              v-if="isHttps"
              class="flex min-h-[44px] items-center justify-between gap-4 py-2"
            >
              <div class="flex items-center gap-1.5">
                <span class="text-[13px] font-medium text-gray-800">
                  {{ t("proxyModal.skipTlsVerify") }}
                </span>
                <HelpTooltip :text="t('proxyModal.skipTlsVerifyHelp')" />
              </div>
              <ToggleSwitch v-model="form.skipTlsVerify" size="sm" />
            </div>

            <div
              v-if="isSocks5"
              class="flex min-h-[44px] items-center justify-between gap-4 py-2"
            >
              <div class="flex items-center gap-1.5">
                <span class="text-[13px] font-medium text-gray-800">
                  {{ t("proxyModal.udpRelay") }}
                </span>
                <HelpTooltip :text="t('proxyModal.udpRelayHelp')" />
              </div>
              <ToggleSwitch v-model="form.udpRelay" size="sm" />
            </div>

            <div
              v-if="!isDirectInterface"
              class="flex min-h-[44px] items-center justify-between gap-4 py-2"
            >
              <div class="flex items-center gap-1.5">
                <span class="text-[13px] font-medium text-gray-800">
                  {{ t("proxyModal.fallbackDirect") }}
                </span>
                <HelpTooltip :text="t('proxyModal.fallbackDirectHelp')" />
              </div>
              <ToggleSwitch v-model="form.fallbackDirect" size="sm" />
            </div>

            <div
              v-if="!isDirectInterface"
              class="flex min-h-[44px] items-center justify-between gap-4 py-2"
            >
              <div class="flex items-center gap-1.5">
                <span class="text-[13px] font-medium text-gray-800">
                  {{ t("proxyModal.allowLan") }}
                </span>
                <HelpTooltip :text="t('proxyModal.allowLanHelp')" />
              </div>
              <ToggleSwitch v-model="form.allowLan" size="sm" />
            </div>

            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <div class="flex items-center gap-1.5">
                <span class="text-[13px] font-medium text-gray-800">
                  {{ t("proxyModal.interface") }}
                </span>
                <HelpTooltip
                  :text="
                    isDirectInterface
                      ? t('proxyModal.interfaceDirectHelp')
                      : t('proxyModal.interfaceProxyHelp')
                  "
                />
              </div>
              <SelectMenu
                v-model="form.interfaceName"
                :options="interfaceOptions"
                :placeholder="isDirectInterface ? t('proxyModal.interfacePlaceholder') : t('common.auto')"
                variant="inline"
              />
            </div>
          </div>
        </div>

        <div
          class="flex items-center justify-between border-t border-gray-200/75 bg-white px-5 py-3"
        >
          <button
            type="button"
            class="rounded-lg bg-[#EFEFF2] px-4 py-1.5 text-[13px] font-medium text-gray-800 transition hover:bg-[#E5E5EA]"
            @click="emit('close')"
          >
            {{ t("common.cancel") }}
          </button>
          <button
            type="button"
            :disabled="!canSubmit"
            class="rounded-lg px-4 py-1.5 text-[13px] font-medium transition"
            :class="
              canSubmit
                ? 'bg-[#0A7AFF] text-white hover:bg-blue-600'
                : 'cursor-not-allowed bg-[#F3F4F6] text-gray-400'
            "
            @click="submit"
          >
            {{ t("common.save") }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
