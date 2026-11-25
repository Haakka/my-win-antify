<script setup lang="ts">
import { computed, ref } from "vue";
import {
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  Check,
  Download,
  Globe,
  GripVertical,
  Plus,
  Search,
  Settings,
  Trash2,
  Upload,
} from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { useI18n } from "@/i18n";
import type { Rule, RulesImportPreview } from "@/types";
import { formatRelativeTime } from "@/utils/format";
import HelpTooltip from "@/components/HelpTooltip.vue";
import ModalDialog from "@/components/ModalDialog.vue";
import ToggleSwitch from "@/components/ToggleSwitch.vue";
import RuleFormModal from "@/components/RuleFormModal.vue";

const store = useAppStore();
const { locale, t } = useI18n();
const searchQuery = ref("");
const sortAsc = ref(true);
const modalOpen = ref(false);
const editingRule = ref<Rule | null>(null);
const importModalOpen = ref(false);
const importRulesText = ref("");
const importPreview = ref<RulesImportPreview | null>(null);
const importError = ref<string | null>(null);

const reordering = ref(false);
const draftRules = ref<Rule[]>([]);
const draggingId = ref<string | null>(null);

const displayedRules = computed(() => {
  if (reordering.value) return draftRules.value;
  const q = searchQuery.value.trim().toLowerCase();
  const list = store.activeProfileRules.filter((r) => {
    if (!q) return true;
    const target = matcherTarget(r).toLowerCase();
    return r.name.toLowerCase().includes(q) || target.includes(q);
  });
  return sortAsc.value ? list : list.slice().reverse();
});

function enterReorder() {
  searchQuery.value = "";
  sortAsc.value = true;
  draftRules.value = store.activeProfileRules.slice();
  reordering.value = true;
}

async function finishReorder() {
  const next = JSON.parse(JSON.stringify(store.config));
  const priorityMap = new Map<string, number>();
  draftRules.value.forEach((r, idx) => {
    priorityMap.set(r.id, (idx + 1) * 10);
  });
  for (const r of next.rules as Rule[]) {
    const p = priorityMap.get(r.id);
    if (p !== undefined) {
      r.priority = p;
    }
  }
  reordering.value = false;
  await store.saveConfig(next);
}

function moveRule(idx: number, delta: number) {
  const target = idx + delta;
  if (target < 0 || target >= draftRules.value.length) return;
  const arr = draftRules.value.slice();
  const [item] = arr.splice(idx, 1);
  if (item) {
    arr.splice(target, 0, item);
    draftRules.value = arr;
  }
}

function onDragStart(id: string) {
  draggingId.value = id;
}

function onDropRule(targetId: string) {
  if (!draggingId.value || draggingId.value === targetId) return;
  const fromIdx = draftRules.value.findIndex((r) => r.id === draggingId.value);
  const toIdx = draftRules.value.findIndex((r) => r.id === targetId);
  if (fromIdx >= 0 && toIdx >= 0) {
    const arr = draftRules.value.slice();
    const [item] = arr.splice(fromIdx, 1);
    if (item) {
      arr.splice(toIdx, 0, item);
      draftRules.value = arr;
    }
  }
  draggingId.value = null;
}

function matcherTarget(rule: Rule): string {
  if (rule.matcher.kind === "executablePath") return rule.matcher.path;
  if (rule.matcher.kind === "processName") return rule.matcher.name;
  return rule.matcher.dir;
}

function matcherTypeLabel(rule: Rule): string {
  if (rule.matcher.kind === "directory") return t("rules.typeDir");
  if (rule.matcher.kind === "processName") return t("rules.typeProc");
  return t("rules.typeApp");
}

function proxyNameOf(proxyId: string | null): string {
  if (!proxyId) return "—";
  return (
    store.config.proxies.find((p) => p.id === proxyId)?.name ??
    t("rules.deletedProxy")
  );
}

