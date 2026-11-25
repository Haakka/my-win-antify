<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref } from "vue";
import { Check, ChevronsUpDown } from "lucide-vue-next";

export interface SelectOption {
  value: string;
  label: string;
  description?: string;
  disabled?: boolean;
}

const props = withDefaults(
  defineProps<{
    modelValue: string;
    options: SelectOption[];
    placeholder?: string;
    variant?: "inline" | "pill";
  }>(),
  {
    placeholder: "请选择...",
    variant: "inline",
  },
);

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
}>();

const triggerRef = ref<HTMLButtonElement | null>(null);
const menuRef = ref<HTMLDivElement | null>(null);
const open = ref(false);
const menuStyle = ref<Record<string, string>>({});

const selectedOption = computed(() =>
  props.options.find((o) => o.value === props.modelValue),
);

async function positionMenu() {
  await nextTick();
  const trigger = triggerRef.value;
  if (!trigger) return;
  const rect = trigger.getBoundingClientRect();
  const minWidth = Math.max(rect.width + 24, 180);
  const maxWidth = 280;
  const left = Math.min(
    Math.max(12, rect.right - minWidth),
    window.innerWidth - minWidth - 12,
  );
  const spaceBelow = window.innerHeight - rect.bottom;
  const openAbove = spaceBelow < 190 && rect.top > 190;
  menuStyle.value = {
    left: `${left}px`,
    minWidth: `${minWidth}px`,
    maxWidth: `${maxWidth}px`,
    top: openAbove ? `${rect.top - 6}px` : `${rect.bottom + 6}px`,
    transform: openAbove ? "translateY(-100%)" : "none",
  };
}

function toggle() {
  open.value = !open.value;
  if (open.value) {
    void positionMenu();
  }
}

function choose(opt: SelectOption) {
  if (opt.disabled) return;
  emit("update:modelValue", opt.value);
  open.value = false;
}

function onPointerDown(ev: MouseEvent) {
  if (!open.value) return;
  const target = ev.target as Node | null;
  if (
    target &&
    !triggerRef.value?.contains(target) &&
    !menuRef.value?.contains(target)
  ) {
    open.value = false;
  }
}

function onKeyDown(ev: KeyboardEvent) {
  if (open.value && ev.key === "Escape") {
    ev.preventDefault();
    ev.stopPropagation();
    ev.stopImmediatePropagation();
    open.value = false;
  }
}

window.addEventListener("mousedown", onPointerDown);
window.addEventListener("keydown", onKeyDown, true);
onBeforeUnmount(() => {
  window.removeEventListener("mousedown", onPointerDown);
  window.removeEventListener("keydown", onKeyDown, true);
});
</script>

<template>
  <button
    ref="triggerRef"
    type="button"
    class="inline-flex items-center gap-2 text-[13px] font-medium text-gray-900 transition focus:outline-none focus-visible:ring-2 focus-visible:ring-[#0A7AFF]/40"
    :class="
      variant === 'pill'
        ? 'h-8 rounded-lg bg-[#EFEFF2] px-3 hover:bg-[#E5E5EA]'
        : 'rounded-lg px-1 py-0.5 hover:bg-gray-200/50'
    "
    @click.stop="toggle"
  >
    <slot name="icon" />
    <span class="max-w-[170px] truncate">
      {{ selectedOption?.label ?? placeholder }}
    </span>
    <span
      class="flex h-5 w-5 shrink-0 items-center justify-center rounded-full"
      :class="variant === 'pill' ? 'text-gray-500' : 'bg-gray-200/90 text-gray-600'"
    >
      <ChevronsUpDown class="h-3 w-3" />
    </span>
  </button>

  <Teleport to="body">
    <div
      v-if="open"
      ref="menuRef"
      class="fixed z-[95] max-h-64 overflow-y-auto rounded-xl border border-gray-200/90 bg-white/95 p-1 shadow-xl backdrop-blur-md dark:border-white/10 dark:bg-[#22252D]/95"
      :style="menuStyle"
    >
      <button
        v-for="opt in options"
        :key="opt.value"
        type="button"
        :disabled="opt.disabled"
        class="flex w-full items-center justify-between gap-3 rounded-lg px-2.5 py-1.5 text-left text-[12.5px] transition"
        :class="
          opt.disabled
            ? 'cursor-not-allowed text-gray-300'
            : opt.value === modelValue
              ? 'bg-[#0A7AFF] text-white'
              : 'text-gray-800 hover:bg-gray-100'
        "
        @click="choose(opt)"
      >
        <div class="min-w-0">
          <div class="truncate font-medium">{{ opt.label }}</div>
          <div
            v-if="opt.description"
            class="truncate text-[11px]"
            :class="
              opt.value === modelValue ? 'text-blue-100' : 'text-gray-400'
            "
          >
            {{ opt.description }}
          </div>
        </div>
        <Check
          v-if="opt.value === modelValue"
          class="h-3.5 w-3.5 shrink-0"
        />
      </button>
    </div>
  </Teleport>
</template>
