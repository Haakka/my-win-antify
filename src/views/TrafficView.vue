<script setup lang="ts">
import { computed, ref } from "vue";
import { ArrowDown, ArrowUp } from "lucide-vue-next";
import { useAppStore } from "@/stores/app";
import { formatBps, formatBytes } from "@/utils/format";
import HelpTooltip from "@/components/HelpTooltip.vue";

const store = useAppStore();
const breakdownTab = ref<"rule" | "proxy" | "app" | "host">("rule");

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
      const t = 3 / Math.sqrt(s);
      tangents[i] = t * a * d;
      tangents[i + 1] = t * b * d;
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
    timeLabel: secondsAgo === 0 ? "现在" : `${secondsAgo} 秒前`,
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
      label = conn.ruleName ?? "默认";
    } else if (breakdownTab.value === "proxy") {
      if (conn.action === "proxy") {
        key = conn.proxyId ?? "__proxy__";
        label = conn.proxyName ?? "代理";
      } else if (conn.action === "block") {
        key = "__block__";
        label = "已阻断";
      } else {
        key = "__direct__";
        label = "直连";
      }
    } else if (breakdownTab.value === "app") {
      key = conn.executablePath || conn.processName || conn.appName;
      label = conn.appName || conn.processName || "未知应用";
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
        流量
      </h1>
      <div class="flex items-center gap-1.5 text-[12px] text-gray-400">
        <span>只统计经 MyWinAntify 处理的连接。</span>
        <HelpTooltip
          text="仅统计经由 MyWinAntify 流量引擎处理（命中规则或代理路由）的连接流量；未启动引擎或系统绕过流量不计入此处。"
        />
      </div>
    </div>

    <!-- Summary & 60s Live Chart Card -->
    <div class="antify-card space-y-5 p-5">
      <div class="flex flex-wrap gap-8">
        <div>
          <div class="text-[11.5px] text-gray-500">经代理</div>
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
          <div class="text-[11.5px] text-gray-500">直连</div>
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
          <span class="font-semibold text-gray-800">最近 60 秒</span>
          <div class="flex items-center gap-4 text-gray-600">
            <span class="inline-flex items-center gap-1.5">
              <span class="h-2 w-2 rounded-full bg-blue-500" />
              下载 {{ formatBps(store.stats.downloadBps) }}
            </span>
            <span class="inline-flex items-center gap-1.5">
              <span class="h-2 w-2 rounded-full bg-orange-500" />
              上传 {{ formatBps(store.stats.uploadBps) }}
            </span>
          </div>
        </div>

        <div
          class="rounded-xl border border-gray-100 bg-gray-50/70 px-3 pt-3 pb-2"
        >
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
                  <linearGradient
                    id="trafficDownAreaGradient"
                    x1="0"
                    y1="0"
                    x2="0"
                    y2="1"
                  >
                    <stop offset="0%" stop-color="#3B82F6" stop-opacity="0.22" />
                    <stop offset="100%" stop-color="#3B82F6" stop-opacity="0.01" />
                  </linearGradient>
                  <linearGradient
                    id="trafficUpAreaGradient"
                    x1="0"
                    y1="0"
                    x2="0"
                    y2="1"
                  >
                    <stop offset="0%" stop-color="#F97316" stop-opacity="0.18" />
                    <stop offset="100%" stop-color="#F97316" stop-opacity="0.01" />
                  </linearGradient>
                </defs>

                <!-- Horizontal dashed gridlines -->
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

                <!-- Area fills -->
                <path :d="downAreaPath" fill="url(#trafficDownAreaGradient)" />
                <path :d="upAreaPath" fill="url(#trafficUpAreaGradient)" />

                <!-- Continuous smooth lines -->
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

              <!-- Hover hairline, markers, and floating tooltip -->
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
                    下载 {{ formatBps(hoveredSample.downBps) }}
                  </div>
                  <div class="flex items-center gap-1.5 font-mono text-gray-800">
                    <span class="h-1.5 w-1.5 rounded-full bg-orange-500" />
                    上传 {{ formatBps(hoveredSample.upBps) }}
                  </div>
                </div>
              </template>
            </div>
          </div>

          <div
            class="mt-1.5 flex items-center justify-between pl-16.5 text-[10px] text-gray-400"
          >
            <span>60 秒前</span>
            <span>30 秒前</span>
            <span>现在</span>
          </div>
        </div>
      </div>
    </div>

    <!-- Breakdown Table Card (Rule / Proxy / App / Host) -->
    <div class="antify-card overflow-hidden p-4">
      <div class="mb-3 flex items-center justify-between gap-3">
        <div class="inline-flex rounded-lg bg-[#EFEFF2] p-0.5 text-[12.5px]">
          <button
            v-for="tab in [
              { id: 'rule', label: '规则' },
              { id: 'proxy', label: '代理' },
              { id: 'app', label: '应用' },
              { id: 'host', label: '主机' },
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
          基于当前会话缓存连接统计
        </span>
      </div>

      <div
        class="grid grid-cols-[1.8fr_0.7fr_0.7fr_0.5fr] items-center gap-2 border-b border-gray-100 px-3 pb-2 text-[11.5px] font-medium text-gray-400"
      >
        <span>名称</span>
        <span class="text-right">上传</span>
        <span class="text-right">下载</span>
        <span class="text-right">连接</span>
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
          暂无会话流量明细
        </div>
      </div>
    </div>
  </div>
</template>
