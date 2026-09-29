import type { AppId } from "@/lib/api/types";
import type {
  McpServer,
  Provider,
  SessionMessage,
  SessionMeta,
  Settings,
} from "@/types";
import { deepClone } from "@/utils/deepClone";

type ProvidersByApp = Record<AppId, Record<string, Provider>>;
type CurrentProviderState = Record<AppId, string>;
type McpConfigState = Record<AppId, Record<string, McpServer>>;
type LiveProviderIdsByApp = Record<
  "opencode" | "openclaw" | "hermes",
  string[]
>;

const createDefaultProviders = (): ProvidersByApp => ({
  claude: {
    "claude-1": {
      id: "claude-1",
      name: "Claude Default",
      settingsConfig: {},
      category: "official",
      sortIndex: 0,
      createdAt: Date.now(),
    },
    "claude-2": {
      id: "claude-2",
      name: "Claude Custom",
      settingsConfig: {},
      category: "custom",
      sortIndex: 1,
      createdAt: Date.now() + 1,
    },
  },
  "claude-desktop": {},
  codex: {
    "codex-1": {
      id: "codex-1",
      name: "Codex Default",
      settingsConfig: {},
      category: "official",
      sortIndex: 0,
      createdAt: Date.now(),
    },
    "codex-2": {
      id: "codex-2",
      name: "Codex Secondary",
      settingsConfig: {},
      category: "custom",
      sortIndex: 1,
      createdAt: Date.now() + 1,
    },
  },
  gemini: {
    "gemini-1": {
      id: "gemini-1",
      name: "Gemini Default",
      settingsConfig: {
        env: {
          GEMINI_API_KEY: "test-key",
          GOOGLE_GEMINI_BASE_URL: "https://generativelanguage.googleapis.com",
        },
      },
      category: "official",
      sortIndex: 0,
      createdAt: Date.now(),
    },
  },
  grokbuild: {},
  opencode: {},
  openclaw: {},
  hermes: {},
});

const createDefaultCurrent = (): CurrentProviderState => ({
  claude: "claude-1",
  "claude-desktop": "",
  codex: "codex-1",
  gemini: "gemini-1",
  grokbuild: "",
  opencode: "",
  openclaw: "",
  hermes: "",
});

let providers = createDefaultProviders();
let current = createDefaultCurrent();
let liveProviderIds: LiveProviderIdsByApp = {
  opencode: [],
  openclaw: [],
  hermes: [],
};
let settingsState: Settings = {
  showInTray: true,
  minimizeToTrayOnClose: true,
  enableClaudePluginIntegration: false,
  claudeConfigDir: "/default/claude",
  codexConfigDir: "/default/codex",
  language: "zh",
};
let appConfigDirOverride: string | null = null;
const sessionMessageKey = (providerId: string, sourcePath: string) =>
  `${providerId}:${sourcePath}`;

const createDefaultSessions = (): SessionMeta[] => {
  const now = Date.now();
  return [
    {
      providerId: "codex",
      sessionId: "codex-session-1",
      title: "Codex Session One",
      summary: "Codex summary",
      projectDir: "/mock/codex",
      createdAt: now - 2000,
      lastActiveAt: now - 1000,
      sourcePath: "/mock/codex/session-1.jsonl",
      resumeCommand: "codex resume codex-session-1",
    },
    {
      providerId: "claude",
      sessionId: "claude-session-1",
      title: "Claude Session One",
      summary: "Claude summary",
      projectDir: "/mock/claude",
      createdAt: now - 4000,
      lastActiveAt: now - 3000,
      sourcePath: "/mock/claude/session-1.jsonl",
      resumeCommand: "claude --resume claude-session-1",
    },
  ];
};

const createDefaultSessionMessages = (): Record<string, SessionMessage[]> => ({
  [sessionMessageKey("codex", "/mock/codex/session-1.jsonl")]: [
    {
      role: "user",
      content: "First codex message",
      ts: Date.now() - 1000,
    },
  ],
  [sessionMessageKey("claude", "/mock/claude/session-1.jsonl")]: [
    {
      role: "user",
      content: "First claude message",
      ts: Date.now() - 3000,
    },
  ],
});

let sessionsState = createDefaultSessions();
let sessionMessagesState = createDefaultSessionMessages();
let mcpConfigs: McpConfigState = {
  claude: {
    sample: {
      id: "sample",
      name: "Sample Claude Server",
      enabled: true,
      apps: {
        claude: true,
        codex: false,
        gemini: false,
        opencode: false,
        openclaw: false,
        hermes: false,
      },
      server: {
        type: "stdio",
        command: "claude-server",
      },
    },
  },
  "claude-desktop": {},
  codex: {
    httpServer: {
      id: "httpServer",
      name: "HTTP Codex Server",
      enabled: false,
      apps: {
        claude: false,
        codex: true,
        gemini: false,
        opencode: false,
        openclaw: false,
        hermes: false,
      },
      server: {
        type: "http",
        url: "http://localhost:3000",
      },
    },
  },
  gemini: {},
  grokbuild: {},
  opencode: {},
  openclaw: {},
  hermes: {},
};

