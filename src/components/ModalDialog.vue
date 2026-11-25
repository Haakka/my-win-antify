<script setup lang="ts">
import { X } from "lucide-vue-next";

defineProps<{
  open: boolean;
  title: string;
  widthClass?: string;
}>();

const emit = defineEmits<{
  (e: "close"): void;
}>();
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/25 p-4 backdrop-blur-[2px] dark:bg-black/60"
      @mousedown.self="emit('close')"
    >
      <div
        class="flex max-h-[88vh] w-full flex-col overflow-hidden rounded-2xl border border-gray-200/80 bg-white shadow-2xl dark:border-white/10 dark:bg-[#1C1E24]"
        :class="widthClass ?? 'max-w-lg'"
      >
        <div
          class="flex items-center justify-between border-b border-gray-100 px-5 py-3.5"
        >
          <h3 class="text-[15px] font-semibold text-gray-900">{{ title }}</h3>
          <button
            type="button"
            class="rounded-lg p-1 text-gray-400 transition hover:bg-gray-100 hover:text-gray-600"
            @click="emit('close')"
          >
            <X class="h-4 w-4" />
          </button>
        </div>

        <div class="overflow-y-auto px-5 py-4">
          <slot />
        </div>

        <div
          v-if="$slots.footer"
          class="flex items-center justify-end gap-2.5 border-t border-gray-100 bg-gray-50/70 px-5 py-3"
        >
          <slot name="footer" />
        </div>
      </div>
    </div>
  </Teleport>
</template>
