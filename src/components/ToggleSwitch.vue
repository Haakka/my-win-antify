<script setup lang="ts">
defineProps<{
  modelValue: boolean;
  disabled?: boolean;
  size?: "sm" | "md";
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: boolean): void;
}>();
</script>

<template>
  <button
    type="button"
    role="switch"
    :aria-checked="modelValue"
    :disabled="disabled"
    class="relative inline-flex shrink-0 cursor-pointer items-center rounded-full transition-colors duration-200 focus:outline-none focus-visible:ring-2 focus-visible:ring-[#0A7AFF]/40 disabled:cursor-not-allowed disabled:opacity-50"
    :class="[
      size === 'sm' ? 'h-5 w-9' : 'h-6 w-10.5',
      modelValue ? 'bg-[#0A7AFF]' : 'bg-[#D1D5DB] dark:bg-[#3A3F4B]',
    ]"
    @click.stop="emit('update:modelValue', !modelValue)"
  >
    <span
      class="toggle-thumb inline-block transform rounded-full !bg-white shadow-sm transition-transform duration-200"
      :class="[
        size === 'sm' ? 'h-4 w-4' : 'h-5 w-5',
        modelValue
          ? size === 'sm'
            ? 'translate-x-4.5'
            : 'translate-x-5'
          : 'translate-x-0.5',
      ]"
    />
  </button>
</template>
