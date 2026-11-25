<script setup lang="ts">
import { AlertCircle, RefreshCw, Sparkles } from "lucide-vue-next";
import { useAppStore } from "@/stores/app";

const store = useAppStore();
</script>

<template>
  <div
    v-if="!store.mockMode && store.pipeState !== 'connected'"
    class="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-amber-200/90 bg-amber-50/90 px-4 py-2.5 text-[12.5px] text-amber-900"
  >
    <div class="flex items-center gap-2.5">
      <AlertCircle class="h-4 w-4 shrink-0 text-amber-600" />
      <span>
        后台特权服务尚未运行，点击右侧「启动后台服务」并在系统授权弹窗点击“是”，或开启 Mock 预览体验全部界面交互。
      </span>
    </div>
    <div class="flex items-center gap-2">
      <button
        type="button"
        class="inline-flex items-center gap-1.5 rounded-lg border border-amber-300 bg-white px-2.5 py-1 text-[12px] font-medium text-amber-800 transition hover:bg-amber-100/60 dark:border-amber-500/30 dark:bg-[#252830] dark:hover:bg-[#2C303A]"
        @click="store.reconnectService()"
      >
        <RefreshCw class="h-3.5 w-3.5" />
        启动后台服务
      </button>
      <button
        type="button"
        class="inline-flex items-center gap-1.5 rounded-lg bg-[#0A7AFF] px-2.5 py-1 text-[12px] font-medium text-white transition hover:bg-blue-600"
        @click="store.setMockMode(true)"
      >
        <Sparkles class="h-3.5 w-3.5" />
        开启 Mock 预览
      </button>
    </div>
  </div>
</template>
