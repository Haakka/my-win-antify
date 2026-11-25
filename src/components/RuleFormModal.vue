<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch } from "vue";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  ChevronDown,
  ChevronRight,
  FileDown,
  FolderOpen,
  Search,
  Terminal,
} from "lucide-vue-next";
import { isTauriRuntime } from "@/api/coreClient";
import { useAppStore } from "@/stores/app";
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
    description: `${proxy.host}:${proxy.port}`,
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
      form.targetValue =
        rule.matcher.kind === "executablePath"
          ? rule.matcher.path
          : rule.matcher.kind === "processName"
            ? rule.matcher.name
            : rule.matcher.dir;
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
    validationError.value = "请填写路径或拖入应用 / 文件夹";
    return;
  }
  if (form.action === "proxy" && !form.proxyId) {
    validationError.value = "请选择一个代理服务器";
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
        <!-- Header -->
        <div class="border-b border-gray-200/75 px-5 py-3.5">
          <h3 class="text-[15px] font-semibold text-gray-900">
            {{ editing ? "编辑规则" : "添加规则" }}
          </h3>
        </div>

        <!-- Body -->
        <div class="space-y-4 overflow-y-auto px-5 py-4">
          <div
            v-if="validationError"
            class="rounded-xl bg-red-50 px-3.5 py-2 text-[12px] text-red-600"
          >
            {{ validationError }}
          </div>

          <!-- Top Drag-and-Drop Zone -->
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
            <div class="text-[12.5px]">
              拖入应用、工具、脚本、文件夹或快捷方式
            </div>
          </div>

          <!-- Path Row -->
          <div>
            <div class="flex items-center gap-2.5">
              <label class="w-10 shrink-0 text-[13px] font-medium text-gray-800">
                路径
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
                title="选择本地 .exe"
                class="flex h-8 w-9 shrink-0 items-center justify-center rounded-lg bg-[#EFEFF2] text-gray-700 transition hover:bg-[#E5E5EA]"
                @click="triggerFileBrowse"
              >
                <FolderOpen class="h-4 w-4" />
              </button>
              <button
                type="button"
                title="从运行中进程选取"
                class="flex h-8 w-9 shrink-0 items-center justify-center rounded-lg bg-[#EFEFF2] text-gray-700 transition hover:bg-[#E5E5EA]"
                @click="openProcessPicker"
              >
                <Terminal class="h-4 w-4" />
              </button>
            </div>
            <p class="mt-1.5 pl-[50px] text-[11px] text-gray-400">
              支持 .exe 路径、应用目录或进程名（如 curl.exe），自动匹配规则类型
            </p>
          </div>

          <!-- Action / Proxy / Network Lock Rows -->
          <div class="divide-y divide-gray-200/75 border-t border-gray-200/75">
            <!-- Action Segmented Control -->
            <div class="flex min-h-[46px] items-center justify-between gap-4 py-2">
              <span class="text-[13px] font-medium text-gray-800">动作</span>
              <div class="inline-flex rounded-lg bg-[#EFEFF2] p-0.5 text-[12.5px]">
                <button
                  v-for="opt in [
                    { id: 'direct', label: '直连' },
                    { id: 'proxy', label: '代理' },
                    { id: 'block', label: '阻止' },
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

            <!-- Proxy Server Selector -->
            <div
              v-if="form.action === 'proxy'"
              class="flex min-h-[46px] items-center justify-between gap-4 py-2"
            >
              <span class="text-[13px] font-medium text-gray-800">
                代理服务器
              </span>
              <SelectMenu
                v-model="form.proxyId"
                :options="proxyOptions"
                placeholder="选择代理..."
                variant="pill"
              />
            </div>

            <!-- Network Lock -->
            <div class="flex min-h-[46px] items-center justify-between gap-4 py-2">
              <div class="flex items-center gap-1.5">
                <span class="text-[13px] font-medium text-gray-800">网络锁</span>
                <HelpTooltip
                  text="当上游代理不可用时严格阻断该应用的流量，防止真实 IP 泄露（优先级高于代理节点的「代理失败时直连」）。"
                />
              </div>
              <ToggleSwitch v-model="form.networkLock" size="sm" />
            </div>
          </div>

          <!-- Collapsible Advanced Options -->
          <div class="border-t border-gray-200/75 pt-3">
            <button
              type="button"
              class="inline-flex items-center gap-1.5 text-[13px] font-medium text-gray-700 hover:text-gray-900"
              @click="advancedOpen = !advancedOpen"
            >
              <ChevronDown v-if="advancedOpen" class="h-3.5 w-3.5" />
              <ChevronRight v-else class="h-3.5 w-3.5" />
              <span>高级选项</span>
            </button>

            <div v-if="advancedOpen" class="mt-3 space-y-3">
              <div>
                <label class="mb-1 block text-[12.5px] font-medium text-gray-800">
                  目标主机
                </label>
                <textarea
                  v-model="form.targetHostsText"
                  rows="2"
                  placeholder="任意"
                  class="w-full resize-none rounded-xl border border-gray-200/90 bg-white px-3 py-2 font-mono text-[12px] text-gray-900 outline-none transition focus:border-[#0A7AFF] focus:ring-[3px] focus:ring-[#0A7AFF]/25"
                />
                <p class="mt-1 text-[11px] text-gray-400">
                  示例：*.google.com, 10.0.0.0/8, 10.1.0.0-10.5.255.255, -ads.google.com
                </p>
              </div>

              <div>
                <label class="mb-1 block text-[12.5px] font-medium text-gray-800">
                  目标端口
                </label>
                <input
                  v-model="form.targetPortsText"
                  type="text"
                  placeholder="任意"
                  class="h-8.5 w-full rounded-xl border border-gray-200/90 bg-white px-3 font-mono text-[12px] text-gray-900 outline-none transition focus:border-[#0A7AFF] focus:ring-[3px] focus:ring-[#0A7AFF]/25"
                />
                <p class="mt-1 text-[11px] text-gray-400">
                  示例：80, 443, 8000-9000, -3124
                </p>
              </div>

              <div class="grid grid-cols-2 gap-3 pt-1">
                <div>
                  <label class="mb-1 block text-[12px] font-medium text-gray-700">
                    规则名称（可选）
                  </label>
                  <input
                    v-model="form.customName"
                    type="text"
                    placeholder="自动从路径提取"
                    class="h-8 w-full rounded-lg border border-gray-200/90 bg-white px-2.5 text-[12.5px] text-gray-900 outline-none focus:border-[#0A7AFF]"
                  />
                </div>
                <div class="flex items-end justify-between rounded-lg bg-[#F5F5F7] px-3 py-1.5">
                  <span class="text-[12px] font-medium text-gray-700">
                    包含子进程
                  </span>
                  <ToggleSwitch v-model="form.includeChildren" size="sm" />
                </div>
              </div>
            </div>
          </div>
        </div>

        <!-- Footer: left Cancel, right Create/Save Rule -->
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
            {{ editing ? "保存规则" : "创建规则" }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>

  <!-- Running Windows Process Picker Modal -->
  <ModalDialog
    :open="pickerOpen"
    title="从运行中的 Windows 进程选取"
    width-class="max-w-2xl"
    @close="pickerOpen = false"
  >
    <div class="space-y-3">
      <div class="relative">
        <Search class="absolute left-3 top-2.5 h-4 w-4 text-gray-400" />
        <input
          v-model="processQuery"
          type="text"
          placeholder="搜索进程名、应用名、PID 或 .exe 路径..."
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
                活跃连接
              </span>
            </div>
            <div class="mt-0.5 truncate font-mono text-[11.5px] text-gray-500">
              {{ proc.executablePath }}
            </div>
          </div>
          <span class="shrink-0 text-[12px] font-medium text-[#0A7AFF]">选择</span>
        </button>

        <div
          v-if="filteredProcesses.length === 0"
          class="py-8 text-center text-[12.5px] text-gray-400"
        >
          没有匹配的运行中进程
        </div>
      </div>
    </div>
  </ModalDialog>
</template>
