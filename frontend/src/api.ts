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

export const api = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  pump: () => invoke<Snapshot>("pump"),
  listMessages: (
    sessionId: string,
    opts?: { limit?: number; beforeCreatedAt?: number; beforeId?: string },
  ) =>
    invoke<Message[]>("list_messages", {
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
