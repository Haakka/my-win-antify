<script setup lang="ts">
import { ref, watch } from "vue";
import { Check, Monitor, Moon, Sun } from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { SKINS, useTheme } from "@/composables/useTheme";
import type { ThemeMode } from "@/types";
import HelpTooltip from "@/components/HelpTooltip.vue";
import ToggleSwitch from "@/components/ToggleSwitch.vue";

const store = useAppStore();
const { skin, setSkin, setThemeMode } = useTheme();
const activeTab = ref<"general" | "permissions" | "about">("general");
const bypassHostsText = ref("");

const themeModes: { value: ThemeMode; label: string; icon: typeof Sun }[] = [
  { value: "light", label: "浅色", icon: Sun },
  { value: "dark", label: "深色", icon: Moon },
  { value: "system", label: "跟随系统", icon: Monitor },
];

watch(
  () => store.config.settings.bypassHosts,
  (hosts) => {
    bypassHostsText.value = hosts.join("; ");
  },
  { immediate: true },
);

async function saveBypassHosts() {
  const list = bypassHostsText.value
    .split(/[;\n,]/)
    .map((s) => s.trim())
    .filter(Boolean);
  await store.updateSettings({ bypassHosts: list });
}
</script>

<template>
  <div class="space-y-4">
    <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
      设置
    </h1>

    <div class="flex justify-center">
      <div class="inline-flex rounded-lg bg-gray-200/70 p-0.5 text-[12.5px]">
        <button
          v-for="tab in [
            { id: 'general', label: '通用' },
            { id: 'permissions', label: '权限与驱动' },
            { id: 'about', label: '关于' },
          ]"
          :key="tab.id"
          type="button"
          class="rounded-md px-3.5 py-1 font-medium transition"
          :class="
            activeTab === tab.id
              ? 'bg-white text-gray-900 shadow-2xs'
              : 'text-gray-600 hover:text-gray-900'
          "
          @click="activeTab = tab.id as typeof activeTab"
        >
          {{ tab.label }}
        </button>
      </div>
    </div>

    <div v-if="activeTab === 'general'" class="space-y-3">
      <div class="antify-card divide-y divide-gray-100 px-5">
        <div class="flex items-center justify-between py-3.5">
          <div>
            <div class="text-[13px] font-medium text-gray-800">外观模式</div>
            <div class="text-[11.5px] text-gray-500">
              深色模式采用 FlClash 炭黑高对比配色，支持立即切换
            </div>
          </div>
          <div class="inline-flex rounded-lg bg-[#EFEFF2] p-0.5 text-[12px]">
            <button
              v-for="item in themeModes"
              :key="item.value"
              type="button"
              class="inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 font-medium transition"
              :class="
                store.config.settings.theme === item.value
                  ? 'bg-[#0A7AFF] text-white shadow-2xs'
                  : 'text-gray-600 hover:text-gray-900'
              "
              @click="setThemeMode(item.value)"
            >
              <component :is="item.icon" class="h-3.5 w-3.5" />
              <span>{{ item.label }}</span>
            </button>
          </div>
        </div>

        <div class="flex flex-wrap items-center justify-between gap-3 py-3.5">
          <div>
            <div class="text-[13px] font-medium text-gray-800">主题皮肤</div>
            <div class="text-[11.5px] text-gray-500">
              切换高亮主色调，默认匹配 FlClash 经典蓝
            </div>
          </div>
          <div class="flex flex-wrap items-center gap-1.5">
            <button
              v-for="item in SKINS"
              :key="item.id"
              type="button"
              class="inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[12px] font-medium transition"
              :class="
                skin === item.id
                  ? 'border-[#0A7AFF] bg-blue-50 text-[#0A7AFF] dark:bg-white/[0.08]'
                  : 'border-gray-200 bg-transparent text-gray-600 hover:bg-gray-100'
              "
              @click="setSkin(item.id)"
            >
              <span
                class="flex h-3 w-3 items-center justify-center rounded-full text-white"
                :style="{ backgroundColor: item.preview }"
              >
                <Check v-if="skin === item.id" class="h-2 w-2 stroke-[3]" />
              </span>
              <span>{{ item.label }}</span>
            </button>
          </div>
        </div>

        <div class="flex items-center justify-between py-3.5">
          <span class="text-[13px] font-medium text-gray-800">语言</span>
          <span class="rounded-lg bg-gray-100 px-2.5 py-1 text-[12.5px] text-gray-600">
            简体中文
          </span>
        </div>
      </div>

      <div class="antify-card divide-y divide-gray-100 px-5">
        <div class="flex items-center justify-between py-3.5">
          <div>
            <div class="text-[13px] font-medium text-gray-800">绕过局域网地址</div>
            <div class="text-[11.5px] text-gray-500">
              127.0.0.0/8、10.0.0.0/8、172.16.0.0/12、192.168.0.0/16 保持直连
            </div>
          </div>
          <ToggleSwitch
            size="sm"
            :model-value="store.config.settings.bypassLan"
            @update:model-value="(v) => store.updateSettings({ bypassLan: v })"
          />
        </div>

        <div class="flex items-center justify-between py-3.5">
          <span class="text-[13px] font-medium text-gray-800">启动时开启代理</span>
          <ToggleSwitch
            size="sm"
            :model-value="store.config.settings.autoStartProxy"
            @update:model-value="(v) => store.updateSettings({ autoStartProxy: v })"
          />
        </div>
      </div>

      <div class="antify-card space-y-2 p-5">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-1.5">
            <span class="text-[13px] font-medium text-gray-800">
              保持真实 IP 列表
            </span>
            <HelpTooltip
              text="填入此处的主机或通配符域名（如 *.corp.example.com）将跳过代理保持直连并保留真实 IP。"
            />
          </div>
          <button
            type="button"
            class="rounded-lg bg-gray-100 px-3 py-1 text-[12px] font-medium text-gray-700 hover:bg-gray-200"
            @click="saveBypassHosts"
          >
            保存列表
          </button>
        </div>
        <input
          v-model="bypassHostsText"
          type="text"
          placeholder="*.corp.example.com; intranet.example.com"
          class="w-full rounded-lg border border-gray-200 px-3 py-2 font-mono text-[12px] outline-none focus:border-[#0A7AFF]"
        />
      </div>
    </div>

    <div v-else-if="activeTab === 'permissions'" class="antify-card space-y-3 p-5 text-[13px]">
      <div class="flex items-center justify-between">
        <span class="text-gray-600">Named Pipe 通道</span>
        <code class="rounded bg-gray-100 px-2 py-0.5 text-[12px]">
          \\.\pipe\my-win-antify-core-v1
        </code>
      </div>
      <div class="flex items-center justify-between">
        <span class="text-gray-600">服务连接状态</span>
        <span class="font-medium text-gray-900">{{ store.pipeState }}</span>
      </div>
      <div class="flex items-center justify-between">
        <span class="text-gray-600">WinDivert 驱动就绪</span>
        <span
          class="font-medium"
          :class="store.status.driverReady ? 'text-emerald-600' : 'text-amber-600'"
        >
          {{ store.status.driverReady ? "已就绪" : "未运行 / 未加载" }}
        </span>
      </div>
      <div class="flex items-center justify-between">
        <span class="text-gray-600">核心服务 PID</span>
        <span class="font-mono text-gray-800">{{ store.status.servicePid || "—" }}</span>
      </div>
    </div>

    <div v-else class="antify-card space-y-2.5 p-5 text-[13px] text-gray-600">
      <div class="text-[15px] font-semibold text-gray-900">MyWinAntify v0.1.0</div>
      <p>
        本项目为个人自用与学习研究项目：因为非常喜欢 macOS 端的
        <span class="font-medium text-gray-800">Antify</span>，但目前暂无 Windows
        版本，故尝试仿照其交互与设计理念，在 Windows 上基于 WinDivert FLOW + NETWORK 反射与 Tokio 透明中继实现应用级代理接管（无虚拟网卡，兼容 Clash / FlClash / v2rayN）。
      </p>
      <p>
        当前版本尚不完善，仅供学习与技术交流使用；如果您使用 macOS，请支持官方正版 Antify（https://antifyapp.com）。
      </p>
      <p class="text-[12px] text-gray-400">
        配置文件遵循单一写者原则，由 core-service 统一管理（%ProgramData%\MyWinAntify\config.json）。
      </p>
    </div>
  </div>
</template>
