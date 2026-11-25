<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch } from "vue";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  FileDown,
  FolderOpen,
  Search,
  Terminal,
} from "lucide-vue-next";
import { isTauriRuntime } from "@/api/coreClient";
import { useAppStore } from "@/stores/app";
import { useI18n } from "@/i18n";
import type { Rule, RuleAction, RuleMatcher, RunningProcessInfo } from "@/types";
import { randomId } from "@/utils/format";
import HelpTooltip from "./HelpTooltip.vue";
import ModalDialog from "./ModalDialog.vue";
import SelectMenu, { type SelectOption } from "./SelectMenu.vue";
import ToggleSwitch from "./ToggleSwitch.vue";

const props = defineProps<{
  open: boolean;
  editing: Rule | null;
  prefill?: { name: string; path: string } | null;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "save", rule: Rule): void;
}>();

const store = useAppStore();
const { t } = useI18n();

const form = reactive({
  targetValue: "",
  customName: "",
  action: "proxy" as RuleAction,
  proxyId: "",
  networkLock: false,
  includeChildren: true,
  targetHostsText: "",
  targetPortsText: "",
  enabled: true,
  priority: 100,
});

const advancedOpen = ref(false);
const dragOver = ref(false);
const pickerOpen = ref(false);
const processQuery = ref("");
const validationError = ref<string | null>(null);
const fileInputRef = ref<HTMLInputElement | null>(null);
let unlistenDragDrop: UnlistenFn | null = null;

const canSubmit = computed(() => {
  if (!form.targetValue.trim()) return false;
  if (form.action === "proxy" && !form.proxyId) return false;
  return true;
});

const proxyOptions = computed<SelectOption[]>(() =>
  store.config.proxies.map((proxy) => ({
    value: proxy.id,
    label: proxy.name,
    description:
      proxy.protocol === "directInterface"
        ? proxy.interfaceName ?? "NIC"
        : `${proxy.host}:${proxy.port}`,
  })),
);

const filteredProcesses = computed(() => {
  const q = processQuery.value.trim().toLowerCase();
  return store.runningProcesses.filter((p) => {
    if (!q) return true;
    return (
      p.appName.toLowerCase().includes(q) ||
      p.name.toLowerCase().includes(q) ||
      p.executablePath.toLowerCase().includes(q) ||
      String(p.pid).includes(q)
    );
  });
});

function matcherTargetString(m: RuleMatcher): string {
  if (m.kind === "executablePath") return m.path;
  if (m.kind === "processName") return m.name;
  return m.dir;
}

const duplicateWarnings = computed(() => {
  const warnings: string[] = [];
  const profileId = props.editing?.profileId ?? store.activeProfile.id;
  const others = store.config.rules.filter(
    (r) => r.profileId === profileId && r.id !== props.editing?.id,
  );
  const nameCandidate = (
    form.customName.trim() || deriveNameFromPath(form.targetValue)
  ).toLowerCase();
  if (
    nameCandidate &&
    others.some((r) => r.name.trim().toLowerCase() === nameCandidate)
  ) {
    warnings.push(t("ruleModal.dupNameWarn"));
  }
  const targetNormalized = form.targetValue
    .trim()
    .replace(/\//g, "\\")
    .replace(/\\+$/, "")
    .toLowerCase();
  if (
    targetNormalized &&
    others.some(
      (r) =>
        matcherTargetString(r.matcher)
          .trim()
          .replace(/\//g, "\\")
          .replace(/\\+$/, "")
          .toLowerCase() === targetNormalized,
    )
  ) {
    warnings.push(t("ruleModal.dupTargetWarn"));
  }
  return warnings;
});

watch(
  () => [props.open, props.editing, props.prefill] as const,
  ([isOpen, rule, prefill]) => {
    if (!isOpen) {
      cleanupDragListener();
      return;
    }
    validationError.value = null;
    dragOver.value = false;
    void setupDragListener();

    if (rule) {
      form.targetValue = matcherTargetString(rule.matcher);
      form.customName = rule.name;
      form.includeChildren = rule.matcher.includeChildren;
      form.action = rule.action;
      form.proxyId = rule.proxyId ?? store.config.proxies[0]?.id ?? "";
      form.networkLock = rule.networkLock;
      form.targetHostsText = rule.targetHosts.join(", ");
      form.targetPortsText = rule.targetPorts.join(", ");
      form.enabled = rule.enabled;
      form.priority = rule.priority;
      advancedOpen.value =
        rule.targetHosts.length > 0 || rule.targetPorts.length > 0;
    } else {
      form.targetValue = prefill?.path ?? "";
      form.customName = prefill?.name ?? "";
      form.includeChildren = true;
      form.action = "proxy";
      form.proxyId = store.config.proxies[0]?.id ?? "";
      form.networkLock = false;
      form.targetHostsText = "";
      form.targetPortsText = "";
      form.enabled = true;
      form.priority = (store.activeProfileRules.length + 1) * 10;
      advancedOpen.value = false;
    }
  },
  { immediate: true },
);

async function setupDragListener() {
  if (!isTauriRuntime() || unlistenDragDrop) return;
  try {
    unlistenDragDrop = await getCurrentWebview().onDragDropEvent((event) => {
      if (!props.open) return;
      if (event.payload.type === "enter") {
        dragOver.value = true;
      } else if (event.payload.type === "leave") {
        dragOver.value = false;
      } else if (event.payload.type === "drop") {
        dragOver.value = false;
        const firstPath = event.payload.paths[0];
        if (firstPath) {
          applyDroppedPath(firstPath);
        }
      }
    });
  } catch {
    // Fallback to HTML5 drop when webview drag-drop listener is unavailable.
  }
}

function cleanupDragListener() {
  if (unlistenDragDrop) {
    unlistenDragDrop();
    unlistenDragDrop = null;
  }
  dragOver.value = false;
}

function deriveNameFromPath(value: string): string {
  const cleaned = value.trim().replace(/[\\/]+$/, "");
  const seg = cleaned.split(/[\\/]/).pop() ?? cleaned;
  return seg.replace(/\.exe$/i, "") || cleaned;
}

function applyDroppedPath(pathValue: string) {
  form.targetValue = pathValue.trim();
  if (!form.customName.trim()) {
    form.customName = deriveNameFromPath(pathValue);
  }
}

function handleHtmlDrop(ev: DragEvent) {
  dragOver.value = false;
  const file = ev.dataTransfer?.files?.[0];
  if (!file) return;
  const nativePath = (file as File & { path?: string }).path || file.name;
  applyDroppedPath(nativePath);
}

function onKeyDown(ev: KeyboardEvent) {
  if (props.open && !pickerOpen.value && ev.key === "Escape") {
    emit("close");
  }
}

window.addEventListener("keydown", onKeyDown);
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeyDown);
  cleanupDragListener();
});

