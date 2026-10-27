import { invoke } from "@tauri-apps/api/core";

export type Team = { id: string; name: string };
export type ModelOption = {
  id: string;
  label: string;
};
export type SessionView = {
  id: string;
  team_id: string;
  name: string;
  workspace_path: string;
  provider: string;
  cli_session_id: string | null;
  seat: string;
  run_id: string | null;
  streaming_text?: string | null;
  pending_tool?: string | null;
  queued_count?: number;
  last_error?: string | null;
  avatar?: string;
  model?: string;
  role_description?: string;
  created_at?: number;
};
export type Notification = {
  id: string;
  session_id: string;
  run_id: string | null;
  kind: string;
  priority: number;
  title: string;
  unread: boolean;
  dedupe_key: string;
  created_at: number;
};
export type ProviderStatus = {
  name: string;
  available: boolean;
  detail: string;
  adapter_package: string | null;
  models?: ModelOption[];
  default_model?: string | null;
};
export type Snapshot = {
  teams: Team[];
  sessions: SessionView[];
  notifications: Notification[];
  dispatch_paused: boolean;
  provider: string;
  providers: string[];
  probes: ProviderStatus[];
};
export type Attachment = {
  id: string;
  original_name: string;
  mime: string;
  size: number;
  stored_path: string;
};
export type Message = {
  id: string;
  session_id: string;
  run_id: string | null;
  role: string;
  content: string;
  request_id: string | null;
  created_at: number;
  attachments?: Attachment[];
};

export type ProfilePatch = {
  name: string;
  provider: string;
  model: string;
  roleDescription: string;
  avatar: string;
};

export function isDeskDemo(): boolean {
  return (
    typeof window !== "undefined" &&
    new URLSearchParams(window.location.search).has("demo")
  );
}

function demoSnapshot(): Snapshot {
  return {
    teams: [{ id: "t1", name: "实验室" }],
    sessions: [
      {
        id: "s-ada",
        team_id: "t1",
        name: "Ada",
        workspace_path: "~/desk/ada",
        provider: "cursor",
        cli_session_id: "ada-1",
        seat: "working",
        run_id: "r-ada",
        avatar: "maru",
        model: "default[]",
        role_description: "核心架构与后端开发",
        pending_tool: "read",
      },
      {
        id: "s-ben",
        team_id: "t1",
        name: "Ben",
        workspace_path: "~/desk/ben",
        provider: "claude",
        cli_session_id: "ben-1",
        seat: "awaiting_approval",
        run_id: "r-ben",
        avatar: "sebastian",
        model: "sonnet",
        role_description: "代码审查与安全审计",
      },
      {
        id: "s-cara",
        team_id: "t1",
        name: "Cara",
        workspace_path: "~/desk/cara",
        provider: "antigravity",
        cli_session_id: null,
        seat: "idle",
        run_id: null,
        avatar: "robin",
        model: "gemini-3.8-flash",
        role_description: "前端界面与交互设计",
      },
    ],
    notifications: [
      {
        id: "n1",
        session_id: "s-ben",
        run_id: "r-ben",
        kind: "approval",
        priority: 0,
        title: "待审批: write",
        unread: true,
        dedupe_key: "run:r-ben:approval",
        created_at: 1,
      },
    ],
    dispatch_paused: false,
    provider: "cursor",
    providers: ["cursor", "claude", "antigravity"],
    probes: [
      {
        name: "cursor",
        available: true,
        detail: "agent acp",
        adapter_package: null,
      },
      {
        name: "claude",
        available: true,
        detail: "claude-agent-acp",
        adapter_package: null,
      },
      {
        name: "antigravity",
        available: true,
        detail: "agy_acp_server",
        adapter_package: null,
      },
    ],
  };
}

