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
  type AutomationConfig,
  type CoexistenceConflict,
  type ConfigBackupExport,
  type ConnectionSnapshot,
  type CurrentWifiInfo,
  type DiagnosticBundleSummary,
  type GeneralSettings,
  type NavRouteId,
  type NetworkInterfaceInfo,
  type PipeConnectionState,
  type Profile,
  type ProxyConfig,
  type ProxyHealthSnapshot,
  type ProxyTestResult,
  type Rule,
  type RulesImportPreview,
  type RunningProcessInfo,
  type ServiceStatus,
  type TrafficHistoryRange,
  type TrafficHistoryResponse,
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
  automation: {
    enabled: false,
    wifiRules: [],
  },
};

const DEFAULT_STATUS: ServiceStatus = {
  running: false,
  startedAtMs: null,
  uptimeSecs: 0,
  servicePid: 0,
  coreVersion: "0.2.0",
  driverReady: false,
  activeProfileId: "default",
  lastError: null,
  configError: null,
  invalidRuleIds: [],
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
  proxyHealth: [],
};

export const useAppStore = defineStore("app", () => {
  const isTauriEnv = ref<boolean>(isTauriRuntime());
  const mockMode = ref<boolean>(!isTauriRuntime());
  const activeRoute = ref<NavRouteId>("dashboard");
  const pipeState = ref<PipeConnectionState>("disconnected");
  const documentHidden = ref<boolean>(
    typeof document !== "undefined" ? document.hidden : false,
  );
  const discoverRemainingSecs = ref<number | null>(null);
  const discoverAutoOffNotice = ref(false);
  let discoverTimer: number | null = null;

  const status = ref<ServiceStatus>(structuredClone(DEFAULT_STATUS));
  const stats = ref<TrafficStats>(structuredClone(DEFAULT_STATS));
  const config = ref<AppConfig>(structuredClone(DEFAULT_EMPTY_CONFIG));
  const connections = ref<ConnectionSnapshot[]>([]);
  const activityLogs = ref<ActivityLogEntry[]>([]);
  const runningProcesses = ref<RunningProcessInfo[]>([]);
  const networkInterfaces = ref<NetworkInterfaceInfo[]>([]);
  const proxyTestResults = ref<Record<string, ProxyTestResult>>({});
  const proxyHealthMap = ref<Record<string, ProxyHealthSnapshot>>({});
  const ruleLastMatchedMap = ref<Record<string, number>>({});
  const speedHistory = ref<{ ts: number; upBps: number; downBps: number }[]>([]);
  const currentWifi = ref<CurrentWifiInfo>({
    supported: false,
    connected: false,
    ssid: null,
    interfaceName: null,
  });
  const coexistenceConflicts = ref<CoexistenceConflict[]>([]);
  const trafficHistory = ref<TrafficHistoryResponse | null>(null);
  const trafficHistoryRange = ref<TrafficHistoryRange>("24h");
  const trafficHistoryProfileId = ref<string | null>(null);

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
    if (documentHidden.value) {
      return;
    }
    for (const item of tick.ruleLastMatched) {
      ruleLastMatchedMap.value[item.ruleId] = item.lastMatchedAtMs;
    }
    const nextHealth: Record<string, ProxyHealthSnapshot> = {};
    for (const item of tick.proxyHealth ?? []) {
      nextHealth[item.proxyId] = item;
    }
    proxyHealthMap.value = nextHealth;
    speedHistory.value.push({
      ts: tick.timestampMs,
      upBps: tick.uploadBps,
      downBps: tick.downloadBps,
    });
    if (speedHistory.value.length > 60) {
      speedHistory.value.shift();
    }
  }

  function setDocumentHidden(hidden: boolean) {
    const wasHidden = documentHidden.value;
    documentHidden.value = hidden;
    if (wasHidden && !hidden && isServiceConnected.value) {
      void hydrateSnapshot();
    }
  }

  function syncDiscoverTimer(enabled: boolean) {
    if (!enabled) {
      if (discoverTimer !== null) {
        window.clearInterval(discoverTimer);
        discoverTimer = null;
      }
      discoverRemainingSecs.value = null;
      return;
    }
    if (discoverTimer !== null) {
      return;
    }
    discoverAutoOffNotice.value = false;
    discoverRemainingSecs.value = 600;
    discoverTimer = window.setInterval(() => {
      if (discoverRemainingSecs.value === null) return;
      if (discoverRemainingSecs.value <= 1) {
        syncDiscoverTimer(false);
        discoverAutoOffNotice.value = true;
        void updateSettings({ discoverUnmatched: false }).catch(() => {});
      } else {
        discoverRemainingSecs.value -= 1;
      }
    }, 1000);
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
        if (documentHidden.value || !config.value.settings.recordConnections) {
          return;
        }
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
        if (documentHidden.value) return;
        activityLogs.value.unshift(entry);
        if (activityLogs.value.length > 250) {
          activityLogs.value.pop();
        }
      },
      onConfigChanged(nextCfg) {
        config.value = nextCfg;
        syncDiscoverTimer(nextCfg.settings.discoverUnmatched);
      },
      onPipeState(nextPipe) {
        pipeState.value = nextPipe;
        if (nextPipe === "connected") {
          void hydrateSnapshot();
        }
      },
      onProxyTestsStale() {
        for (const id of Object.keys(proxyTestResults.value)) {
          const item = proxyTestResults.value[id];
          if (item) {
            proxyTestResults.value[id] = { ...item, stale: true };
          }
        }
      },
      onWifiChanged(wifi) {
        currentWifi.value = wifi;
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
      const [st, cfg, conns, logs, wifi, conflicts] = await Promise.all([
        client.getStatus(),
        client.getConfig(),
        client.getConnections(true, 150),
        client.getActivityLogs(120),
        client.getCurrentWifi().catch(() => ({
          supported: false,
          connected: false,
          ssid: null,
          interfaceName: null,
        })),
        client.getCoexistenceConflicts().catch(() => []),
      ]);
      status.value = st;
      config.value = cfg;
      syncDiscoverTimer(cfg.settings.discoverUnmatched);
      connections.value = conns;
      activityLogs.value = logs;
      currentWifi.value = wifi;
      coexistenceConflicts.value = conflicts.filter((c) => !c.dismissed);
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
      const saved = await client.saveConfig(next);
      config.value = saved;
      syncDiscoverTimer(saved.settings.discoverUnmatched);
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
      throw err;
    } finally {
      busySaving.value = false;
    }
  }

  async function restoreConfigBackup() {
    busySaving.value = true;
    errorMessage.value = null;
    try {
      config.value = await client.restoreConfigBackup();
      status.value = await client.getStatus();
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

  async function updateAutomation(automation: AutomationConfig) {
    const next = JSON.parse(JSON.stringify(config.value)) as AppConfig;
    next.automation = automation;
    await saveConfig(next);
  }

  async function exportConfigBackup(
    includePasswords: boolean,
  ): Promise<ConfigBackupExport> {
    return client.exportConfigBackup(includePasswords);
  }

  async function importConfigBackup(json: string): Promise<AppConfig> {
    busySaving.value = true;
    errorMessage.value = null;
    try {
      const saved = await client.importConfigBackup(json);
      config.value = saved;
      status.value = await client.getStatus();
      return saved;
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
      throw err;
    } finally {
      busySaving.value = false;
    }
  }

  async function exportRulesJson(profileId: string | null): Promise<string> {
    return client.exportRulesJson(profileId);
  }

  async function importRulesJson(
    json: string,
    profileId: string,
    dryRun: boolean,
  ): Promise<RulesImportPreview> {
    const res = await client.importRulesJson(json, profileId, dryRun);
    if (!dryRun) {
      config.value = await client.getConfig();
    }
    return res;
  }

  async function refreshTrafficHistory(
    range: TrafficHistoryRange = trafficHistoryRange.value,
    profileId: string | null = trafficHistoryProfileId.value,
  ) {
    trafficHistoryRange.value = range;
    trafficHistoryProfileId.value = profileId;
    if (!isServiceConnected.value) return;
    try {
      trafficHistory.value = await client.getTrafficHistory({
        range,
        profileId,
      });
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
    }
  }

  async function clearTrafficHistory() {
    try {
      await client.clearTrafficHistory();
      await refreshTrafficHistory();
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
    }
  }

  async function refreshCurrentWifi() {
    if (!isServiceConnected.value) return;
    try {
      currentWifi.value = await client.getCurrentWifi();
    } catch {
      // Keep previous Wi-Fi info on transient errors.
    }
  }

  async function refreshCoexistenceConflicts() {
    if (!isServiceConnected.value) return;
    try {
      const conflicts = await client.getCoexistenceConflicts();
      coexistenceConflicts.value = conflicts.filter((c) => !c.dismissed);
    } catch {
      // Ignore transient errors.
    }
  }

  async function dismissCoexistenceConflict(conflictId: string) {
    try {
      await client.dismissCoexistenceConflict(conflictId);
      coexistenceConflicts.value = coexistenceConflicts.value.filter(
        (c) => c.id !== conflictId,
      );
    } catch (err) {
      errorMessage.value = extractErrorMessage(err);
    }
  }

  async function collectDiagnosticBundle(
    outputPath: string | null = null,
  ): Promise<DiagnosticBundleSummary> {
    return client.collectDiagnosticBundle(outputPath);
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
        stale: false,
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
    documentHidden,
    discoverRemainingSecs,
    discoverAutoOffNotice,
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
    proxyHealthMap,
    ruleLastMatchedMap,
    speedHistory,
    currentWifi,
    coexistenceConflicts,
    trafficHistory,
    trafficHistoryRange,
    trafficHistoryProfileId,
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
    setDocumentHidden,
    setMockMode,
    reconnectService,
    toggleEngine,
    saveConfig,
    restoreConfigBackup,
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
    updateAutomation,
    exportConfigBackup,
    importConfigBackup,
    exportRulesJson,
    importRulesJson,
    refreshTrafficHistory,
    clearTrafficHistory,
    refreshCurrentWifi,
    refreshCoexistenceConflicts,
    dismissCoexistenceConflict,
    collectDiagnosticBundle,
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
