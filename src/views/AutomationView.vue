<script setup lang="ts">
import { ref } from "vue";
import { Radio, Wifi } from "lucide-vue-next";
import ToggleSwitch from "@/components/ToggleSwitch.vue";

const autoSwitchEnabled = ref(false);
</script>

<template>
  <div class="space-y-4">
    <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
      自动化
    </h1>

    <div class="antify-card flex items-center justify-between px-5 py-3.5">
      <span class="text-[13.5px] font-medium text-gray-900">自动切换配置</span>
      <ToggleSwitch v-model="autoSwitchEnabled" disabled size="sm" />
    </div>

    <div class="antify-card divide-y divide-gray-100 px-5 py-2">
      <div class="flex items-center gap-3 py-3">
        <Wifi class="h-4.5 w-4.5 text-[#0A7AFF]" />
        <div>
          <div class="text-[13.5px] font-semibold text-gray-900">Wi-Fi 网络</div>
          <div class="text-[12px] text-gray-500">
            连接到所选 Wi-Fi SSID 时自动切换到指定配置（Milestone 2 交付）。
          </div>
        </div>
      </div>

      <div class="flex items-center justify-between py-3">
        <div class="flex items-center gap-3">
          <Radio class="h-4.5 w-4.5 text-[#00B4D8]" />
          <div class="text-[12.5px] text-gray-500">
            Windows WLAN API 监听将在 Milestone 2 集成至 core-service。
          </div>
        </div>
        <span class="rounded-full bg-gray-100 px-2.5 py-0.5 text-[11px] text-gray-500">
          Milestone 2
        </span>
      </div>
    </div>
  </div>
</template>