const DEMO_MESSAGES: Record<string, Message[]> = {
  "s-ada": [
    {
      id: "m1",
      session_id: "s-ada",
      run_id: null,
      role: "user",
      content: "登录会间歇性 401，先看鉴权中间件。",
      request_id: null,
      created_at: 1,
    },
    {
      id: "m2",
      session_id: "s-ada",
      run_id: "r-ada",
      role: "assistant",
      content:
        "正在读 `auth.ts`。token 刷新和 ACP session 过期是两回事：后者会让工位变成「下班」，点「恢复上班」即可开新会话。",
      request_id: null,
      created_at: 2,
    },
  ],
};

export const api = {
  snapshot: () =>
    isDeskDemo() ? Promise.resolve(demoSnapshot()) : invoke<Snapshot>("snapshot"),
  pump: () =>
    isDeskDemo() ? Promise.resolve(demoSnapshot()) : invoke<Snapshot>("pump"),
  listMessages: (
    sessionId: string,
    opts?: { limit?: number; beforeCreatedAt?: number; beforeId?: string },
  ) =>
    isDeskDemo()
      ? Promise.resolve(DEMO_MESSAGES[sessionId] ?? [])
      : invoke<Message[]>("list_messages", {
          sessionId,
          limit: opts?.limit ?? null,
          beforeCreatedAt: opts?.beforeCreatedAt ?? null,
          beforeId: opts?.beforeId ?? null,
        }),
  createTeam: (name: string) => invoke<Team>("create_team", { name }),
  createSession: (
    teamId: string,
    name: string,
    workspace: string,
    provider?: string,
    profile?: { model?: string; roleDescription?: string; avatar?: string },
  ) =>
    profile
      ? invoke<SessionView>("create_session", {
          teamId,
          name,
          workspace,
          provider: provider ?? null,
          model: profile.model ?? null,
          roleDescription: profile.roleDescription ?? null,
          avatar: profile.avatar ?? null,
        })
      : invoke<SessionView>("create_session", {
          teamId,
          name,
          workspace,
          provider: provider ?? null,
        }),
  submitMessage: (sessionId: string, text: string, attachmentIds: string[]) =>
    invoke<string>("submit_message", { sessionId, text, attachmentIds }),
  cancelRun: (runId: string) => invoke<string>("cancel_run", { runId }),
  stopAndReplace: (sessionId: string, text: string, attachmentIds?: string[]) =>
    attachmentIds && attachmentIds.length > 0
      ? invoke<string>("stop_and_replace", { sessionId, text, attachmentIds })
      : invoke<string>("stop_and_replace", { sessionId, text }),
  resolveApproval: (runId: string, allow: boolean) =>
    invoke<void>("resolve_approval", { runId, allow }),
  stageAttachment: (path: string) =>
    invoke<Attachment>("stage_attachment", { path }),
  stageBytes: (name: string, mime: string, bytes: number[]) =>
    invoke<Attachment>("stage_bytes", { name, mime, bytes }),
  readImageDataUrl: (path: string) =>
    invoke<string>("read_image_data_url", { path }),
  markRead: (id: string) => invoke<void>("mark_read", { id }),
  markAllRead: () => invoke<void>("mark_all_read"),
  resumeQueue: () => invoke<void>("resume_queue"),
  probe: () => invoke<ProviderStatus[]>("probe"),
  installAdapter: (name: string) => invoke<ProviderStatus[]>("install_adapter", { name }),
  clockIn: (sessionId: string) => invoke<SessionView>("clock_in", { sessionId }),
  setSessionProvider: (sessionId: string, provider: string) =>
    invoke<SessionView>("set_session_provider", { sessionId, provider }),
  updateSessionProfile: (sessionId: string, patch: ProfilePatch) =>
    invoke<SessionView>("update_session_profile", {
      sessionId,
      name: patch.name,
      provider: patch.provider,
      model: patch.model,
      roleDescription: patch.roleDescription,
      avatar: patch.avatar,
    }),
  deleteSession: (sessionId: string) =>
    invoke<void>("delete_session", { sessionId }),
};
