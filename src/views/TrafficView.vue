<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { ArrowDown, ArrowUp, Trash2 } from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { useI18n } from "@/i18n";
import type { TrafficHistoryRange } from "@/types";
import { formatBps, formatBytes } from "@/utils/format";
import HelpTooltip from "@/components/HelpTooltip.vue";
import SelectMenu, { type SelectOption } from "@/components/SelectMenu.vue";

const store = useAppStore();
const { t } = useI18n();
const breakdownTab = ref<"rule" | "proxy" | "app" | "host">("rule");
const historyRange = ref<TrafficHistoryRange>("24h");
const historyProfileId = ref<string>("");
const hoveredBarIndex = ref<number | null>(null);

const historyProfileOptions = computed<SelectOption[]>(() => [
  { value: "", label: t("traffic.allProfiles") },
  ...store.config.profiles.map((p) => ({ value: p.id, label: p.name })),
]);

onMounted(() => {
  void loadHistory();
});

async function loadHistory() {
  await store.refreshTrafficHistory(
    historyRange.value,
    historyProfileId.value || null,
  );
}

function switchRange(range: TrafficHistoryRange) {
  historyRange.value = range;
  void loadHistory();
}

function changeHistoryProfile(profileId: string | number) {
  historyProfileId.value = String(profileId);
  void loadHistory();
}

const historyBuckets = computed(() => store.trafficHistory?.buckets ?? []);

const historyMaxTotal = computed(() => {
  const totals = historyBuckets.value.map(
    (b) => b.proxiedUp + b.proxiedDown + b.directUp + b.directDown,
  );
  return Math.max(1024, ...totals);
});

const hoveredBucket = computed(() => {
  if (hoveredBarIndex.value === null) return null;
  return historyBuckets.value[hoveredBarIndex.value] ?? null;
});

const CHART_W = 600;
const CHART_H = 120;
const PAD_TOP = 8;
const PAD_BOTTOM = 4;
const BASELINE_Y = CHART_H - PAD_BOTTOM;
const PLOT_H = CHART_H - PAD_TOP - PAD_BOTTOM;

interface Point {
  x: number;
  y: number;
}

const hoverIndex = ref<number | null>(null);

const paddedHistory = computed(() => {
  const slots = store.speedHistory.slice(-60);
  const padCount = Math.max(0, 60 - slots.length);
  const padding = Array.from({ length: padCount }, () => ({
    ts: 0,
    upBps: 0,
    downBps: 0,
  }));
  return [...padding, ...slots];
});

function niceSpeedCeiling(raw: number): number {
  const target = Math.max(1024, raw);
  let unit = 1024;
  while (target > unit * 500 && unit < 1024 ** 4) {
    unit *= 1024;
  }
  const steps = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1024];
  for (const step of steps) {
    const candidate = step * unit;
    if (candidate >= target) return candidate;
  }
  return Math.ceil(target / unit) * unit;
}

const yCeiling = computed(() => {
  const peaks = paddedHistory.value.map((p) => Math.max(p.upBps, p.downBps));
  return niceSpeedCeiling(Math.max(1024, ...peaks));
});

function toY(bps: number): number {
  const ratio = Math.max(0, Math.min(1, bps / yCeiling.value));
  return BASELINE_Y - ratio * PLOT_H;
}

const downPoints = computed<Point[]>(() =>
  paddedHistory.value.map((pt, i) => ({
    x: (i / 59) * CHART_W,
    y: toY(pt.downBps),
  })),
);

const upPoints = computed<Point[]>(() =>
  paddedHistory.value.map((pt, i) => ({
    x: (i / 59) * CHART_W,
    y: toY(pt.upBps),
  })),
);

