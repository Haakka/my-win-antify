<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  AlertTriangle,
  Check,
  Download,
  FileText,
  Monitor,
  Moon,
  Scale,
  Sun,
  Upload,
} from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { SKINS, useTheme } from "@/composables/useTheme";
import { normalizeLocale, useI18n } from "@/i18n";
import type { DiagnosticBundleSummary, ThemeMode } from "@/types";
import HelpTooltip from "@/components/HelpTooltip.vue";
import ModalDialog from "@/components/ModalDialog.vue";
import ToggleSwitch from "@/components/ToggleSwitch.vue";

const store = useAppStore();
const { skin, setSkin, setThemeMode } = useTheme();
const { t } = useI18n();
const activeTab = ref<"general" | "permissions" | "about">("general");
const bypassHostsText = ref("");
const bypassSaveState = ref<"idle" | "saved" | "error">("idle");
const bypassSaveError = ref<string | null>(null);
const launchAtLoginError = ref<string | null>(null);
let bypassSavedTimer: number | null = null;

const includePasswordsInBackup = ref(false);
const backupWarning = ref<string | null>(null);
const backupMessage = ref<string | null>(null);
const importModalOpen = ref(false);
const importJsonText = ref("");
const importError = ref<string | null>(null);
const diagnosticBusy = ref(false);
const diagnosticResult = ref<DiagnosticBundleSummary | null>(null);
const diagnosticError = ref<string | null>(null);
const licensesModalOpen = ref(false);

const thirdPartyLicenses = [
  {
    name: "WinDivert 2.2.2-A",
    license: "LGPL-3.0-or-later / GPL-2.0-or-later",
    author: "Copyright (c) 2019-2022 basil00",
    url: "https://reqrypt.org/windivert.html",
  },
  {
    name: "Tauri v2",
    license: "MIT / Apache-2.0",
    author: "Tauri Programme within The Commons Conservancy",
    url: "https://tauri.app",
  },
  {
    name: "Tokio & Rustls",
    license: "MIT / Apache-2.0 / ISC",
    author: "Tokio Contributors & Rustls Contributors",
    url: "https://tokio.rs",
  },
  {
    name: "Vue 3, Pinia, Tailwind CSS & Lucide",
    license: "MIT / ISC",
    author: "Evan You, Eduardo San Martin Morote, Tailwind Labs, Lucide",
    url: "https://vuejs.org",
  },
];

const themeModes = computed<{ value: ThemeMode; label: string; icon: typeof Sun }[]>(
  () => [
    { value: "light", label: t("settings.themeLight"), icon: Sun },
    { value: "dark", label: t("settings.themeDark"), icon: Moon },
    { value: "system", label: t("settings.themeSystem"), icon: Monitor },
  ],
);

const currentLocale = computed(() =>
  normalizeLocale(store.config.settings.language),
);

watch(
  () => store.config.settings.bypassHosts,
  (hosts) => {
    bypassHostsText.value = hosts.join("; ");
  },
  { immediate: true },
);

async function switchLanguage(lang: "zh-CN" | "en-US") {
  if (currentLocale.value === lang) return;
  await store.updateSettings({ language: lang });
}

async function saveBypassHosts() {
  if (bypassSavedTimer !== null) {
    window.clearTimeout(bypassSavedTimer);
    bypassSavedTimer = null;
  }
  bypassSaveError.value = null;
  const list = bypassHostsText.value
    .split(/[;\n,]/)
    .map((s) => s.trim())
    .filter(Boolean);
  try {
    await store.updateSettings({ bypassHosts: list });
    bypassSaveState.value = "saved";
    bypassSavedTimer = window.setTimeout(() => {
      bypassSaveState.value = "idle";
    }, 2500);
  } catch (err) {
    bypassSaveState.value = "error";
    bypassSaveError.value =
      err instanceof Error
        ? err.message
        : err && typeof err === "object" && "message" in err
          ? String((err as { message: unknown }).message)
          : "Invalid bypass host pattern";
  }
}

