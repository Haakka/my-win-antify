<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch } from "vue";
import { Eye, EyeOff, Globe } from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
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
  interfaceName: "",
});

const showPassword = ref(false);
const validationError = ref<string | null>(null);

const protocolOptions: SelectOption[] = [
  { value: "socks5", label: "SOCKS5", description: "通用 SOCKS5 代理协议" },
  { value: "http", label: "HTTP (M2)", description: "将在 Milestone 2 支持", disabled: true },
];

const interfaceOptions = computed<SelectOption[]>(() => {
  const options: SelectOption[] = [
    { value: "", label: "自动", description: "由 Windows 系统路由表自动选择" },
  ];
  const seen = new Set<string>();
  for (const iface of store.networkInterfaces) {
    seen.add(iface.name);
    options.push({
      value: iface.name,
      label: iface.name,
      description: `${iface.ipv4}${iface.isUp ? "" : " (未连接)"} · ${iface.description}`,
    });
  }
  if (form.interfaceName && !seen.has(form.interfaceName)) {
    options.push({
      value: form.interfaceName,
      label: form.interfaceName,
      description: "当前不在线",
    });
  }
  return options;
});

const canSubmit = computed(() => {
  const portNum = Number(form.port);
  return (
    form.name.trim().length > 0 &&
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
      form.port = String(item.port);
      form.username = item.auth?.username ?? "";
      form.password = item.auth?.password ?? "";
      form.udpRelay = item.udpRelay;
      form.fallbackDirect = item.fallbackDirect;
      form.allowLan = item.allowLan;
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
      form.interfaceName = "";
    }
  },
  { immediate: true },
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
  const host = form.host.trim();
  const port = Number(form.port);

  if (!name) {
    validationError.value = "请输入代理名称";
    return;
  }
  if (!host) {
    validationError.value = "请输入服务器地址";
    return;
  }
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    validationError.value = "端口范围需在 1 ~ 65535";
    return;
  }

  const username = form.username.trim();
  const password = form.password;
  if ((username.length > 0) !== (password.length > 0)) {
    validationError.value = "填写认证信息时，用户名与密码需同时填写";
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
    allowLan: form.allowLan,
    udpRelay: form.udpRelay,
    fallbackDirect: form.fallbackDirect,
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
        <!-- Header -->
        <div class="border-b border-gray-200/75 px-5 py-3.5">
          <h3 class="text-[15px] font-semibold text-gray-900">
            {{ editing ? "编辑代理" : "添加代理" }}
          </h3>
        </div>

        <!-- Body -->
        <div class="space-y-3.5 overflow-y-auto px-5 py-4">
          <div
            v-if="validationError"
            class="rounded-xl bg-red-50 px-3.5 py-2 text-[12px] text-red-600"
          >
            {{ validationError }}
          </div>

          <!-- Group 1: 名称 / 类型 -->
          <div class="divide-y divide-gray-200/75 rounded-2xl bg-[#F5F5F7] px-4">
            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <label class="shrink-0 text-[13px] font-medium text-gray-800">
                名称
              </label>
              <input
                v-model="form.name"
                type="text"
                autofocus
                placeholder="例如：FlClash"
                class="h-8 w-[220px] rounded-lg border border-gray-200/90 bg-white px-3 text-[13px] text-gray-900 outline-none transition focus:border-[#0A7AFF] focus:ring-[3px] focus:ring-[#0A7AFF]/25"
              />
            </div>

            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <label class="shrink-0 text-[13px] font-medium text-gray-800">
                类型
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

          <!-- Group 2: 服务器地址 / 端口 -->
          <div class="divide-y divide-gray-200/75 rounded-2xl bg-[#F5F5F7] px-4">
            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <label class="shrink-0 text-[13px] font-medium text-gray-800">
                服务器地址
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
                端口
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

          <!-- Group 3: 用户名 / 密码（可选） -->
          <div class="divide-y divide-gray-200/75 rounded-2xl bg-[#F5F5F7] px-4">
            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <label class="shrink-0 text-[13px] font-medium text-gray-800">
                用户名（可选）
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
                密码（可选）
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
                  :title="showPassword ? '隐藏密码' : '显示密码'"
                  class="flex h-7 w-7 items-center justify-center rounded-md text-gray-500 transition hover:bg-gray-200/70 hover:text-gray-800"
                  @click="showPassword = !showPassword"
                >
                  <EyeOff v-if="showPassword" class="h-4 w-4" />
                  <Eye v-else class="h-4 w-4" />
                </button>
              </div>
            </div>
          </div>

          <!-- Group 4: 高级策略开关 -->
          <div class="divide-y divide-gray-200/75 rounded-2xl bg-[#F5F5F7] px-4">
            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <div class="flex items-center gap-1.5">
                <span class="text-[13px] font-medium text-gray-800">UDP 转发</span>
                <HelpTooltip
                  text="允许通过 SOCKS5 UDP ASSOCIATE 转发该代理的 UDP 流量（如 DNS、游戏、QUIC；当前 M1 为 TCP 优先，UDP 转发将在 M2 生效）。"
                />
              </div>
              <ToggleSwitch v-model="form.udpRelay" size="sm" />
            </div>

            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <div class="flex items-center gap-1.5">
                <span class="text-[13px] font-medium text-gray-800">
                  代理失败时直连
                </span>
                <HelpTooltip
                  text="当上游代理连接超时或被拒绝时，自动回退为直连目标服务器；若对应规则开启了「网络锁」，则优先生效网络锁并拒绝回退。"
                />
              </div>
              <ToggleSwitch v-model="form.fallbackDirect" size="sm" />
            </div>

            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <div class="flex items-center gap-1.5">
                <span class="text-[13px] font-medium text-gray-800">
                  可访问私有网络
                </span>
                <HelpTooltip
                  text="允许经由该代理转发发往内网 / 局域网私有 IP 段（如 10.0.0.0/8、192.168.0.0/16）的流量。"
                />
              </div>
              <ToggleSwitch v-model="form.allowLan" size="sm" />
            </div>

            <div class="flex min-h-[44px] items-center justify-between gap-4 py-2">
              <div class="flex items-center gap-1.5">
                <span class="text-[13px] font-medium text-gray-800">网络接口</span>
                <HelpTooltip
                  text="指定连接上游代理时绑定的本机出站网卡；选择「自动」则由 Windows 系统路由表决定出口网卡（本地 127.0.0.1 代理无需绑定）。"
                />
              </div>
              <SelectMenu
                v-model="form.interfaceName"
                :options="interfaceOptions"
                variant="inline"
              />
            </div>
          </div>
        </div>

        <!-- Footer: left Cancel, right Save -->
        <div
          class="flex items-center justify-between border-t border-gray-200/75 bg-white px-5 py-3"
        >
          <button
            type="button"
            class="rounded-lg bg-[#EFEFF2] px-4 py-1.5 text-[13px] font-medium text-gray-800 transition hover:bg-[#E5E5EA]"
            @click="emit('close')"
          >
            取消
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
            保存
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
