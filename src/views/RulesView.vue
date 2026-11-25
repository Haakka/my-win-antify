<script setup lang="ts">
import { computed, ref } from "vue";
import {
  ArrowUpDown,
  Globe,
  Plus,
  Search,
  Settings,
  Trash2,
} from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import type { Rule } from "@/types";
import { formatRelativeTime } from "@/utils/format";
import HelpTooltip from "@/components/HelpTooltip.vue";
import ToggleSwitch from "@/components/ToggleSwitch.vue";
import RuleFormModal from "@/components/RuleFormModal.vue";

const store = useAppStore();
const searchQuery = ref("");
const sortAsc = ref(true);
const modalOpen = ref(false);
const editingRule = ref<Rule | null>(null);

const displayedRules = computed(() => {
  const q = searchQuery.value.trim().toLowerCase();
  const list = store.activeProfileRules.filter((r) => {
    if (!q) return true;
    const target = matcherTarget(r).toLowerCase();
    return r.name.toLowerCase().includes(q) || target.includes(q);
  });
  return sortAsc.value ? list : list.slice().reverse();
});

function matcherTarget(rule: Rule): string {
  if (rule.matcher.kind === "executablePath") return rule.matcher.path;
  if (rule.matcher.kind === "processName") return rule.matcher.name;
  return rule.matcher.dir;
}

function matcherTypeLabel(rule: Rule): string {
  if (rule.matcher.kind === "directory") return "目录";
  if (rule.matcher.kind === "processName") return "进程";
  return "应用";
}

function proxyNameOf(proxyId: string | null): string {
  if (!proxyId) return "—";
  return store.config.proxies.find((p) => p.id === proxyId)?.name ?? "已删除";
}

function openCreate() {
  editingRule.value = null;
  modalOpen.value = true;
}

function openEdit(rule: Rule) {
  editingRule.value = rule;
  modalOpen.value = true;
}

async function handleSave(rule: Rule) {
  await store.upsertRule(rule);
  modalOpen.value = false;
}
</script>