async function toggleLaunchAtLogin(enabled: boolean) {
  launchAtLoginError.value = null;
  try {
    await store.updateSettings({ launchAtLogin: enabled });
  } catch (err) {
    launchAtLoginError.value =
      err instanceof Error
        ? err.message
        : err && typeof err === "object" && "message" in err
          ? String((err as { message: unknown }).message)
          : "Registry update failed";
  }
}

async function handleExportBackup() {
  backupMessage.value = null;
  backupWarning.value = null;
  try {
    const exported = await store.exportConfigBackup(includePasswordsInBackup.value);
    backupWarning.value = exported.warning;
    const blob = new Blob([exported.json], { type: "application/json;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `my-win-antify-backup-${new Date().toISOString().slice(0, 10)}.json`;
    a.click();
    URL.revokeObjectURL(url);
    backupMessage.value = exported.includesPasswords
      ? t("settings.backupExportedPlain")
      : t("settings.backupExportedMasked");
  } catch (err) {
    backupMessage.value = err instanceof Error ? err.message : "Export failed";
  }
}

function handleImportFile(ev: Event) {
  const input = ev.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  const reader = new FileReader();
  reader.onload = () => {
    importJsonText.value = typeof reader.result === "string" ? reader.result : "";
  };
  reader.readAsText(file);
  input.value = "";
}

async function submitBackupImport() {
  importError.value = null;
  try {
    await store.importConfigBackup(importJsonText.value);
    importModalOpen.value = false;
    importJsonText.value = "";
    backupMessage.value = t("settings.backupImported");
  } catch (err) {
    importError.value =
      err instanceof Error
        ? err.message
        : err && typeof err === "object" && "message" in err
          ? String((err as { message: unknown }).message)
          : "Import failed";
  }
}

async function generateDiagnosticBundle() {
  diagnosticBusy.value = true;
  diagnosticError.value = null;
  try {
    diagnosticResult.value = await store.collectDiagnosticBundle(null);
  } catch (err) {
    diagnosticError.value =
      err instanceof Error ? err.message : "Failed to collect diagnostics";
  } finally {
    diagnosticBusy.value = false;
  }
}
</script>

<template>
  <div class="space-y-4">
    <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
      {{ t("settings.title") }}
    </h1>

    <div class="flex justify-center">
      <div class="inline-flex rounded-lg bg-gray-200/70 p-0.5 text-[12.5px]">
        <button
          v-for="tab in [
            { id: 'general', label: t('settings.tabGeneral') },
            { id: 'permissions', label: t('settings.tabPermissions') },
            { id: 'about', label: t('settings.tabAbout') },
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
        <div class="flex flex-wrap items-center justify-between gap-3 py-3.5">
          <div>
            <div class="text-[13px] font-medium text-gray-800">
              {{ t("settings.appearance") }}
            </div>
            <div class="text-[11.5px] text-gray-500">
              {{ t("settings.appearanceDesc") }}
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
            <div class="text-[13px] font-medium text-gray-800">
              {{ t("settings.skin") }}
            </div>
            <div class="text-[11.5px] text-gray-500">
              {{ t("settings.skinDesc") }}
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

        <div class="flex flex-wrap items-center justify-between gap-3 py-3.5">
          <div>
            <div class="text-[13px] font-medium text-gray-800">
              {{ t("settings.language") }}
            </div>
            <div class="text-[11.5px] text-gray-500">
              {{ t("settings.languageDesc") }}
            </div>
          </div>
          <div class="inline-flex rounded-lg bg-[#EFEFF2] p-0.5 text-[12px]">
            <button
              type="button"
              class="rounded-md px-3 py-1 font-medium transition"
              :class="
                currentLocale === 'zh-CN'
                  ? 'bg-[#0A7AFF] text-white shadow-2xs'
                  : 'text-gray-600 hover:text-gray-900'
              "
              @click="switchLanguage('zh-CN')"
            >
              简体中文
            </button>
            <button
              type="button"
              class="rounded-md px-3 py-1 font-medium transition"
              :class="
                currentLocale === 'en-US'
                  ? 'bg-[#0A7AFF] text-white shadow-2xs'
                  : 'text-gray-600 hover:text-gray-900'
              "
              @click="switchLanguage('en-US')"
            >
              English
            </button>
          </div>
        </div>
      </div>

      <div class="antify-card divide-y divide-gray-100 px-5">
        <div class="flex items-center justify-between py-3.5">
          <div>
            <div class="text-[13px] font-medium text-gray-800">
              {{ t("settings.bypassLan") }}
            </div>
            <div class="text-[11.5px] text-gray-500">
              {{ t("settings.bypassLanDesc") }}
            </div>
          </div>
          <ToggleSwitch
            size="sm"
            :model-value="store.config.settings.bypassLan"
            @update:model-value="(v) => store.updateSettings({ bypassLan: v })"
          />
        </div>

        <div class="flex items-center justify-between py-3.5">
          <div>
            <div class="text-[13px] font-medium text-gray-800">
              {{ t("settings.launchAtLogin") }}
            </div>
            <div class="text-[11.5px] text-gray-500">
              {{ t("settings.launchAtLoginDesc") }}
            </div>
            <div v-if="launchAtLoginError" class="mt-1 text-[11.5px] text-red-600">
              {{ launchAtLoginError }}
            </div>
          </div>
          <ToggleSwitch
            size="sm"
            :model-value="store.config.settings.launchAtLogin"
            @update:model-value="toggleLaunchAtLogin"
          />
        </div>

        <div class="flex items-center justify-between py-3.5">
          <span class="text-[13px] font-medium text-gray-800">
            {{ t("settings.autoStart") }}
          </span>
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
              {{ t("settings.bypassHosts") }}
            </span>
            <HelpTooltip :text="t('settings.bypassHostsHelp')" />
          </div>
          <div class="flex items-center gap-2.5">
            <span
              v-if="bypassSaveState === 'saved'"
              class="inline-flex items-center gap-1 text-[12px] font-medium text-emerald-600"
            >
              <Check class="h-3.5 w-3.5" />
              {{ t("settings.saved") }}
            </span>
            <button
              type="button"
              :disabled="store.busySaving"
              class="rounded-lg bg-gray-100 px-3 py-1 text-[12px] font-medium text-gray-700 hover:bg-gray-200 disabled:opacity-50"
              @click="saveBypassHosts"
            >
              {{ t("settings.saveList") }}
            </button>
          </div>
        </div>
        <input
          v-model="bypassHostsText"
          type="text"
          placeholder="*.corp.example.com; intranet.example.com"
          class="w-full rounded-lg border px-3 py-2 font-mono text-[12px] outline-none focus:border-[#0A7AFF]"
          :class="
            bypassSaveState === 'error'
              ? 'border-red-400 bg-red-50/40'
              : 'border-gray-200'
          "
          @input="
            bypassSaveState = 'idle';
            bypassSaveError = null;
          "
        />
        <p v-if="bypassSaveState === 'error' && bypassSaveError" class="text-[11.5px] text-red-600">
          {{ bypassSaveError }}
        </p>
      </div>

      <div class="antify-card space-y-3 p-5">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div>
            <div class="text-[13px] font-medium text-gray-800">
              {{ t("settings.backupTitle") }}
            </div>
            <div class="text-[11.5px] text-gray-500">
              {{ t("settings.backupDesc") }}
            </div>
          </div>
          <div class="flex items-center gap-2">
            <button
              type="button"
              class="inline-flex items-center gap-1.5 rounded-lg bg-gray-100 px-3 py-1.5 text-[12px] font-medium text-gray-700 transition hover:bg-gray-200"
              @click="handleExportBackup"
            >
              <Download class="h-3.5 w-3.5" />
              {{ t("settings.exportBackup") }}
            </button>
            <button
              type="button"
              class="inline-flex items-center gap-1.5 rounded-lg bg-[#0A7AFF] px-3 py-1.5 text-[12px] font-medium text-white transition hover:bg-blue-600"
              @click="
                importError = null;
                importModalOpen = true;
              "
            >
              <Upload class="h-3.5 w-3.5" />
              {{ t("settings.importBackup") }}
            </button>
          </div>
        </div>

        <label class="flex items-center gap-2 text-[12px] text-gray-600">
          <input
            v-model="includePasswordsInBackup"
            type="checkbox"
            class="h-3.5 w-3.5 rounded border-gray-300 accent-[#0A7AFF]"
          />
          <span>{{ t("settings.includePasswords") }}</span>
        </label>

        <div
          v-if="includePasswordsInBackup || backupWarning"
          class="flex items-center gap-2 rounded-lg bg-amber-50 px-3 py-2 text-[11.5px] text-amber-800 dark:bg-amber-950/30 dark:text-amber-300"
        >
          <AlertTriangle class="h-3.5 w-3.5 shrink-0" />
          <span>{{ backupWarning ?? t("settings.plaintextWarning") }}</span>
        </div>

        <div v-if="backupMessage" class="text-[12px] text-emerald-600">
          {{ backupMessage }}
        </div>
      </div>

      <div class="antify-card space-y-2.5 p-5">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div>
            <div class="text-[13px] font-medium text-gray-800">
              {{ t("settings.diagTitle") }}
            </div>
            <div class="text-[11.5px] text-gray-500">
              {{ t("settings.diagDesc") }}
            </div>
          </div>
          <button
            type="button"
            :disabled="diagnosticBusy"
            class="inline-flex items-center gap-1.5 rounded-lg bg-gray-100 px-3 py-1.5 text-[12px] font-medium text-gray-700 transition hover:bg-gray-200 disabled:opacity-50"
            @click="generateDiagnosticBundle"
          >
            <FileText class="h-3.5 w-3.5" />
            {{ diagnosticBusy ? t("settings.diagGenerating") : t("settings.diagGenerate") }}
          </button>
        </div>

        <div
          v-if="diagnosticResult"
          class="rounded-lg bg-emerald-50/80 px-3 py-2 text-[11.5px] text-emerald-800 dark:bg-emerald-950/30 dark:text-emerald-300"
        >
          <div>
            {{ t("settings.diagSavedTo") }}
            <code class="font-mono">{{ diagnosticResult.path }}</code>
          </div>
          <div class="mt-0.5 opacity-85">
            {{
              t("settings.diagSummary", {
                logs: diagnosticResult.logCount,
                nics: diagnosticResult.interfaceCount,
                fields: diagnosticResult.sanitizedFields.join(", "),
              })
            }}
          </div>
        </div>
        <div v-if="diagnosticError" class="text-[12px] text-red-600">
          {{ diagnosticError }}
        </div>
      </div>
    </div>

    <div v-else-if="activeTab === 'permissions'" class="antify-card space-y-3 p-5 text-[13px]">
      <div class="flex items-center justify-between">
        <span class="text-gray-600">{{ t("settings.pipeChannel") }}</span>
        <code class="rounded bg-gray-100 px-2 py-0.5 text-[12px]">
          \\.\pipe\my-win-antify-core-v1
        </code>
      </div>
      <div class="flex items-center justify-between">
        <span class="text-gray-600">{{ t("settings.pipeState") }}</span>
        <span class="font-medium text-gray-900">{{ store.pipeState }}</span>
      </div>
      <div class="flex items-center justify-between">
        <span class="text-gray-600">{{ t("settings.driverReady") }}</span>
        <span
          class="font-medium"
          :class="store.status.driverReady ? 'text-emerald-600' : 'text-amber-600'"
        >
          {{ store.status.driverReady ? t("settings.driverYes") : t("settings.driverNo") }}
        </span>
      </div>
      <div class="flex items-center justify-between">
        <span class="text-gray-600">{{ t("settings.corePid") }}</span>
        <span class="font-mono text-gray-800">{{ store.status.servicePid || "—" }}</span>
      </div>
    </div>

    <div v-else class="antify-card space-y-3 p-5 text-[13px] text-gray-600">
      <div class="flex items-center justify-between">
        <div class="text-[15px] font-semibold text-gray-900">
          MyWinAntify v0.5.0
        </div>
        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-lg bg-gray-100 px-3 py-1.5 text-[12px] font-medium text-gray-700 transition hover:bg-gray-200"
          @click="licensesModalOpen = true"
        >
          <Scale class="h-3.5 w-3.5" />
          {{ t("settings.licensesBtn") }}
        </button>
      </div>
      <p>{{ t("settings.aboutP1") }}</p>
      <p>{{ t("settings.aboutP2") }}</p>
      <p class="text-[12px] text-gray-400">
        {{ t("settings.aboutConfigNote") }}
      </p>
    </div>

    <ModalDialog
      :open="importModalOpen"
      :title="t('settings.importModalTitle')"
      @close="importModalOpen = false"
    >
      <div class="space-y-3 text-[12.5px]">
        <div v-if="importError" class="rounded-lg bg-red-50 px-3 py-2 text-red-600">
          {{ importError }}
        </div>
        <input
          type="file"
          accept=".json,application/json"
          class="block w-full text-[12px] text-gray-600 file:mr-3 file:rounded-lg file:border-0 file:bg-gray-100 file:px-3 file:py-1.5 file:text-[12px] file:font-medium file:text-gray-700 hover:file:bg-gray-200"
          @change="handleImportFile"
        />
        <textarea
          v-model="importJsonText"
          rows="8"
          :placeholder="t('settings.importPlaceholder')"
          class="w-full rounded-xl border border-gray-200 p-3 font-mono text-[11.5px] outline-none focus:border-[#0A7AFF]"
        />
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
          :disabled="!importJsonText.trim() || store.busySaving"
          class="rounded-lg bg-[#0A7AFF] px-4 py-1.5 text-[12.5px] font-medium text-white hover:bg-blue-600 disabled:opacity-50"
          @click="submitBackupImport"
        >
          {{ t("settings.confirmImport") }}
        </button>
      </template>
    </ModalDialog>

    <ModalDialog
      :open="licensesModalOpen"
      :title="t('settings.licensesTitle')"
      @close="licensesModalOpen = false"
    >
      <div class="space-y-3 text-[12.5px]">
        <div
          v-for="item in thirdPartyLicenses"
          :key="item.name"
          class="rounded-xl border border-gray-100 bg-gray-50/70 p-3 dark:bg-[#252830]"
        >
          <div class="flex items-center justify-between gap-2">
            <span class="font-semibold text-gray-900">{{ item.name }}</span>
            <span class="rounded-full bg-blue-50 px-2 py-0.5 text-[11px] font-medium text-[#0A7AFF]">
              {{ item.license }}
            </span>
          </div>
          <div class="mt-1 text-[11.5px] text-gray-500">{{ item.author }}</div>
          <div class="mt-0.5 font-mono text-[11px] text-gray-400">{{ item.url }}</div>
        </div>
      </div>
      <template #footer>
        <button
          type="button"
          class="rounded-lg bg-[#0A7AFF] px-4 py-1.5 text-[12.5px] font-medium text-white hover:bg-blue-600"
          @click="licensesModalOpen = false"
        >
          {{ t("common.close") }}
        </button>
      </template>
    </ModalDialog>
  </div>
</template>