function buildSmoothPath(points: Point[]): string {
  const n = points.length;
  if (n === 0) return "";
  const first = points[0]!;
  if (n === 1) return `M ${first.x.toFixed(2)} ${first.y.toFixed(2)}`;

  const slopes: number[] = new Array(n - 1).fill(0);
  for (let i = 0; i < n - 1; i++) {
    const p0 = points[i]!;
    const p1 = points[i + 1]!;
    const dx = p1.x - p0.x;
    slopes[i] = dx > 0 ? (p1.y - p0.y) / dx : 0;
  }

  const tangents: number[] = new Array(n).fill(0);
  tangents[0] = slopes[0]!;
  tangents[n - 1] = slopes[n - 2]!;
  for (let i = 1; i < n - 1; i++) {
    const prev = slopes[i - 1]!;
    const next = slopes[i]!;
    tangents[i] = prev * next <= 0 ? 0 : (prev + next) / 2;
  }

  for (let i = 0; i < n - 1; i++) {
    const d = slopes[i]!;
    if (d === 0) {
      tangents[i] = 0;
      tangents[i + 1] = 0;
      continue;
    }
    const a = tangents[i]! / d;
    const b = tangents[i + 1]! / d;
    const s = a * a + b * b;
    if (s > 9) {
      const k = 3 / Math.sqrt(s);
      tangents[i] = k * a * d;
      tangents[i + 1] = k * b * d;
    }
  }

  let path = `M ${first.x.toFixed(2)} ${first.y.toFixed(2)}`;
  for (let i = 0; i < n - 1; i++) {
    const p0 = points[i]!;
    const p1 = points[i + 1]!;
    const h = p1.x - p0.x;
    const cp1x = p0.x + h / 3;
    const cp1y = p0.y + (tangents[i]! * h) / 3;
    const cp2x = p1.x - h / 3;
    const cp2y = p1.y - (tangents[i + 1]! * h) / 3;
    path += ` C ${cp1x.toFixed(2)} ${cp1y.toFixed(2)}, ${cp2x.toFixed(2)} ${cp2y.toFixed(2)}, ${p1.x.toFixed(2)} ${p1.y.toFixed(2)}`;
  }
  return path;
}

function buildAreaPath(linePath: string, points: Point[]): string {
  if (!linePath || points.length === 0) return "";
  const first = points[0]!;
  const last = points[points.length - 1]!;
  return `${linePath} L ${last.x.toFixed(2)} ${BASELINE_Y.toFixed(2)} L ${first.x.toFixed(2)} ${BASELINE_Y.toFixed(2)} Z`;
}

const downLinePath = computed(() => buildSmoothPath(downPoints.value));
const downAreaPath = computed(() =>
  buildAreaPath(downLinePath.value, downPoints.value),
);
const upLinePath = computed(() => buildSmoothPath(upPoints.value));
const upAreaPath = computed(() =>
  buildAreaPath(upLinePath.value, upPoints.value),
);

const hoveredSample = computed(() => {
  if (hoverIndex.value === null) return null;
  const idx = hoverIndex.value;
  const pt = paddedHistory.value[idx];
  const downPt = downPoints.value[idx];
  const upPt = upPoints.value[idx];
  if (!pt || !downPt || !upPt) return null;
  const secondsAgo = 59 - idx;
  return {
    idx,
    xPct: (idx / 59) * 100,
    downYPct: (downPt.y / CHART_H) * 100,
    upYPct: (upPt.y / CHART_H) * 100,
    timeLabel:
      secondsAgo === 0
        ? t("traffic.now")
        : t("traffic.secsAgo", { sec: secondsAgo }),
    downBps: pt.downBps,
    upBps: pt.upBps,
  };
});

function handleChartMouseMove(event: MouseEvent) {
  const el = event.currentTarget as HTMLElement | null;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  if (rect.width <= 0) return;
  const ratio = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
  hoverIndex.value = Math.round(ratio * 59);
}

function handleChartMouseLeave() {
  hoverIndex.value = null;
}

interface BreakdownRow {
  key: string;
  label: string;
  upBytes: number;
  downBytes: number;
  connections: number;
}