const cloneProviders = (value: ProvidersByApp) =>
  deepClone(value) as ProvidersByApp;

// ── fork identity + update pin ──────────────────────────────────────────────
// Mirrors the backend contract, including the derived fields: `pinned` and
// `refusalCode` follow from `updateMode`, and `officialUpdaterRegistered` is an
// observation the backend only reports true after a restart.
export type ForkInfoFixture = {
  isFork: boolean;
  name: string;
  bundleVersion: string;
  forkVersion: string;
  baseVersion: string;
  baseCommit: string;
  commit: string;
  serial: number;
  upstreamRepo: string;
  upstreamUrl: string;
  updateMode: "pinned" | "official";
  acknowledgedAt: string | null;
  pinned: boolean;
  officialUpdaterRegistered: boolean;
  refusalCode: string | null;
};

export type UpstreamReleaseFixture = {
  tag: string;
  version: string;
  publishedAt: string | null;
  htmlUrl: string | null;
  notes: string | null;
  isNewer: boolean;
};

const createDefaultForkInfo = (): ForkInfoFixture => ({
  isFork: true,
  name: "CC Switch Fork",
  bundleVersion: "3.19.1",
  forkVersion: "3.19.1+fork.1.03cbb56",
  baseVersion: "3.19.1",
  baseCommit: "2852962",
  commit: "03cbb56",
  serial: 1,
  upstreamRepo: "farion1231/cc-switch",
  upstreamUrl: "https://github.com/farion1231/cc-switch",
  updateMode: "pinned",
  acknowledgedAt: null,
  pinned: true,
  officialUpdaterRegistered: false,
  refusalCode: "FORK_UPDATER_PINNED",
});

let forkInfoState = createDefaultForkInfo();
let upstreamReleaseState: UpstreamReleaseFixture | null = null;
let lastForkPolicyCall: { mode: string; acknowledge: boolean } | null = null;

export const getForkInfo = () => deepClone(forkInfoState) as ForkInfoFixture;

export const setForkInfo = (patch: Partial<ForkInfoFixture>) => {
  forkInfoState = { ...forkInfoState, ...patch };
  const pinned = forkInfoState.updateMode === "pinned";
  forkInfoState.pinned = pinned;
  forkInfoState.refusalCode = pinned ? "FORK_UPDATER_PINNED" : null;
};

export const setUpstreamRelease = (release: UpstreamReleaseFixture | null) => {
  upstreamReleaseState =
    release === null ? null : (deepClone(release) as UpstreamReleaseFixture);
};

export const getUpstreamRelease = () =>
  upstreamReleaseState === null
    ? null
    : (deepClone(upstreamReleaseState) as UpstreamReleaseFixture);

export const setLastForkPolicyCall = (
  call: { mode: string; acknowledge: boolean } | null,
) => {
  lastForkPolicyCall = call;
};

export const getLastForkPolicyCall = () => lastForkPolicyCall;

export const resetProviderState = () => {
  providers = createDefaultProviders();
  current = createDefaultCurrent();
  forkInfoState = createDefaultForkInfo();
  upstreamReleaseState = null;
  lastForkPolicyCall = null;
  liveProviderIds = {
    opencode: [],
    openclaw: [],
    hermes: [],
  };
  sessionsState = createDefaultSessions();
  sessionMessagesState = createDefaultSessionMessages();
  settingsState = {
    showInTray: true,
    minimizeToTrayOnClose: true,
    enableClaudePluginIntegration: false,
    claudeConfigDir: "/default/claude",
    codexConfigDir: "/default/codex",
    language: "zh",
  };
  appConfigDirOverride = null;
  mcpConfigs = {
    claude: {
      sample: {
        id: "sample",
        name: "Sample Claude Server",
        enabled: true,
        apps: {
          claude: true,
          codex: false,
          gemini: false,
          opencode: false,
          openclaw: false,
          hermes: false,
        },
        server: {
          type: "stdio",
          command: "claude-server",
        },
      },
    },
    "claude-desktop": {},
    codex: {
      httpServer: {
        id: "httpServer",
        name: "HTTP Codex Server",
        enabled: false,
        apps: {
          claude: false,
          codex: true,
          gemini: false,
          opencode: false,
          openclaw: false,
          hermes: false,
        },
        server: {
          type: "http",
          url: "http://localhost:3000",
        },
      },
    },
    gemini: {},
    grokbuild: {},
    opencode: {},
    openclaw: {},
    hermes: {},
  };
};

