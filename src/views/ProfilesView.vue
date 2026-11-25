<script setup lang="ts">
import { ref } from "vue";
import { Globe, ListChecks, Pencil, Plus, Trash2 } from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import type { Profile } from "@/types";
import ProfileFormModal from "@/components/ProfileFormModal.vue";

const store = useAppStore();
const modalOpen = ref(false);
const editing = ref<Profile | null>(null);

function ruleCountFor(profileId: string): number {
  return store.config.rules.filter((r) => r.profileId === profileId).length;
}

function openCreate() {
  editing.value = null;
  modalOpen.value = true;
}

function openEdit(prof: Profile) {
  editing.value = prof;
  modalOpen.value = true;
}

async function handleSave(prof: Profile) {
  await store.upsertProfile(prof);
  modalOpen.value = false;
}
</script>

<template>
  <div class="space-y-4">
    <div class="flex items-center justify-between">
      <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
        配置
      </h1>
      <button
        type="button"
        title="新增配置"
        class="inline-flex h-7 w-8 items-center justify-center rounded-lg border border-gray-200 bg-white text-gray-700 transition hover:bg-gray-50"
        @click="openCreate"
      >
        <Plus class="h-4 w-4" />
      </button>
    </div>

    <div class="space-y-3">
      <div
        v-for="prof in store.config.profiles"
        :key="prof.id"
        class="flex cursor-pointer items-center justify-between rounded-2xl bg-white p-4 transition"
        :class="
          prof.id === store.activeProfile.id
            ? 'border-2 border-[#0A7AFF]/60 shadow-xs'
            : 'border border-gray-200/80 hover:border-gray-300'
        "
        @click="store.setActiveProfile(prof.id)"
      >
        <div class="flex items-center gap-3.5">
          <div
            class="flex h-11 w-11 items-center justify-center rounded-full bg-blue-50 text-[#0A7AFF]"
          >
            <Globe class="h-5 w-5" />
          </div>
          <div>
            <div class="flex items-center gap-2">
              <span class="text-[14.5px] font-semibold text-gray-900">
                {{ prof.name }}
              </span>
              <span
                v-if="prof.id === store.activeProfile.id"
                class="rounded-full bg-emerald-100 px-2 py-0.5 text-[11px] font-medium text-emerald-700"
              >
                已启用
              </span>
            </div>
            <div class="mt-1 flex items-center gap-3 text-[12px] text-gray-500">
              <span class="inline-flex items-center gap-1">
                <ListChecks class="h-3.5 w-3.5" />
                {{ ruleCountFor(prof.id) }} 条规则
              </span>
              <span
                class="rounded-full bg-emerald-50 px-2 py-0.5 text-[11px] font-medium text-emerald-700"
              >
                默认:
                {{
                  prof.defaultAction === "direct"
                    ? "直连"
                    : prof.defaultAction === "proxy"
                      ? "代理"
                      : "阻断"
                }}
              </span>
            </div>
          </div>
        </div>

        <div class="flex items-center gap-2" @click.stop>
          <button
            type="button"
            class="rounded-lg p-1.5 text-gray-500 transition hover:bg-gray-100 hover:text-gray-800"
            @click="openEdit(prof)"
          >
            <Pencil class="h-4 w-4" />
          </button>
          <button
            v-if="store.config.profiles.length > 1"
            type="button"
            class="rounded-lg p-1.5 text-red-400 transition hover:bg-red-50 hover:text-red-600"
            @click="store.deleteProfile(prof.id)"
          >
            <Trash2 class="h-4 w-4" />
          </button>
        </div>
      </div>
    </div>

    <ProfileFormModal
      :open="modalOpen"
      :editing="editing"
      @close="modalOpen = false"
      @save="handleSave"
    />
  </div>
</template>