const breakdownRows = computed<BreakdownRow[]>(() => {
  const map = new Map<string, BreakdownRow>();
  for (const conn of store.connections) {
    if (
      !store.config.settings.discoverUnmatched &&
      !conn.ruleId &&
      conn.action === "direct"
    ) {
      continue;
    }

    let key = "";
    let label = "";
    if (breakdownTab.value === "rule") {
      key = conn.ruleId ?? "__default__";
      label = conn.ruleName ?? t("traffic.defaultLabel");
    } else if (breakdownTab.value === "proxy") {
      if (conn.action === "proxy") {
        key = conn.proxyId ?? "__proxy__";
        label = conn.proxyName ?? t("common.proxy");
      } else if (conn.action === "block") {
        key = "__block__";
        label = t("connections.blocked");
      } else {
        key = "__direct__";
        label = t("common.direct");
      }
    } else if (breakdownTab.value === "app") {
      key = conn.executablePath || conn.processName || conn.appName;
      label = conn.appName || conn.processName || t("traffic.unknownApp");
    } else {
      const host = conn.domain || conn.dstAddr.split(":")[0] || conn.dstAddr;
      key = host;
      label = host;
    }

    const existing = map.get(key);
    if (existing) {
      existing.upBytes += conn.bytesUp;
      existing.downBytes += conn.bytesDown;
      existing.connections += 1;
    } else {
      map.set(key, {
        key,
        label,
        upBytes: conn.bytesUp,
        downBytes: conn.bytesDown,
        connections: 1,
      });
    }
  }

  return Array.from(map.values()).sort(
    (a, b) => b.upBytes + b.downBytes - (a.upBytes + a.downBytes),
  );
});
</script>

