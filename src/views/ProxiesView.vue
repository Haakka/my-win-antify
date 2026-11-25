<script setup lang="ts">
import { computed, ref } from "vue";
import {
  Gauge,
  Plus,
  Search,
  Settings,
  Trash2,
} from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import type { ProxyConfig } from "@/types";
import HelpTooltip from "@/components/HelpTooltip.vue";
import ProxyFormModal from "@/components/ProxyFormModal.vue";

const store = useAppStore();
const searchQuery = ref("");
const modalOpen = ref(false);
const editingProxy = ref<ProxyConfig | null>(null);

const filteredProxies = computed(() => {
  const q = searchQuery.value.trim().toLowerCase();
  if (!q) return store.config.proxies;
  return store.config.proxies.filter(
    (p) =>
      p.name.toLowerCase().includes(q) ||
      p.host.toLowerCase().includes(q) ||
      String(p.port).includes(q),
  );
});

function openCreate() {
  editingProxy.value = null;
  modalOpen.value = true;
}

function openEdit(proxy: ProxyConfig) {
  editingProxy.value = proxy;
  modalOpen.value = true;
}

async function handleSave(proxy: ProxyConfig) {
  await store.upsertProxy(proxy);
  modalOpen.value = false;
}
</script>

<template>
  <div class="space-y-4">
    <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
      代理
    </h1>

    <!-- Toolbar -->
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="flex items-center gap-2.5">
        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-full bg-gray-200/80 px-3 py-1.5 text-[12.5px] font-medium text-gray-700 transition hover:bg-gray-300/80"
          @click="store.testAllProxies()"
        >
          <Gauge class="h-3.5 w-3.5 text-gray-600" />
          全部测试
        </button>

        <div class="relative">
          <Search class="absolute left-3 top-2 h-3.5 w-3.5 text-gray-400" />
          <input
            v-model="searchQuery"
            type="text"
            placeholder="搜索"
            class="w-48 rounded-full border border-gray-200/90 bg-white py-1.5 pl-8 pr-3 text-[12.5px] outline-none focus:border-[#0A7AFF]"
          />
        </div>
      </div>

      <button
        type="button"
        title="新增代理"
        class="inline-flex h-7 w-8 items-center justify-center rounded-lg border border-gray-200 bg-white text-gray-700 shadow-2xs transition hover:bg-gray-50"
        @click="openCreate"
      >
        <Plus class="h-4 w-4" />
      </button>
    </div>

    <!-- Table Header -->
    <div
      class="grid grid-cols-[1.2fr_1.4fr_0.8fr_0.7fr_0.9fr_0.9fr_0.7fr_0.6fr_0.8fr] items-center gap-2 border-b border-gray-200/80 px-3 pb-2 text-[11.5px] font-medium text-gray-500"
    >
      <span>名称</span>
      <span>服务器</span>
      <span>类型</span>
      <span>延迟</span>
      <span>网络接口</span>
      <span class="inline-flex items-center gap-1">
        <span>健康度</span>
        <HelpTooltip
          text="根据最近一次测速连通性评估代理节点状态（100% 表示 SOCKS5 握手成功，0% 表示连接失败或超时）。"
        />
      </span>
      <span>认证</span>
      <span>内网</span>
      <span class="text-right">操作</span>
    </div>

    <!-- Zebra Rounded Rows -->
    <div class="space-y-1.5">
      <div
        v-for="(proxy, idx) in filteredProxies"
        :key="proxy.id"
        class="grid grid-cols-[1.2fr_1.4fr_0.8fr_0.7fr_0.9fr_0.9fr_0.7fr_0.6fr_0.8fr] items-center gap-2 rounded-xl px-3 py-2.5 text-[12.5px] transition"
        :class="idx % 2 === 0 ? 'bg-white' : 'bg-[#EDEEF2]/75'"
      >
        <div class="flex items-center gap-2 font-semibold text-gray-900">
          <span
            class="h-2 w-2 rounded-full"
            :class="proxy.enabled ? 'bg-emerald-500' : 'bg-gray-300'"
          />
          <span class="truncate">{{ proxy.name }}</span>
        </div>

        <span class="truncate font-mono text-[12px] text-gray-600">
          {{ proxy.host }}:{{ proxy.port }}
        </span>

        <div>
          <span
            class="inline-flex rounded-full bg-purple-100/90 px-2.5 py-0.5 text-[11px] font-semibold uppercase text-purple-700"
          >
            {{ proxy.protocol }}
          </span>
        </div>

        <span class="font-mono text-[12px]">
          <template v-if="store.testingProxyIds[proxy.id]">
            <span class="text-blue-500">测试中...</span>
          </template>
          <template v-else-if="store.proxyTestResults[proxy.id]?.reachable">
            <span class="font-medium text-emerald-600">
              {{ store.proxyTestResults[proxy.id].latencyMs }} ms
            </span>
          </template>
          <template v-else-if="store.proxyTestResults[proxy.id]">
            <span class="text-red-500">超时</span>
          </template>
          <template v-else>
            <span class="text-gray-400">—</span>
          </template>
        </span>

        <span class="truncate text-gray-500">
          {{ proxy.interfaceName ?? "自动" }}
        </span>

        <div class="flex items-center gap-1.5 text-[12px]">
          <span
            class="h-2 w-2 rounded-full"
            :class="
              store.proxyTestResults[proxy.id]?.reachable
                ? 'bg-emerald-500'
                : store.proxyTestResults[proxy.id]
                  ? 'bg-red-500'
                  : 'bg-gray-400'
            "
          />
          <span class="text-gray-600">
            {{
              store.proxyTestResults[proxy.id]?.reachable
                ? "100%"
                : store.proxyTestResults[proxy.id]
                  ? "0%"
                  : "未测"
            }}
          </span>
        </div>

        <span class="text-gray-500">
          {{ proxy.auth ? "账密" : "无" }}
        </span>

        <span class="text-gray-500">
          {{ proxy.allowLan ? "允许" : "否" }}
        </span>

        <div class="flex items-center justify-end gap-2">
          <button
            type="button"
            title="测速"
            class="rounded-md p-1 text-[#0A7AFF] transition hover:bg-blue-50"
            @click="store.testProxy(proxy.id)"
          >
            <Gauge class="h-4 w-4" />
          </button>
          <button
            type="button"
            title="编辑"
            class="rounded-md p-1 text-gray-500 transition hover:bg-gray-100 hover:text-gray-800"
            @click="openEdit(proxy)"
          >
            <Settings class="h-4 w-4" />
          </button>
          <button
            type="button"
            title="删除"
            class="rounded-md p-1 text-red-400 transition hover:bg-red-50 hover:text-red-600"
            @click="store.deleteProxy(proxy.id)"
          >
            <Trash2 class="h-4 w-4" />
          </button>
        </div>
      </div>

      <div
        v-if="filteredProxies.length === 0"
        class="antify-card py-10 text-center text-[13px] text-gray-400"
      >
        暂无代理节点，点击右上角「+」添加一个上游 SOCKS5 代理。
      </div>
    </div>

    <ProxyFormModal
      :open="modalOpen"
      :editing="editingProxy"
      @close="modalOpen = false"
      @save="handleSave"
    />
  </div>
</template>