function isInvalidRule(ruleId: string): boolean {
  return store.status.invalidRuleIds.includes(ruleId);
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

async function handleExportRules() {
  const json = await store.exportRulesJson(store.activeProfile.id);
  const blob = new Blob([json], { type: "application/json;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `my-win-antify-rules-${store.activeProfile.name}.json`;
  a.click();
  URL.revokeObjectURL(url);
}

function openImportRules() {
  importRulesText.value = "";
  importPreview.value = null;
  importError.value = null;
  importModalOpen.value = true;
}

function handleImportRulesFile(ev: Event) {
  const input = ev.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  const reader = new FileReader();
  reader.onload = () => {
    importRulesText.value = typeof reader.result === "string" ? reader.result : "";
    importPreview.value = null;
  };
  reader.readAsText(file);
  input.value = "";
}

async function previewImportRules() {
  importError.value = null;
  try {
    importPreview.value = await store.importRulesJson(
      importRulesText.value,
      store.activeProfile.id,
      true,
    );
  } catch (err) {
    importError.value =
      err instanceof Error ? err.message : "Failed to parse or validate rules JSON";
  }
}

async function confirmImportRules() {
  importError.value = null;
  try {
    await store.importRulesJson(
      importRulesText.value,
      store.activeProfile.id,
      false,
    );
    importModalOpen.value = false;
  } catch (err) {
    importError.value =
      err instanceof Error ? err.message : "Failed to import rules";
  }
}
</script>

<template>
  <div class="space-y-4">
    <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
      {{ t("rules.title") }}
    </h1>

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

      <div class="flex flex-wrap items-center gap-2">
        <div v-if="!reordering" class="relative">
          <Search class="absolute left-2.5 top-2 h-3.5 w-3.5 text-gray-400" />
          <input
            v-model="searchQuery"
            type="text"
            :placeholder="t('rules.filterPlaceholder')"
            class="w-40 rounded-lg border border-gray-200 bg-white py-1 pl-8 pr-2.5 text-[12.5px] outline-none focus:border-[#0A7AFF]"
          />
        </div>
        <button
          v-if="!reordering && store.activeProfileRules.length > 1"
          type="button"
          class="inline-flex h-7 items-center gap-1 rounded-lg border border-gray-200 bg-white px-2 text-[12px] font-medium text-gray-600 transition hover:bg-gray-50"
          @click="enterReorder"
        >
          <ArrowUpDown class="h-3.5 w-3.5" />
          {{ t("rules.reorder") }}
        </button>
        <button
          v-else-if="reordering"
          type="button"
          class="inline-flex h-7 items-center gap-1 rounded-lg bg-[#0A7AFF] px-2.5 text-[12px] font-medium text-white transition hover:bg-blue-600"
          @click="finishReorder"
        >
          <Check class="h-3.5 w-3.5" />
          {{ t("rules.doneReorder") }}
        </button>

        <button
          type="button"
          :title="t('rules.exportBtn')"
          class="inline-flex h-7 items-center gap-1 rounded-lg border border-gray-200 bg-white px-2 text-[12px] text-gray-600 transition hover:bg-gray-50"
          @click="handleExportRules"
        >
          <Download class="h-3.5 w-3.5" />
          <span class="hidden sm:inline">{{ t("rules.exportBtn") }}</span>
        </button>
        <button
          type="button"
          :title="t('rules.importBtn')"
          class="inline-flex h-7 items-center gap-1 rounded-lg border border-gray-200 bg-white px-2 text-[12px] text-gray-600 transition hover:bg-gray-50"
          @click="openImportRules"
        >
          <Upload class="h-3.5 w-3.5" />
          <span class="hidden sm:inline">{{ t("rules.importBtn") }}</span>
        </button>
        <button
          v-if="!reordering"
          type="button"
          :title="t('rules.toggleSort')"
          class="inline-flex h-7 w-8 items-center justify-center rounded-lg border border-gray-200 bg-white text-gray-600 transition hover:bg-gray-50"
          @click="sortAsc = !sortAsc"
        >
          <ArrowUpDown class="h-3.5 w-3.5" />
        </button>
        <button
          type="button"
          :title="t('rules.addRule')"
          class="inline-flex h-7 w-8 items-center justify-center rounded-lg border border-gray-200 bg-white text-gray-700 transition hover:bg-gray-50"
          @click="openCreate"
        >
          <Plus class="h-4 w-4" />
        </button>
      </div>
    </div>

    <div
      v-if="reordering"
      class="rounded-xl border border-blue-200/80 bg-blue-50/70 px-3.5 py-2 text-[12px] text-blue-800 dark:border-blue-500/30 dark:bg-blue-950/30 dark:text-blue-200"
    >
      {{ t("rules.reorderHint") }}
    </div>

    <!-- Header (progressive responsive columns for >=960px windows) -->
    <div
      class="grid grid-cols-[56px_2fr_68px_78px_100px_76px] items-center gap-2 border-b border-gray-200/80 px-3 pb-2 text-[11.5px] font-medium text-gray-500 lg:grid-cols-[56px_2fr_68px_78px_100px_80px_76px] xl:grid-cols-[64px_2.2fr_76px_84px_110px_84px_110px_76px]"
    >
      <span>{{ t("rules.colEnabled") }}</span>
      <span>{{ t("rules.colApp") }}</span>
      <span>{{ t("rules.colType") }}</span>
      <span>{{ t("rules.colAction") }}</span>
      <span>{{ t("rules.colProxy") }}</span>
      <span class="hidden items-center gap-1 lg:inline-flex">
        <span>{{ t("rules.colNetworkLock") }}</span>
        <HelpTooltip :text="t('rules.networkLockHelp')" />
      </span>
      <span class="hidden xl:block">{{ t("rules.colLastMatch") }}</span>
      <span class="text-right">{{ t("rules.colActions") }}</span>
    </div>

    <!-- Rows -->
    <div class="space-y-1.5">
      <div
        v-for="(rule, idx) in displayedRules"
        :key="rule.id"
        :draggable="reordering"
        class="grid grid-cols-[56px_2fr_68px_78px_100px_76px] items-center gap-2 rounded-xl px-3 py-2 text-[12.5px] transition lg:grid-cols-[56px_2fr_68px_78px_100px_80px_76px] xl:grid-cols-[64px_2.2fr_76px_84px_110px_84px_110px_76px]"
        :class="[
          isInvalidRule(rule.id)
            ? 'border border-red-300/80 bg-red-50/70 dark:border-red-500/35 dark:bg-red-950/30'
            : idx % 2 === 0
              ? 'bg-white'
              : 'bg-[#EDEEF2]/75',
          reordering ? 'cursor-grab ring-1 ring-blue-200/70' : '',
        ]"
        @dragstart="onDragStart(rule.id)"
        @dragover.prevent
        @drop.prevent="onDropRule(rule.id)"
      >
        <div class="flex items-center gap-1">
          <GripVertical v-if="reordering" class="h-3.5 w-3.5 shrink-0 text-gray-400" />
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
                {{ t("rules.childProcessTag") }}
              </span>
              <button
                v-if="isInvalidRule(rule.id)"
                type="button"
                class="rounded-full bg-red-100 px-2 py-0.5 text-[10.5px] font-semibold text-red-700 transition hover:bg-red-200 dark:bg-red-500/20 dark:text-red-300"
                :title="t('rules.invalidTooltip')"
                @click="openEdit(rule)"
              >
                {{ t("rules.invalidBadge") }}
              </button>
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
            {{ t(`common.${rule.action}`) }}
          </span>
        </div>

        <span class="truncate text-gray-600">
          {{ rule.action === "proxy" ? proxyNameOf(rule.proxyId) : "—" }}
        </span>

        <div class="hidden lg:block">
          <ToggleSwitch
            size="sm"
            :model-value="rule.networkLock"
            @update:model-value="(val) => store.toggleRuleNetworkLock(rule.id, val)"
          />
        </div>

        <div class="hidden items-center gap-1.5 text-[12px] xl:flex">
          <template v-if="store.ruleLastMatchedMap[rule.id]">
            <span
              class="h-2 w-2 rounded-full"
              :class="
                formatRelativeTime(store.ruleLastMatchedMap[rule.id], locale).recent
                  ? 'bg-emerald-500'
                  : 'bg-gray-400'
              "
            />
            <span class="text-gray-600">
              {{ formatRelativeTime(store.ruleLastMatchedMap[rule.id], locale).label }}
            </span>
          </template>
          <span v-else class="text-gray-400">—</span>
        </div>

        <div v-if="reordering" class="flex items-center justify-end gap-1">
          <button
            type="button"
            :disabled="idx === 0"
            class="rounded-md p-1 text-gray-500 transition hover:bg-gray-100 disabled:opacity-30"
            @click="moveRule(idx, -1)"
          >
            <ArrowUp class="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            :disabled="idx === displayedRules.length - 1"
            class="rounded-md p-1 text-gray-500 transition hover:bg-gray-100 disabled:opacity-30"
            @click="moveRule(idx, 1)"
          >
            <ArrowDown class="h-3.5 w-3.5" />
          </button>
        </div>

        <div v-else class="flex items-center justify-end gap-1.5">
          <button
            type="button"
            :title="t('common.edit')"
            class="rounded-md p-1 text-gray-500 transition hover:bg-gray-100 hover:text-gray-800"
            @click="openEdit(rule)"
          >
            <Settings class="h-4 w-4" />
          </button>
          <button
            type="button"
            :title="t('common.delete')"
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
        {{ t("rules.empty") }}
      </div>
    </div>

    <RuleFormModal
      :open="modalOpen"
      :editing="editingRule"
      @close="modalOpen = false"
      @save="handleSave"
    />

    <ModalDialog
      :open="importModalOpen"
      :title="t('rules.importTitle')"
      @close="importModalOpen = false"
    >
      <div class="space-y-3 text-[12.5px]">
        <div class="text-gray-500">
          {{ t("rules.importTargetProfile", { name: store.activeProfile.name }) }}
        </div>
        <div v-if="importError" class="rounded-lg bg-red-50 px-3 py-2 text-red-600">
          {{ importError }}
        </div>
        <input
          type="file"
          accept=".json,application/json"
          class="block w-full text-[12px] text-gray-600 file:mr-3 file:rounded-lg file:border-0 file:bg-gray-100 file:px-3 file:py-1.5 file:text-[12px] file:font-medium file:text-gray-700 hover:file:bg-gray-200"
          @change="handleImportRulesFile"
        />
        <textarea
          v-model="importRulesText"
          rows="6"
          :placeholder="t('rules.importPlaceholder')"
          class="w-full rounded-xl border border-gray-200 p-3 font-mono text-[11.5px] outline-none focus:border-[#0A7AFF]"
          @input="importPreview = null"
        />
        <div
          v-if="importPreview"
          class="space-y-1.5 rounded-xl bg-gray-50 p-3 text-[12px] dark:bg-[#252830]"
        >
          <div>{{ t("rules.importTotal", { count: importPreview.totalInFile }) }}</div>
          <div class="text-emerald-600">
            {{ t("rules.importAdded", { count: importPreview.added.length }) }}
            <span v-if="importPreview.added.length">({{ importPreview.added.join(", ") }})</span>
          </div>
          <div class="text-gray-500">
            {{ t("rules.importSkipped", { count: importPreview.skippedDuplicates.length }) }}
          </div>
          <div v-if="importPreview.missingProxyRules.length" class="text-amber-600">
            {{
              t("rules.importMissingProxy", {
                names: importPreview.missingProxyRules.join(", "),
              })
            }}
          </div>
        </div>
      </div>
      <template #footer>
        <button
          type="button"
          class="rounded-lg border border-gray-200 bg-white px-3.5 py-1.5 text-[12.5px] font-medium text-gray-700 hover:bg-gray-50"
          @click="importModalOpen = false"
        >
          {{ t("common.cancel") }}
        </button>
        <button
          type="button"
          :disabled="!importRulesText.trim()"
          class="rounded-lg bg-gray-100 px-3.5 py-1.5 text-[12.5px] font-medium text-gray-800 hover:bg-gray-200 disabled:opacity-50"
          @click="previewImportRules"
        >
          {{ t("rules.previewBtn") }}
        </button>
        <button
          type="button"
          :disabled="!importPreview || importPreview.added.length === 0"
          class="rounded-lg bg-[#0A7AFF] px-4 py-1.5 text-[12.5px] font-medium text-white hover:bg-blue-600 disabled:opacity-50"
          @click="confirmImportRules"
        >
          {{ t("rules.confirmImportBtn") }}
        </button>
      </template>
    </ModalDialog>
  </div>
</template>