<template>
  <div class="space-y-4">
    <div class="flex items-center justify-between gap-3">
      <h1 class="text-[18px] font-semibold tracking-tight text-gray-900">
        {{ t("traffic.title") }}
      </h1>
      <div class="flex items-center gap-1.5 text-[12px] text-gray-400">
        <span>{{ t("traffic.scopeNote") }}</span>
        <HelpTooltip :text="t('traffic.scopeHelp')" />
      </div>
    </div>

    <div class="antify-card space-y-5 p-5">
      <div class="flex flex-wrap gap-8">
        <div>
          <div class="text-[11.5px] text-gray-500">{{ t("traffic.proxied") }}</div>
          <div class="mt-1 flex items-center gap-3 text-[15px] font-semibold text-gray-900">
            <span class="inline-flex items-center gap-1">
              <ArrowUp class="h-3.5 w-3.5 text-orange-500" />
              {{ formatBytes(store.stats.totalProxiedUp) }}
            </span>
            <span class="inline-flex items-center gap-1">
              <ArrowDown class="h-3.5 w-3.5 text-blue-500" />
              {{ formatBytes(store.stats.totalProxiedDown) }}
            </span>
          </div>
        </div>

        <div>
          <div class="text-[11.5px] text-gray-500">{{ t("traffic.direct") }}</div>
          <div class="mt-1 flex items-center gap-3 text-[15px] font-semibold text-gray-900">
            <span class="inline-flex items-center gap-1">
              <ArrowUp class="h-3.5 w-3.5 text-orange-500" />
              {{ formatBytes(store.stats.totalDirectUp) }}
            </span>
            <span class="inline-flex items-center gap-1">
              <ArrowDown class="h-3.5 w-3.5 text-blue-500" />
              {{ formatBytes(store.stats.totalDirectDown) }}
            </span>
          </div>
        </div>
      </div>

      <div>
        <div class="mb-2 flex items-center justify-between text-[12px]">
          <span class="font-semibold text-gray-800">{{ t("traffic.last60s") }}</span>
          <div class="flex items-center gap-4 text-gray-600">
            <span class="inline-flex items-center gap-1.5">
              <span class="h-2 w-2 rounded-full bg-blue-500" />
              {{ t("traffic.download") }} {{ formatBps(store.stats.downloadBps) }}
            </span>
            <span class="inline-flex items-center gap-1.5">
              <span class="h-2 w-2 rounded-full bg-orange-500" />
              {{ t("traffic.upload") }} {{ formatBps(store.stats.uploadBps) }}
            </span>
          </div>
        </div>

        <div class="rounded-xl border border-gray-100 bg-gray-50/70 px-3 pt-3 pb-2">
          <div class="flex items-stretch gap-2.5">
            <div
              class="flex h-32 w-14 shrink-0 flex-col justify-between py-0.5 text-right font-mono text-[10px] leading-none text-gray-400 tabular-nums"
            >
              <span>{{ formatBps(yCeiling) }}</span>
              <span>{{ formatBps(yCeiling / 2) }}</span>
              <span>0 B/s</span>
            </div>

            <div
              class="relative h-32 flex-1 cursor-crosshair select-none"
              @mousemove="handleChartMouseMove"
              @mouseleave="handleChartMouseLeave"
            >
              <svg
                :viewBox="`0 0 ${CHART_W} ${CHART_H}`"
                preserveAspectRatio="none"
                class="h-full w-full overflow-visible"
              >
                <defs>
                  <linearGradient id="trafficDownAreaGradient" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stop-color="#3B82F6" stop-opacity="0.22" />
                    <stop offset="100%" stop-color="#3B82F6" stop-opacity="0.01" />
                  </linearGradient>
                  <linearGradient id="trafficUpAreaGradient" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stop-color="#F97316" stop-opacity="0.18" />
                    <stop offset="100%" stop-color="#F97316" stop-opacity="0.01" />
                  </linearGradient>
                </defs>

                <line
                  x1="0"
                  :y1="PAD_TOP"
                  :x2="CHART_W"
                  :y2="PAD_TOP"
                  stroke="var(--chart-grid)"
                  stroke-dasharray="3 3"
                  stroke-width="1"
                  vector-effect="non-scaling-stroke"
                />
                <line
                  x1="0"
                  :y1="PAD_TOP + PLOT_H / 2"
                  :x2="CHART_W"
                  :y2="PAD_TOP + PLOT_H / 2"
                  stroke="var(--chart-grid)"
                  stroke-dasharray="3 3"
                  stroke-width="1"
                  vector-effect="non-scaling-stroke"
                />
                <line
                  x1="0"
                  :y1="BASELINE_Y"
                  :x2="CHART_W"
                  :y2="BASELINE_Y"
                  stroke="var(--chart-grid)"
                  stroke-width="1"
                  vector-effect="non-scaling-stroke"
                />

                <path :d="downAreaPath" fill="url(#trafficDownAreaGradient)" />
                <path :d="upAreaPath" fill="url(#trafficUpAreaGradient)" />

                <path
                  :d="downLinePath"
                  fill="none"
                  stroke="#3B82F6"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  vector-effect="non-scaling-stroke"
                />
                <path
                  :d="upLinePath"
                  fill="none"
                  stroke="#F97316"
                  stroke-width="1.8"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  vector-effect="non-scaling-stroke"
                />
              </svg>

              <template v-if="hoveredSample">
                <div
                  class="pointer-events-none absolute top-0 bottom-0 w-px bg-gray-300/90 dark:bg-white/25"
                  :style="{ left: `${hoveredSample.xPct}%` }"
                />
                <div
                  class="pointer-events-none absolute h-2.5 w-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white bg-blue-500 shadow-xs dark:border-[#1C1E24]"
                  :style="{
                    left: `${hoveredSample.xPct}%`,
                    top: `${hoveredSample.downYPct}%`,
                  }"
                />
                <div
                  class="pointer-events-none absolute h-2.5 w-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white bg-orange-500 shadow-xs dark:border-[#1C1E24]"
                  :style="{
                    left: `${hoveredSample.xPct}%`,
                    top: `${hoveredSample.upYPct}%`,
                  }"
                />
                <div
                  class="pointer-events-none absolute top-1 z-10 rounded-lg border border-gray-200/90 bg-white/95 px-2.5 py-1.5 text-[11px] shadow-md backdrop-blur-xs dark:border-white/10 dark:bg-[#22252D]/95"
                  :style="
                    hoveredSample.idx > 40
                      ? { right: `${100 - hoveredSample.xPct + 1.5}%` }
                      : { left: `${hoveredSample.xPct + 1.5}%` }
                  "
                >
                  <div class="mb-0.5 font-medium text-gray-500">
                    {{ hoveredSample.timeLabel }}
                  </div>
                  <div class="flex items-center gap-1.5 font-mono text-gray-800">
                    <span class="h-1.5 w-1.5 rounded-full bg-blue-500" />
                    {{ t("traffic.download") }} {{ formatBps(hoveredSample.downBps) }}
                  </div>
                  <div class="flex items-center gap-1.5 font-mono text-gray-800">
                    <span class="h-1.5 w-1.5 rounded-full bg-orange-500" />
                    {{ t("traffic.upload") }} {{ formatBps(hoveredSample.upBps) }}
                  </div>
                </div>
              </template>
            </div>
          </div>

          <div class="mt-1.5 flex items-center justify-between pl-16.5 text-[10px] text-gray-400">
            <span>{{ t("traffic.s60Ago") }}</span>
            <span>{{ t("traffic.s30Ago") }}</span>
            <span>{{ t("traffic.now") }}</span>
          </div>
        </div>
      </div>
    </div>

    <div class="antify-card space-y-4 p-5">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="flex items-center gap-3">
          <span class="text-[14px] font-semibold text-gray-900">
            {{ t("traffic.historyTitle") }}
          </span>
          <div class="inline-flex rounded-lg bg-[#EFEFF2] p-0.5 text-[12px]">
            <button
              type="button"
              class="rounded-md px-2.5 py-1 font-medium transition"
              :class="
                historyRange === '24h'
                  ? 'bg-[#0A7AFF] text-white shadow-2xs'
                  : 'text-gray-600 hover:text-gray-900'
              "
              @click="switchRange('24h')"
            >
              {{ t("traffic.range24h") }}
            </button>
            <button
              type="button"
              class="rounded-md px-2.5 py-1 font-medium transition"
              :class="
                historyRange === 'month'
                  ? 'bg-[#0A7AFF] text-white shadow-2xs'
                  : 'text-gray-600 hover:text-gray-900'
              "
              @click="switchRange('month')"
            >
              {{ t("traffic.rangeMonth") }}
            </button>
          </div>
        </div>

        <div class="flex items-center gap-2">
          <SelectMenu
            :model-value="historyProfileId"
            :options="historyProfileOptions"
            variant="pill"
            @update:model-value="changeHistoryProfile"
          />
          <button
            type="button"
            :title="t('traffic.clearHistory')"
            class="inline-flex items-center gap-1 rounded-lg border border-gray-200 bg-white px-2.5 py-1 text-[12px] text-gray-600 transition hover:bg-gray-50 hover:text-red-600"
            @click="store.clearTrafficHistory()"
          >
            <Trash2 class="h-3.5 w-3.5" />
            {{ t("traffic.clearHistory") }}
          </button>
        </div>
      </div>

      <div
        v-if="store.trafficHistory"
        class="flex flex-wrap items-center gap-6 text-[12px] text-gray-600"
      >
        <span>
          {{ t("traffic.historyProxiedTotal") }}
          <strong class="text-gray-900">
            {{
              formatBytes(
                store.trafficHistory.totalProxiedUp +
                  store.trafficHistory.totalProxiedDown,
              )
            }}
          </strong>
        </span>
        <span>
          {{ t("traffic.historyDirectTotal") }}
          <strong class="text-gray-900">
            {{
              formatBytes(
                store.trafficHistory.totalDirectUp +
                  store.trafficHistory.totalDirectDown,
              )
            }}
          </strong>
        </span>
      </div>

      <div
        v-if="historyBuckets.length > 0"
        class="relative rounded-xl border border-gray-100 bg-gray-50/70 px-3 pt-4 pb-2"
      >
        <div
          v-if="hoveredBucket"
          class="mb-2 flex flex-wrap items-center justify-between gap-2 rounded-lg bg-white px-3 py-1.5 text-[11.5px] shadow-2xs dark:bg-[#22252D]"
        >
          <span class="font-semibold text-gray-800">{{ hoveredBucket.label }}</span>
          <div class="flex items-center gap-4 font-mono text-gray-600">
            <span>
              {{ t("common.proxy") }}: ↑{{ formatBytes(hoveredBucket.proxiedUp) }} ↓{{
                formatBytes(hoveredBucket.proxiedDown)
              }}
            </span>
            <span>
              {{ t("common.direct") }}: ↑{{ formatBytes(hoveredBucket.directUp) }} ↓{{
                formatBytes(hoveredBucket.directDown)
              }}
            </span>
          </div>
        </div>

        <div class="flex h-28 items-end gap-1">
          <div
            v-for="(bucket, idx) in historyBuckets"
            :key="bucket.bucketStartMs"
            class="group flex h-full flex-1 flex-col items-center justify-end"
            @mouseenter="hoveredBarIndex = idx"
            @mouseleave="hoveredBarIndex = null"
          >
            <div class="flex w-full flex-1 flex-col justify-end gap-[1px]">
              <div
                class="w-full rounded-t-xs bg-[#0A7AFF]/85 transition-all group-hover:bg-[#0A7AFF]"
                :style="{
                  height: `${Math.max(
                    bucket.proxiedUp + bucket.proxiedDown > 0 ? 3 : 0,
                    ((bucket.proxiedUp + bucket.proxiedDown) / historyMaxTotal) *
                      100,
                  )}%`,
                }"
              />
              <div
                class="w-full rounded-b-xs bg-emerald-500/80 transition-all group-hover:bg-emerald-500"
                :style="{
                  height: `${Math.max(
                    bucket.directUp + bucket.directDown > 0 ? 2 : 0,
                    ((bucket.directUp + bucket.directDown) / historyMaxTotal) *
                      100,
                  )}%`,
                }"
              />
            </div>
          </div>
        </div>

        <div class="mt-1.5 flex items-center justify-between text-[10px] text-gray-400">
          <span>{{ historyBuckets[0]?.label }}</span>
          <span>{{ historyBuckets[Math.floor(historyBuckets.length / 2)]?.label }}</span>
          <span>{{ historyBuckets[historyBuckets.length - 1]?.label }}</span>
        </div>
      </div>

      <div
        v-else
        class="rounded-xl border border-dashed border-gray-200 py-8 text-center text-[12.5px] text-gray-400"
      >
        {{ t("traffic.historyEmpty") }}
      </div>
    </div>

    <div class="antify-card overflow-hidden p-4">
      <div class="mb-3 flex items-center justify-between gap-3">
        <div class="inline-flex rounded-lg bg-[#EFEFF2] p-0.5 text-[12.5px]">
          <button
            v-for="tab in [
              { id: 'rule', label: t('traffic.tabRule') },
              { id: 'proxy', label: t('traffic.tabProxy') },
              { id: 'app', label: t('traffic.tabApp') },
              { id: 'host', label: t('traffic.tabHost') },
            ]"
            :key="tab.id"
            type="button"
            class="rounded-[7px] px-3 py-1 font-medium transition"
            :class="
              breakdownTab === tab.id
                ? 'bg-[#0A7AFF] text-white shadow-2xs'
                : 'text-gray-700 hover:text-gray-900'
            "
            @click="breakdownTab = tab.id as typeof breakdownTab"
          >
            {{ tab.label }}
          </button>
        </div>

        <span class="text-[11.5px] text-gray-400">
          {{ t("traffic.sessionScope") }}
        </span>
      </div>

      <div
        class="grid grid-cols-[1.8fr_0.7fr_0.7fr_0.5fr] items-center gap-2 border-b border-gray-100 px-3 pb-2 text-[11.5px] font-medium text-gray-400"
      >
        <span>{{ t("traffic.colName") }}</span>
        <span class="text-right">{{ t("traffic.colUp") }}</span>
        <span class="text-right">{{ t("traffic.colDown") }}</span>
        <span class="text-right">{{ t("traffic.colConns") }}</span>
      </div>

      <div class="divide-y divide-gray-100/80">
        <div
          v-for="row in breakdownRows"
          :key="row.key"
          class="grid grid-cols-[1.8fr_0.7fr_0.7fr_0.5fr] items-center gap-2 px-3 py-2 text-[12.5px]"
        >
          <span class="truncate font-medium text-gray-900">{{ row.label }}</span>
          <span class="text-right font-mono text-gray-600">
            {{ formatBytes(row.upBytes) }}
          </span>
          <span class="text-right font-mono text-gray-600">
            {{ formatBytes(row.downBytes) }}
          </span>
          <span class="text-right font-mono text-gray-500">
            {{ row.connections }}
          </span>
        </div>

        <div
          v-if="breakdownRows.length === 0"
          class="py-8 text-center text-[12.5px] text-gray-400"
        >
          {{ t("traffic.breakdownEmpty") }}
        </div>
      </div>
    </div>
  </div>
</template>
