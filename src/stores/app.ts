import { computed, ref } from "vue";
import { defineStore } from "pinia";
import {
  createCoreClient,
  isTauriRuntime,
  type CoreClient,
} from "@/api/coreClient";
import {
  CONFIG_SCHEMA_VERSION,
  type ActivityLogEntry,
  type AppConfig,
  type ConnectionSnapshot,
  type GeneralSettings,
  type NavRouteId,
  type NetworkInterfaceInfo,
  type PipeConnectionState,
  type Profile,
  type ProxyConfig,
  type ProxyTestResult,
  type Rule,
  type RunningProcessInfo,
  type ServiceStatus,
  type TrafficStats,
} from "@/types";

const DEFAULT_EMPTY_CONFIG: AppConfig = {
  schemaVersion: CONFIG_SCHEMA_VERSION,
  activeProfileId: "default",
  profiles: [
    {
      id: "default",
      name: "General",
      defaultAction: "direct",
      defaultProxyId: null,
    },
  ],
  proxies: [],
  rules: [],
  settings: {
    theme: "light",
    language: "zh-CN",
    launchAtLogin: false,
    autoStartProxy: false,
    bypassLan: true,
    bypassHosts: [],
    recordConnections: true,
    discoverUnmatched: false,
  },
};

const DEFAULT_STATUS: ServiceStatus = {
  running: false,
  startedAtMs: null,
  uptimeSecs: 0,
  servicePid: 0,
  coreVersion: "0.1.0",
  driverReady: false,
  activeProfileId: "default",
  lastError: null,
};

const DEFAULT_STATS: TrafficStats = {
  timestampMs: Date.now(),
  running: false,
  uptimeSecs: 0,
  activeConnections: 0,
  activeApps: 0,
  uploadBps: 0,
  downloadBps: 0,
  totalProxiedUp: 0,
  totalProxiedDown: 0,
  totalDirectUp: 0,
  totalDirectDown: 0,
  ruleLastMatched: [],
};