<template>
  <div class="space-y-4">
    <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
      规则
    </h1>

    <!-- Profile Pills & Toolbar -->
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="flex flex-wrap items-center gap-2">
        <button
          v-for="prof in store.config.profiles"
          :key="prof.id"
          type="button"
          class="inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-[12.5px] font-medium transition"
          :class="
            prof.id === store.activeProfile.id
              ? 'bg-blue-50 text-[#0A7AFF] ring-1 ring-[#0A7AFF]/30'
              : 'bg-gray-200/70 text-gray-600 hover:bg-gray-200'
          "
          @click="store.setActiveProfile(prof.id)"
        >
          <Globe class="h-3.5 w-3.5" />
          {{ prof.name }}
        </button>
      </div>

      <div class="flex items-center gap-2">
        <div class="relative">
          <Search class="absolute left-2.5 top-2 h-3.5 w-3.5 text-gray-400" />
          <input
            v-model="searchQuery"
            type="text"
            placeholder="过滤规则..."
            class="w-44 rounded-lg border border-gray-200 bg-white py-1 pl-8 pr-2.5 text-[12.5px] outline-none focus:border-[#0A7AFF]"
          />
        </div>
        <button
          type="button"
          title="切换排序"
          class="inline-flex h-7 w-8 items-center justify-center rounded-lg border border-gray-200 bg-white text-gray-600 transition hover:bg-gray-50"
          @click="sortAsc = !sortAsc"
        >
          <ArrowUpDown class="h-3.5 w-3.5" />
        </button>
        <button
          type="button"
          title="新增规则"
          class="inline-flex h-7 w-8 items-center justify-center rounded-lg border border-gray-200 bg-white text-gray-700 transition hover:bg-gray-50"
          @click="openCreate"
        >
          <Plus class="h-4 w-4" />
        </button>
      </div>
    </div>

    <!-- Header -->
    <div
      class="grid grid-cols-[64px_2.2fr_76px_84px_110px_84px_110px_76px] items-center gap-2 border-b border-gray-200/80 px-3 pb-2 text-[11.5px] font-medium text-gray-500"
    >
      <span>已启用</span>
      <span>应用</span>
      <span>类型</span>
      <span>动作</span>
      <span>代理</span>
      <span class="inline-flex items-center gap-1">
        <span>网络锁</span>
        <HelpTooltip
          text="开启后，若上游代理不可用将严格阻断该应用的连接，绝不回退直连，防止真实 IP 泄露。"
        />
      </span>
      <span>最近匹配</span>
      <span class="text-right">操作</span>
    </div>

    <!-- Rows -->
    <div class="space-y-1.5">
      <div
        v-for="(rule, idx) in displayedRules"
        :key="rule.id"
        class="grid grid-cols-[64px_2.2fr_76px_84px_110px_84px_110px_76px] items-center gap-2 rounded-xl px-3 py-2 text-[12.5px]"
        :class="idx % 2 === 0 ? 'bg-white' : 'bg-[#EDEEF2]/75'"
      >
        <div>
          <ToggleSwitch
            size="sm"
            :model-value="rule.enabled"
            @update:model-value="(val) => store.toggleRule(rule.id, val)"
          />
        </div>

        <div class="flex min-w-0 items-center gap-2.5">
          <div
            class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-slate-900 text-[12px] font-bold text-white"
          >
            {{ rule.name.slice(0, 1).toUpperCase() }}
          </div>
          <div class="min-w-0">
            <div class="flex items-center gap-1.5 truncate font-semibold text-gray-900">
              <span class="truncate">{{ rule.name }}</span>
              <span
                v-if="rule.matcher.includeChildren"
                class="rounded bg-gray-200/70 px-1 py-0 text-[10px] font-normal text-gray-600"
              >
                +子进程
              </span>
            </div>
            <div class="truncate font-mono text-[11px] text-gray-400">
              {{ matcherTarget(rule) }}
            </div>
          </div>
        </div>

        <div>
          <span
            class="inline-flex rounded-full bg-blue-50 px-2.5 py-0.5 text-[11px] font-medium text-[#0A7AFF]"
          >
            {{ matcherTypeLabel(rule) }}
          </span>
        </div>

        <div class="flex items-center gap-1.5 text-[12.5px]">
          <span
            class="h-2 w-2 rounded-full"
            :class="
              rule.action === 'proxy'
                ? 'bg-[#0A7AFF]'
                : rule.action === 'direct'
                  ? 'bg-emerald-500'
                  : 'bg-red-500'
            "
          />
          <span class="text-gray-700">
            {{
              rule.action === "proxy"
                ? "代理"
                : rule.action === "direct"
                  ? "直连"
                  : "阻断"
            }}
          </span>
        </div>

        <span class="truncate text-gray-600">
          {{ rule.action === "proxy" ? proxyNameOf(rule.proxyId) : "—" }}
        </span>

        <div>
          <ToggleSwitch
            size="sm"
            :model-value="rule.networkLock"
            @update:model-value="(val) => store.toggleRuleNetworkLock(rule.id, val)"
          />
        </div>

        <div class="flex items-center gap-1.5 text-[12px]">
          <template v-if="store.ruleLastMatchedMap[rule.id]">
            <span
              class="h-2 w-2 rounded-full"
              :class="
                formatRelativeTime(store.ruleLastMatchedMap[rule.id]).recent
                  ? 'bg-emerald-500'
                  : 'bg-gray-400'
              "
            />
            <span class="text-gray-600">
              {{ formatRelativeTime(store.ruleLastMatchedMap[rule.id]).label }}
            </span>
          </template>
          <span v-else class="text-gray-400">—</span>
        </div>

        <div class="flex items-center justify-end gap-1.5">
          <button
            type="button"
            title="编辑规则"
            class="rounded-md p-1 text-gray-500 transition hover:bg-gray-100 hover:text-gray-800"
            @click="openEdit(rule)"
          >
            <Settings class="h-4 w-4" />
          </button>
          <button
            type="button"
            title="删除规则"
            class="rounded-md p-1 text-red-400 transition hover:bg-red-50 hover:text-red-600"
            @click="store.deleteRule(rule.id)"
          >
            <Trash2 class="h-4 w-4" />
          </button>
        </div>
      </div>

      <div
        v-if="displayedRules.length === 0"
        class="antify-card py-10 text-center text-[13px] text-gray-400"
      >
        当前配置暂无规则，点击右上角「+」选择 .exe 或运行中进程添加。
      </div>
    </div>

    <RuleFormModal
      :open="modalOpen"
      :editing="editingRule"
      @close="modalOpen = false"
      @save="handleSave"
    />
  </div>
</template>