export const getProviders = (appType: AppId) =>
  cloneProviders(providers)[appType] ?? {};

export const getCurrentProviderId = (appType: AppId) => current[appType] ?? "";

export const getLiveProviderIds = (
  appType: "opencode" | "openclaw" | "hermes",
) => [...liveProviderIds[appType]];

export const setLiveProviderIds = (
  appType: "opencode" | "openclaw" | "hermes",
  ids: string[],
) => {
  liveProviderIds[appType] = [...ids];
};

export const setCurrentProviderId = (appType: AppId, providerId: string) => {
  current[appType] = providerId;
};

export const updateProviders = (
  appType: AppId,
  data: Record<string, Provider>,
) => {
  providers[appType] = cloneProviders({ [appType]: data } as ProvidersByApp)[
    appType
  ];
};

export const setProviders = (
  appType: AppId,
  data: Record<string, Provider>,
) => {
  providers[appType] = deepClone(data) as Record<string, Provider>;
};

export const addProvider = (appType: AppId, provider: Provider) => {
  providers[appType] = providers[appType] ?? {};
  providers[appType][provider.id] = provider;
};

export const updateProvider = (appType: AppId, provider: Provider) => {
  if (!providers[appType]) return;
  providers[appType][provider.id] = {
    ...providers[appType][provider.id],
    ...provider,
  };
};

export const deleteProvider = (appType: AppId, providerId: string) => {
  if (!providers[appType]) return;
  delete providers[appType][providerId];
  if (current[appType] === providerId) {
    const fallback = Object.keys(providers[appType])[0] ?? "";
    current[appType] = fallback;
  }
};

export const updateSortOrder = (
  appType: AppId,
  updates: { id: string; sortIndex: number }[],
) => {
  if (!providers[appType]) return;
  updates.forEach(({ id, sortIndex }) => {
    const provider = providers[appType][id];
    if (provider) {
      providers[appType][id] = { ...provider, sortIndex };
    }
  });
};

export const listProviders = (appType: AppId) =>
  deepClone(providers[appType] ?? {}) as Record<string, Provider>;

export const getSettings = () => deepClone(settingsState) as Settings;

export const setSettings = (data: Partial<Settings>) => {
  settingsState = { ...settingsState, ...data };
};

export const getAppConfigDirOverride = () => appConfigDirOverride;

export const setAppConfigDirOverrideState = (value: string | null) => {
  appConfigDirOverride = value;
};

export const getMcpConfig = (appType: AppId) => {
  const servers = deepClone(mcpConfigs[appType] ?? {}) as Record<
    string,
    McpServer
  >;
  return {
    configPath: `/mock/${appType}.mcp.json`,
    servers,
  };
};

export const setMcpConfig = (
  appType: AppId,
  value: Record<string, McpServer>,
) => {
  mcpConfigs[appType] = deepClone(value) as Record<string, McpServer>;
};

export const setMcpServerEnabled = (
  appType: AppId,
  id: string,
  enabled: boolean,
) => {
  if (!mcpConfigs[appType]?.[id]) return;
  mcpConfigs[appType][id] = {
    ...mcpConfigs[appType][id],
    enabled,
  };
};

export const upsertMcpServer = (
  appType: AppId,
  id: string,
  server: McpServer,
) => {
  if (!mcpConfigs[appType]) {
    mcpConfigs[appType] = {};
  }
  mcpConfigs[appType][id] = deepClone(server) as McpServer;
};

export const deleteMcpServer = (appType: AppId, id: string) => {
  if (!mcpConfigs[appType]) return;
  delete mcpConfigs[appType][id];
};

export const listSessions = () => deepClone(sessionsState) as SessionMeta[];

export const getSessionMessages = (providerId: string, sourcePath: string) =>
  deepClone(
    sessionMessagesState[sessionMessageKey(providerId, sourcePath)] ?? [],
  ) as SessionMessage[];

export const deleteSession = (
  providerId: string,
  sessionId: string,
  sourcePath: string,
) => {
  sessionsState = sessionsState.filter(
    (session) =>
      !(
        session.providerId === providerId &&
        session.sessionId === sessionId &&
        session.sourcePath === sourcePath
      ),
  );
  delete sessionMessagesState[sessionMessageKey(providerId, sourcePath)];
  return true;
};

export const setSessionFixtures = (
  sessions: SessionMeta[],
  messages: Record<string, SessionMessage[]>,
) => {
  sessionsState = deepClone(sessions) as SessionMeta[];
  sessionMessagesState = deepClone(messages) as Record<
    string,
    SessionMessage[]
  >;
};