export const useAppStore = defineStore("app", () => {
  const isTauriEnv = ref<boolean>(isTauriRuntime());
  const mockMode = ref<boolean>(!isTauriRuntime());
  const activeRoute = ref<NavRouteId>("dashboard");
  const pipeState = ref<PipeConnectionState>("disconnected");

  const status = ref<ServiceStatus>(structuredClone(DEFAULT_STATUS));
  const stats = ref<TrafficStats>(structuredClone(DEFAULT_STATS));
  const config = ref<AppConfig>(structuredClone(DEFAULT_EMPTY_CONFIG));
  const connections = ref<ConnectionSnapshot[]>([]);
  const activityLogs = ref<ActivityLogEntry[]>([]);
  const runningProcesses = ref<RunningProcessInfo[]>([]);
  const networkInterfaces = ref<NetworkInterfaceInfo[]>([]);
  const proxyTestResults = ref<Record<string, ProxyTestResult>>({});
  const ruleLastMatchedMap = ref<Record<string, number>>({});
  const speedHistory = ref<{ ts: number; upBps: number; downBps: number }[]>([]);

  const busyEngine = ref(false);
  const busySaving = ref(false);
  const testingProxyIds = ref<Record<string, boolean>>({});
  const errorMessage = ref<string | null>(null);

  // Global quick-add modal triggers (e.g., from Dashboard '+' cards)
  const quickAddProxyOpen = ref(false);
  const quickAddRuleOpen = ref(false);
  const quickAddRulePrefill = ref<{ name: string; path: string } | null>(null);
  const quickAddProfileOpen = ref(false);

  let client: CoreClient = createCoreClient(mockMode.value);
  let unsub: (() => void) | null = null;

  const activeProfile = computed<Profile>(() => {
    return (
      config.value.profiles.find(
        (p) => p.id === config.value.activeProfileId,
      ) ?? config.value.profiles[0] ?? DEFAULT_EMPTY_CONFIG.profiles[0]
    );
  });

  const activeProfileRules = computed<Rule[]>(() => {
    const pid = activeProfile.value.id;
    return config.value.rules
      .filter((r) => r.profileId === pid)
      .slice()
      .sort((a, b) => a.priority - b.priority);
  });

  const isServiceConnected = computed(
    () => mockMode.value || pipeState.value === "connected",
  );

  function applyStatsTick(tick: TrafficStats) {
    stats.value = tick;
    status.value.running = tick.running;
    status.value.uptimeSecs = tick.uptimeSecs;
    for (const item of tick.ruleLastMatched) {
      ruleLastMatchedMap.value[item.ruleId] = item.lastMatchedAtMs;
    }
    speedHistory.value.push({
      ts: tick.timestampMs,
      upBps: tick.uploadBps,
      downBps: tick.downloadBps,
    });
    if (speedHistory.value.length > 60) {
      speedHistory.value.shift();
    }
  }

  async function bindAndHydrate() {
    if (unsub) {
      unsub();
      unsub = null;
    }
    errorMessage.value = null;
    client = createCoreClient(mockMode.value);

    unsub = await client.subscribe({
      onStatusChanged(next) {
        status.value = next;
      },
      onStatsTick(tick) {
        applyStatsTick(tick);
      },
      onConnectionEvent(conn) {
        if (!config.value.settings.recordConnections) return;
        const idx = connections.value.findIndex((c) => c.id === conn.id);
        if (idx >= 0) {
          connections.value[idx] = conn;
        } else {
          connections.value.unshift(conn);
          if (connections.value.length > 250) {
            connections.value.pop();
          }
        }
      },
      onActivityLog(entry) {
        activityLogs.value.unshift(entry);
        if (activityLogs.value.length > 250) {
          activityLogs.value.pop();
        }
      },
      onConfigChanged(nextCfg) {
        config.value = nextCfg;
      },
      onPipeState(nextPipe) {
        pipeState.value = nextPipe;
        if (nextPipe === "connected") {
          void hydrateSnapshot();
        }
      },
    });

    await hydrateSnapshot();
  }

  async function hydrateSnapshot() {
    try {
      const currentPipe = await client.getPipeState();
      pipeState.value = currentPipe;
      if (!mockMode.value && currentPipe !== "connected") {
        return;
      }
      const [st, cfg, conns, logs] = await Promise.all([
        client.getStatus(),
        client.getConfig(),
        client.getConnections(true, 150),
        client.getActivityLogs(120),
      ]);
      status.value = st;
      config.value = cfg;
      connections.value = conns;
      activityLogs.value = logs;
      errorMessage.value = null;
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
    }
  }

  async function init() {
    await bindAndHydrate();
  }

  async function setMockMode(enabled: boolean) {
    mockMode.value = enabled;
    connections.value = [];
    activityLogs.value = [];
    speedHistory.value = [];
    await bindAndHydrate();
  }

  async function reconnectService() {
    if (mockMode.value) {
      mockMode.value = false;
    }
    if (isTauriRuntime() && pipeState.value !== "connected") {
      try {
        await client.launchService();
        await new Promise((resolve) => window.setTimeout(resolve, 400));
      } catch (err) {
        errorMessage.value = extractErrorMessage(err);
      }
    }
    await bindAndHydrate();
  }

  async function toggleEngine() {
    if (busyEngine.value) return;
    busyEngine.value = true;
    errorMessage.value = null;
    try {
      status.value = status.value.running
        ? await client.stopEngine()
        : await client.startEngine();
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
    } finally {
      busyEngine.value = false;
    }
  }

  async function saveConfig(next: AppConfig) {
    busySaving.value = true;
    errorMessage.value = null;
    try {
      config.value = await client.saveConfig(next);
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
      throw err;
    } finally {
      busySaving.value = false;
    }
  }

  async function upsertProxy(proxy: ProxyConfig) {
    const next = JSON.parse(JSON.stringify(config.value)) as AppConfig;
    const idx = next.proxies.findIndex((p) => p.id === proxy.id);
    if (idx >= 0) {
      next.proxies[idx] = proxy;
    } else {
      next.proxies.push(proxy);
    }
    await saveConfig(next);
  }

  async function deleteProxy(proxyId: string) {
    const next = JSON.parse(JSON.stringify(config.value)) as AppConfig;
    next.proxies = next.proxies.filter((p) => p.id !== proxyId);
    for (const r of next.rules) {
      if (r.proxyId === proxyId) {
        r.proxyId = next.proxies[0]?.id ?? null;
      }
    }
    for (const p of next.profiles) {
      if (p.defaultProxyId === proxyId) {
        p.defaultProxyId = next.proxies[0]?.id ?? null;
      }
    }
    await saveConfig(next);
  }

  async function upsertRule(rule: Rule) {
    const next = JSON.parse(JSON.stringify(config.value)) as AppConfig;
    const idx = next.rules.findIndex((r) => r.id === rule.id);
    if (idx >= 0) {
      next.rules[idx] = rule;
    } else {
      next.rules.push(rule);
    }
    await saveConfig(next);
  }

  async function deleteRule(ruleId: string) {
    const next = JSON.parse(JSON.stringify(config.value)) as AppConfig;
    next.rules = next.rules.filter((r) => r.id !== ruleId);
    await saveConfig(next);
  }

  async function toggleRule(ruleId: string, enabled: boolean) {
    const next = JSON.parse(JSON.stringify(config.value)) as AppConfig;
    const rule = next.rules.find((r) => r.id === ruleId);
    if (!rule) return;
    rule.enabled = enabled;
    await saveConfig(next);
  }

  async function toggleRuleNetworkLock(ruleId: string, networkLock: boolean) {
    const next = JSON.parse(JSON.stringify(config.value)) as AppConfig;
    const rule = next.rules.find((r) => r.id === ruleId);
    if (!rule) return;
    rule.networkLock = networkLock;
    await saveConfig(next);
  }

  async function upsertProfile(profile: Profile) {
    const next = JSON.parse(JSON.stringify(config.value)) as AppConfig;
    const idx = next.profiles.findIndex((p) => p.id === profile.id);
    if (idx >= 0) {
      next.profiles[idx] = profile;
    } else {
      next.profiles.push(profile);
    }
    await saveConfig(next);
  }

  async function deleteProfile(profileId: string) {
    if (config.value.profiles.length <= 1) return;
    const next = JSON.parse(JSON.stringify(config.value)) as AppConfig;
    next.profiles = next.profiles.filter((p) => p.id !== profileId);
    next.rules = next.rules.filter((r) => r.profileId !== profileId);
    if (next.activeProfileId === profileId) {
      next.activeProfileId = next.profiles[0].id;
    }
    await saveConfig(next);
  }

  async function setActiveProfile(profileId: string) {
    errorMessage.value = null;
    try {
      config.value = await client.setActiveProfile(profileId);
      status.value.activeProfileId = profileId;
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
    }
  }

  async function updateSettings(patch: Partial<GeneralSettings>) {
    const next = JSON.parse(JSON.stringify(config.value)) as AppConfig;
    next.settings = { ...next.settings, ...patch };
    await saveConfig(next);
  }

  async function testProxy(proxyId: string) {
    testingProxyIds.value[proxyId] = true;
    try {
      const res = await client.testProxy(proxyId);
      proxyTestResults.value[proxyId] = res;
    } catch (err) {
      proxyTestResults.value[proxyId] = {
        proxyId,
        reachable: false,
        latencyMs: null,
        error: extractErrorMessage(err),
        checkedAtMs: Date.now(),
      };
    } finally {
      testingProxyIds.value[proxyId] = false;
    }
  }

  async function testAllProxies() {
    await Promise.all(config.value.proxies.map((p) => testProxy(p.id)));
  }

  async function refreshProcesses() {
    try {
      runningProcesses.value = await client.listRunningProcesses();
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
    }
  }

  async function refreshNetworkInterfaces() {
    try {
      networkInterfaces.value = await client.listNetworkInterfaces();
    } catch {
      // Keep existing list on transient RPC failures; "自动" remains available.
    }
  }

  async function clearConnections() {
    try {
      await client.clearConnections();
      connections.value = [];
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
    }
  }

  async function refreshConnections() {
    if (!isServiceConnected.value) return;
    try {
      connections.value = await client.getConnections(true, 150);
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
    }
  }

  async function refreshActivityLogs() {
    if (!isServiceConnected.value) return;
    try {
      activityLogs.value = await client.getActivityLogs(120);
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
    }
  }

  async function clearActivityLogs() {
    try {
      await client.clearActivityLogs();
      activityLogs.value = [];
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
    }
  }

  function openQuickAddRule(prefill?: { name: string; path: string }) {
    quickAddRulePrefill.value = prefill ?? null;
    quickAddRuleOpen.value = true;
  }

  return {
    isTauriEnv,
    mockMode,
    activeRoute,
    pipeState,
    isServiceConnected,
    status,
    stats,
    config,
    activeProfile,
    activeProfileRules,
    connections,
    activityLogs,
    runningProcesses,
    networkInterfaces,
    proxyTestResults,
    ruleLastMatchedMap,
    speedHistory,
    busyEngine,
    busySaving,
    testingProxyIds,
    errorMessage,
    quickAddProxyOpen,
    quickAddRuleOpen,
    quickAddRulePrefill,
    quickAddProfileOpen,
    init,
    hydrateSnapshot,
    setMockMode,
    reconnectService,
    toggleEngine,
    saveConfig,
    upsertProxy,
    deleteProxy,
    upsertRule,
    deleteRule,
    toggleRule,
    toggleRuleNetworkLock,
    upsertProfile,
    deleteProfile,
    setActiveProfile,
    updateSettings,
    testProxy,
    testAllProxies,
    refreshProcesses,
    refreshNetworkInterfaces,
    refreshConnections,
    clearConnections,
    refreshActivityLogs,
    clearActivityLogs,
    openQuickAddRule,
  };
});

function extractErrorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object") {
    if ("message" in err && typeof err.message === "string") {
      return err.message;
    }
    return JSON.stringify(err);
  }
  return "Unknown IPC error";
}
