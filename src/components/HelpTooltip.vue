<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref } from "vue";
import { HelpCircle } from "lucide-vue-next";

defineProps<{
  text: string;
}>();

const triggerRef = ref<HTMLButtonElement | null>(null);
const open = ref(false);
const stylePos = ref<{ top: string; left: string }>({
  top: "0px",
  left: "0px",
});
let hideTimer: number | null = null;

function cancelHide() {
  if (hideTimer !== null) {
    window.clearTimeout(hideTimer);
    hideTimer = null;
  }
}

async function updatePosition() {
  await nextTick();
  const el = triggerRef.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  const maxWidth = 240;
  const left = Math.min(
    Math.max(12, rect.left + rect.width / 2 - maxWidth / 2),
    window.innerWidth - maxWidth - 12,
  );
  const placeBelow = rect.top < 110;
  const top = placeBelow ? rect.bottom + 8 : rect.top - 8;
  stylePos.value = {
    top: `${top}px`,
    left: `${left}px`,
  };
  placeAtBottom.value = placeBelow;
}

const placeAtBottom = ref(false);

function show() {
  cancelHide();
  open.value = true;
  void updatePosition();
}

function scheduleHide() {
  cancelHide();
  hideTimer = window.setTimeout(() => {
    open.value = false;
  }, 100);
}

function toggleClick() {
  if (open.value) {
    open.value = false;
  } else {
    show();
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

window.addEventListener("keydown", onKeyDown, true);

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeyDown, true);
  cancelHide();
});
</script>

<template>
  <button
    ref="triggerRef"
    type="button"
    :aria-label="text"
    class="inline-flex h-4 w-4 items-center justify-center rounded-full text-gray-400 transition hover:text-gray-600 focus:outline-none focus-visible:ring-2 focus-visible:ring-[#0A7AFF]/40"
    @mouseenter="show"
    @mouseleave="scheduleHide"
    @focus="show"
    @blur="scheduleHide"
    @click.stop="toggleClick"
  >
    <HelpCircle class="h-3.5 w-3.5" />
  </button>

  <Teleport to="body">
    <div
      v-if="open"
      class="pointer-events-auto fixed z-[90] w-[240px] rounded-xl bg-[#1F2937]/95 px-3 py-2 text-[11.5px] leading-relaxed text-white shadow-xl backdrop-blur-xs"
      :class="placeAtBottom ? '' : '-translate-y-full'"
      :style="stylePos"
      @mouseenter="cancelHide"
      @mouseleave="scheduleHide"
    >
      {{ text }}
    </div>
  </Teleport>
</template>