async function openProcessPicker() {
  pickerOpen.value = true;
  await store.refreshProcesses();
}

function selectProcess(proc: RunningProcessInfo) {
  form.targetValue = proc.executablePath || proc.name;
  if (!form.customName.trim()) {
    form.customName = proc.appName || deriveNameFromPath(form.targetValue);
  }
  pickerOpen.value = false;
}

function triggerFileBrowse() {
  fileInputRef.value?.click();
}

function handleFilePicked(ev: Event) {
  const input = ev.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  const chosenPath = (file as File & { path?: string }).path || file.name;
  applyDroppedPath(chosenPath);
  input.value = "";
}

function inferMatcher(rawPath: string): RuleMatcher {
  const value = rawPath.trim();
  if (value.includes("\\") || value.includes("/")) {
    if (/\.(exe|com|bat|cmd|ps1)$/i.test(value)) {
      return {
        kind: "executablePath",
        path: value,
        includeChildren: form.includeChildren,
      };
    }
    return {
      kind: "directory",
      dir: value.replace(/[\\/]+$/, ""),
      includeChildren: form.includeChildren,
    };
  }
  return {
    kind: "processName",
    name: value,
    includeChildren: form.includeChildren,
  };
}

function parseList(raw: string): string[] {
  return raw
    .split(/[\n,;]+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function submit() {
  const target = form.targetValue.trim();
  if (!target) {
    validationError.value = t("ruleModal.errTarget");
    return;
  }
  if (form.action === "proxy" && !form.proxyId) {
    validationError.value = t("ruleModal.errProxy");
    return;
  }

  const ruleName = form.customName.trim() || deriveNameFromPath(target);
  emit("save", {
    id: props.editing?.id ?? randomId("rule"),
    profileId: props.editing?.profileId ?? store.activeProfile.id,
    enabled: form.enabled,
    name: ruleName,
    matcher: inferMatcher(target),
    action: form.action,
    proxyId: form.action === "proxy" ? form.proxyId : null,
    networkLock: form.networkLock,
    priority: Number(form.priority) || 100,
    targetHosts: parseList(form.targetHostsText),
    targetPorts: parseList(form.targetPortsText),
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
        class="flex max-h-[88vh] w-full max-w-[460px] flex-col overflow-hidden rounded-[18px] border border-gray-200/90 bg-white shadow-2xl"
      >
        <div class="border-b border-gray-200/75 px-5 py-3.5">
          <h3 class="text-[15px] font-semibold text-gray-900">
            {{ editing ? t("ruleModal.editTitle") : t("ruleModal.addTitle") }}
          </h3>
        </div>

        <div class="space-y-4 overflow-y-auto px-5 py-4">
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

          <div
            class="flex cursor-pointer flex-col items-center justify-center rounded-2xl border-[1.5px] border-dashed px-4 py-6 text-center transition"
            :class="
              dragOver
                ? 'border-[#0A7AFF] bg-blue-50/70 text-[#0A7AFF]'
                : 'border-gray-300/90 bg-[#FAFAFB] text-gray-500 hover:border-gray-400'
            "
            @dragover.prevent="dragOver = true"
            @dragleave.prevent="dragOver = false"
            @drop.prevent="handleHtmlDrop"
            @click="triggerFileBrowse"
          >
            <FileDown class="mb-2 h-7 w-7 stroke-[1.6] text-gray-400" />
            <div class="text-[12.5px]">{{ t("ruleModal.dropHint") }}</div>
          </div>

          <div>
            <div class="flex items-center gap-2.5">
              <label class="w-10 shrink-0 text-[13px] font-medium text-gray-800">
                {{ t("ruleModal.path") }}
              </label>
              <input
                v-model="form.targetValue"
                type="text"
                placeholder="C:\Program Files\Example\app.exe"
                class="h-8.5 min-w-0 flex-1 rounded-lg border border-gray-200/90 bg-white px-3 font-mono text-[12.5px] text-gray-900 outline-none transition focus:border-[#0A7AFF] focus:ring-[3px] focus:ring-[#0A7AFF]/25"
              />
              <input
                ref="fileInputRef"
                type="file"
                accept=".exe"
                class="hidden"
                @change="handleFilePicked"
              />
              <button
                type="button"
                :title="t('ruleModal.pickExe')"
                class="flex h-8 w-9 shrink-0 items-center justify-center rounded-lg bg-[#EFEFF2] text-gray-700 transition hover:bg-[#E5E5EA]"
                @click="triggerFileBrowse"
              >
                <FolderOpen class="h-4 w-4" />
              </button>
              <button
                type="button"
                :title="t('ruleModal.pickProcess')"
                class="flex h-8 w-9 shrink-0 items-center justify-center rounded-lg bg-[#EFEFF2] text-gray-700 transition hover:bg-[#E5E5EA]"
                @click="openProcessPicker"
              >
                <Terminal class="h-4 w-4" />
              </button>
            </div>
            <p class="mt-1.5 pl-[50px] text-[11px] text-gray-400">
              {{ t("ruleModal.pathHint") }}
            </p>
          </div>

          <div class="divide-y divide-gray-200/75 border-t border-gray-200/75">
            <div class="flex min-h-[46px] items-center justify-between gap-4 py-2">
              <span class="text-[13px] font-medium text-gray-800">{{ t("ruleModal.action") }}</span>
              <div class="inline-flex rounded-lg bg-[#EFEFF2] p-0.5 text-[12.5px]">
                <button
                  v-for="opt in [
                    { id: 'direct', label: t('common.direct') },
                    { id: 'proxy', label: t('common.proxy') },
                    { id: 'block', label: t('common.blockAction') },
                  ]"
                  :key="opt.id"
                  type="button"
                  class="rounded-[7px] px-3 py-1 font-medium transition"
                  :class="
                    form.action === opt.id
                      ? 'bg-[#0A7AFF] text-white shadow-2xs'
                      : 'text-gray-700 hover:text-gray-900'
                  "
                  @click="form.action = opt.id as RuleAction"
                >
                  {{ opt.label }}
                </button>
              </div>
            </div>

            <div
              v-if="form.action === 'proxy'"
              class="flex min-h-[46px] items-center justify-between gap-4 py-2"
            >
              <span class="text-[13px] font-medium text-gray-800">
                {{ t("ruleModal.proxyServer") }}
              </span>
              <SelectMenu
                v-model="form.proxyId"
                :options="proxyOptions"
                :placeholder="t('ruleModal.proxyPlaceholder')"
                variant="pill"
              />
            </div>

            <div class="flex min-h-[46px] items-center justify-between gap-4 py-2">
              <div class="flex items-center gap-1.5">
                <span class="text-[13px] font-medium text-gray-800">{{ t("ruleModal.networkLock") }}</span>
                <HelpTooltip :text="t('ruleModal.networkLockHelp')" />
              </div>
              <ToggleSwitch v-model="form.networkLock" size="sm" />
            </div>
          </div>

          <div class="border-t border-gray-200/75 pt-3">
            <button
              type="button"
              class="inline-flex items-center gap-1.5 text-[13px] font-medium text-gray-700 hover:text-gray-900"
              @click="advancedOpen = !advancedOpen"
            >
              <ChevronDown v-if="advancedOpen" class="h-3.5 w-3.5" />
              <ChevronRight v-else class="h-3.5 w-3.5" />
              <span>{{ t("ruleModal.advanced") }}</span>
            </button>

            <div v-if="advancedOpen" class="mt-3 space-y-3">
              <div>
                <label class="mb-1 block text-[12.5px] font-medium text-gray-800">
                  {{ t("ruleModal.targetHosts") }}
                </label>
                <textarea
                  v-model="form.targetHostsText"
                  rows="2"
                  :placeholder="t('ruleModal.targetHostsPlaceholder')"
                  class="w-full resize-none rounded-xl border border-gray-200/90 bg-white px-3 py-2 font-mono text-[12px] text-gray-900 outline-none transition focus:border-[#0A7AFF] focus:ring-[3px] focus:ring-[#0A7AFF]/25"
                />
                <p class="mt-1 text-[11px] text-gray-400">
                  {{ t("ruleModal.targetHostsHint") }}
                </p>
              </div>

              <div>
                <label class="mb-1 block text-[12.5px] font-medium text-gray-800">
                  {{ t("ruleModal.targetPorts") }}
                </label>
                <input
                  v-model="form.targetPortsText"
                  type="text"
                  :placeholder="t('ruleModal.targetPortsPlaceholder')"
                  class="h-8.5 w-full rounded-xl border border-gray-200/90 bg-white px-3 font-mono text-[12px] text-gray-900 outline-none transition focus:border-[#0A7AFF] focus:ring-[3px] focus:ring-[#0A7AFF]/25"
                />
                <p class="mt-1 text-[11px] text-gray-400">
                  {{ t("ruleModal.targetPortsHint") }}
                </p>
              </div>

              <div class="grid grid-cols-2 gap-3 pt-1">
                <div>
                  <label class="mb-1 block text-[12px] font-medium text-gray-700">
                    {{ t("ruleModal.ruleNameOpt") }}
                  </label>
                  <input
                    v-model="form.customName"
                    type="text"
                    :placeholder="t('ruleModal.ruleNamePlaceholder')"
                    class="h-8 w-full rounded-lg border border-gray-200/90 bg-white px-2.5 text-[12.5px] text-gray-900 outline-none focus:border-[#0A7AFF]"
                  />
                </div>
                <div class="flex items-end justify-between rounded-lg bg-[#F5F5F7] px-3 py-1.5">
                  <span class="text-[12px] font-medium text-gray-700">
                    {{ t("ruleModal.includeChildren") }}
                  </span>
                  <ToggleSwitch v-model="form.includeChildren" size="sm" />
                </div>
              </div>
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
            {{ editing ? t("ruleModal.saveRule") : t("ruleModal.createRule") }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>

  <ModalDialog
    :open="pickerOpen"
    :title="t('ruleModal.pickerTitle')"
    width-class="max-w-2xl"
    @close="pickerOpen = false"
  >
    <div class="space-y-3">
      <div class="relative">
        <Search class="absolute left-3 top-2.5 h-4 w-4 text-gray-400" />
        <input
          v-model="processQuery"
          type="text"
          :placeholder="t('ruleModal.pickerSearch')"
          class="w-full rounded-lg border border-gray-200 py-2 pl-9 pr-3 text-[13px] outline-none focus:border-[#0A7AFF]"
        />
      </div>

      <div class="max-h-80 space-y-1.5 overflow-y-auto pr-1">
        <button
          v-for="proc in filteredProcesses"
          :key="proc.pid"
          type="button"
          class="flex w-full items-center justify-between rounded-xl border border-gray-100 bg-gray-50/70 px-3.5 py-2.5 text-left transition hover:border-[#0A7AFF]/40 hover:bg-blue-50/40"
          @click="selectProcess(proc)"
        >
          <div class="min-w-0 pr-3">
            <div class="flex items-center gap-2">
              <span class="font-medium text-gray-900">
                {{ proc.appName || proc.name }}
              </span>
              <span
                class="rounded bg-gray-200/70 px-1.5 py-0.5 font-mono text-[11px] text-gray-600"
              >
                PID {{ proc.pid }}
              </span>
              <span
                v-if="proc.hasActiveConnections"
                class="rounded-full bg-emerald-100 px-2 py-0.5 text-[10.5px] font-medium text-emerald-700"
              >
                {{ t("ruleModal.pickerActive") }}
              </span>
            </div>
            <div class="mt-0.5 truncate font-mono text-[11.5px] text-gray-500">
              {{ proc.executablePath }}
            </div>
          </div>
          <span class="shrink-0 text-[12px] font-medium text-[#0A7AFF]">
            {{ t("ruleModal.pickerSelect") }}
          </span>
        </button>

        <div
          v-if="filteredProcesses.length === 0"
          class="py-8 text-center text-[12.5px] text-gray-400"
        >
          {{ t("ruleModal.pickerEmpty") }}
        </div>
      </div>
    </div>
  </ModalDialog>
</template>
