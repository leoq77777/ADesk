import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useRef, useState, type ClipboardEvent } from "react";
import {
  api,
  isDeskDemo,
  type Attachment,
  type Message,
  type ModelOption,
  type ProviderStatus,
  type SessionView,
  type Snapshot,
} from "./api";
import { LocalOrRemoteImage, MessageContent } from "./Markdown";

const PAGE = 80;
const CLI_LABEL: Record<string, string> = {
  fake: "假 CLI",
  cursor: "Cursor",
  gemini: "Gemini",
  codex: "GPT / Codex",
  claude: "Claude",
  antigravity: "Antigravity",
};
const DEFAULT_MODELS: Record<string, string> = {
  claude: "default",
  codex: "gpt-6.1-sol",
  gemini: "gemini-3.8-flash",
  antigravity: "gemini-3.8-flash",
  cursor: "default[]",
  fake: "fake-default",
};
const FALLBACK_MODEL_OPTIONS: Record<string, ModelOption[]> = {
  cursor: [
    { id: "default[]", label: "Auto (default[])" },
    { id: "claude-sonnet-4-6[thinking=true,context=200k,effort=medium]", label: "claude-sonnet-4-6" },
    { id: "gpt-5.4[context=272k,reasoning=medium,fast=false]", label: "gpt-5.4" },
  ],
  codex: [
    { id: "gpt-6.1-sol", label: "6.1 Sol (gpt-6.1-sol)" },
    { id: "gpt-6-astra", label: "6 Astra (gpt-6-astra)" },
    { id: "gpt-6-luna", label: "6 Luna (gpt-6-luna)" },
    { id: "gpt-5.6-sol", label: "5.6 Sol (gpt-5.6-sol)" },
  ],
  claude: [
    { id: "default", label: "Default (recommended)" },
    { id: "opus", label: "Opus" },
    { id: "sonnet", label: "Sonnet" },
    { id: "haiku", label: "Haiku" },
  ],
  antigravity: [
    { id: "gemini-3.8-flash", label: "Gemini 3.8 Flash" },
    { id: "gemini-4-argon", label: "Gemini 4 Argon" },
  ],
  gemini: [
    { id: "gemini-3.8-flash", label: "Gemini 3.8 Flash" },
  ],
  fake: [
    { id: "fake-default", label: "Fake Default (fake-default)" },
    { id: "fake-fast", label: "Fake Fast (fake-fast)" },
  ],
};
const ROLE_PRESETS = [
  "通用研发与代码协作",
  "核心架构与后端开发",
  "前端界面与交互设计",
  "自动化测试与故障排查",
  "代码审查与安全审计",
];
const SEAT_LABEL: Record<string, string> = {
  idle: "空闲",
  queued: "排队",
  working: "执行中",
  awaiting_approval: "待审批",
  awaiting_input: "等待输入",
  error: "出错",
  away: "下班",
};
const QUICK_PROMPTS = [
  "梳理当前目录结构并说明核心模块",
  "检查当前代码中的潜在问题",
  "运行测试并汇报结果",
];
const STARDEW_CAST = [
  { id: "sebastian", label: "Sebastian · 极客开发" },
  { id: "maru", label: "Maru · 机械与精密工程" },
  { id: "abigail", label: "Abigail · 探索与测试" },
  { id: "harvey", label: "Harvey · 诊断与修复" },
  { id: "elliott", label: "Elliott · 架构与文档" },
  { id: "penny", label: "Penny · 审校与引导" },
  { id: "clint", label: "Clint · 构建与工具链" },
  { id: "robin", label: "Robin · 基建与工程" },
] as const;

type StardewCast = (typeof STARDEW_CAST)[number]["id"];

function getModelsForProvider(
  provider: string,
  probes?: ProviderStatus[],
): { options: ModelOption[]; defaultModel: string } {
  const probe = probes?.find((p) => p.name === provider);
  if (probe?.models && probe.models.length > 0) {
    const def =
      probe.default_model &&
      probe.models.some((m) => m.id === probe.default_model)
        ? probe.default_model
        : probe.models[0].id;
    return { options: probe.models, defaultModel: def };
  }
  const fallback = FALLBACK_MODEL_OPTIONS[provider] ?? [
    { id: "default", label: "默认模型 (default)" },
  ];
  const def = DEFAULT_MODELS[provider] ?? fallback[0].id;
  return { options: fallback, defaultModel: def };
}

function formatModelDisplay(
  provider: string,
  modelId?: string,
  probes?: ProviderStatus[],
): string {
  const raw = modelId?.trim();
  const { options, defaultModel } = getModelsForProvider(provider, probes);
  const target = raw || defaultModel;
  const found = options.find((m) => m.id === target);
  if (found) return found.label;
  return target;
}

function pickCharacter(s: Pick<SessionView, "name" | "avatar">, index = 0): StardewCast {
  const explicit = s.avatar?.trim().toLowerCase();
  if (explicit && STARDEW_CAST.some((c) => c.id === explicit)) {
    return explicit as StardewCast;
  }
  const lower = s.name.toLowerCase();
  for (const c of STARDEW_CAST) {
    if (lower.includes(c.id)) return c.id;
  }
  let hash = index * 7;
  for (let i = 0; i < s.name.length; i++) {
    hash = (hash * 31 + s.name.charCodeAt(i)) >>> 0;
  }
  return STARDEW_CAST[hash % STARDEW_CAST.length].id;
}

function emoteRowClass(seat: string, isClocking: boolean): string {
  if (isClocking) return "emote-ellipsis";
  switch (seat) {
    case "working":
      return "emote-working";
    case "awaiting_approval":
      return "emote-alert";
    case "queued":
      return "emote-ellipsis";
    case "error":
      return "emote-angry";
    case "away":
      return "emote-sleep";
    default:
      return "emote-happy";
  }
}

type Jump = {
  session_id: string;
  notification_id: string;
  run_id: string | null;
};

type SeatFilter = "all" | "active" | "attention";

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatTime(ms: number): string {
  if (!ms || ms < 1_000_000_000) return "";
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

function formatCreatedAt(ms?: number): string {
  if (!ms || ms < 1_000_000_000) return "初始建档";
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return "初始建档";
  const yyyy = d.getFullYear();
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const mi = String(d.getMinutes()).padStart(2, "0");
  return `${yyyy}-${mm}-${dd} ${hh}:${mi}`;
}

export function App() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [teamId, setTeamId] = useState<string | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [hasOlder, setHasOlder] = useState(false);
  const [draft, setDraft] = useState("");
  const [files, setFiles] = useState<Attachment[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [dropOn, setDropOn] = useState(false);
  const [probing, setProbing] = useState(false);
  const [installing, setInstalling] = useState<string | null>(null);
  const [clockingIds, setClockingIds] = useState<Record<string, boolean>>({});
  const [removingIds, setRemovingIds] = useState<Record<string, boolean>>({});
  const [sending, setSending] = useState(false);
  const [seatFilter, setSeatFilter] = useState<SeatFilter>("all");
  const [copiedPath, setCopiedPath] = useState(false);
  const [profileOpen, setProfileOpen] = useState(true);
  const [editingProfile, setEditingProfile] = useState(false);
  const [profileForm, setProfileForm] = useState({
    name: "",
    roleDescription: "",
    avatar: "",
  });
  const [savingProfile, setSavingProfile] = useState(false);
  const [ask, setAsk] = useState<null | {
    kind: "team" | "seat";
    name: string;
    provider: string;
    workspace: string;
    model: string;
    roleDescription: string;
    avatar: string;
  }>(null);

  const logRef = useRef<HTMLDivElement>(null);
  const drafts = useRef<Record<string, string>>({});
  const snapRef = useRef<Snapshot | null>(null);
  const sessionIdRef = useRef<string | null>(null);
  const draftRef = useRef<string>("");
  const loadingOlderRef = useRef(false);

  sessionIdRef.current = sessionId;
  draftRef.current = draft;

  const teamSessions = useMemo(
    () => (snap?.sessions ?? []).filter((s) => !teamId || s.team_id === teamId),
    [snap, teamId],
  );

  const sessions = useMemo(() => {
    if (seatFilter === "active") {
      return teamSessions.filter(
        (s) => s.seat === "working" || s.seat === "queued" || s.seat === "awaiting_approval",
      );
    }
    if (seatFilter === "attention") {
      return teamSessions.filter(
        (s) => s.seat === "awaiting_approval" || s.seat === "error" || s.seat === "away",
      );
    }
    return teamSessions;
  }, [teamSessions, seatFilter]);

  const selected = teamSessions.find((s) => s.id === sessionId) ?? null;
  const notices = snap?.notifications ?? [];
  const readyClis = (snap?.probes ?? []).filter((p) => p.available);
  const currentTeam = snap?.teams.find((t) => t.id === teamId) ?? null;

  const officeStats = useMemo(() => {
    let working = 0;
    let approval = 0;
    let queued = 0;
    let attention = 0;
    for (const s of teamSessions) {
      if (s.seat === "working") working++;
      if (s.seat === "awaiting_approval") {
        approval++;
        attention++;
      }
      if (s.seat === "queued" || (s.queued_count ?? 0) > 0) queued++;
      if (s.seat === "error" || s.seat === "away") attention++;
    }
    return { total: teamSessions.length, working, approval, queued, attention };
  }, [teamSessions]);

  async function refresh(next?: Snapshot) {
    const s = next ?? (await api.snapshot());
    snapRef.current = s;
    setSnap(s);
    setTeamId((cur) => cur ?? s.teams[0]?.id ?? null);
  }

  async function loadLatest(sid: string) {
    const page = await api.listMessages(sid, { limit: PAGE });
    setMessages(page);
    setHasOlder(page.length === PAGE);
  }

  function selectSession(id: string) {
    const curId = sessionIdRef.current;
    if (curId && curId !== id) {
      drafts.current[curId] = draftRef.current;
    }
    sessionIdRef.current = id;
    setSessionId(id);
    const nextDraft = drafts.current[id] ?? "";
    draftRef.current = nextDraft;
    setDraft(nextDraft);
    setFiles([]);
    setEditingProfile(false);
  }

  // If no session is selected yet (or current was deleted), auto-select faulty/away seat so its Profile is immediately visible
  useEffect(() => {
    if (teamSessions.length === 0) {
      if (sessionId !== null) setSessionId(null);
      return;
    }
    if (sessionId && teamSessions.some((s) => s.id === sessionId)) {
      return;
    }
    const attentionSeat = teamSessions.find(
      (s) => s.seat === "away" || s.seat === "error",
    );
    if (attentionSeat) {
      selectSession(attentionSeat.id);
      setProfileOpen(true);
    }
  }, [teamSessions, sessionId]);

  // Sync profile edit form when selected employee changes
  useEffect(() => {
    if (!selected) return;
    const idx = teamSessions.findIndex((x) => x.id === selected.id);
    const charId = pickCharacter(selected, idx >= 0 ? idx : 0);
    setProfileForm({
      name: selected.name,
      roleDescription: selected.role_description || "通用研发与代码协作",
      avatar: charId,
    });
    if (selected.seat === "away" || selected.seat === "error") {
      setProfileOpen(true);
    }
  }, [
    selected?.id,
    selected?.name,
    selected?.role_description,
    selected?.avatar,
    selected?.seat,
  ]);

  async function jumpTo(session: string, noteId?: string) {
    setErr(null);
    const latest = snapRef.current ?? (await api.snapshot().catch(() => null));
    const sess = latest?.sessions.find((s) => s.id === session);
    if (latest && !sess) {
      setErr("该通知对应的员工工位不存在或已移除");
      if (noteId) await api.markRead(noteId).catch(() => {});
      await refresh().catch(() => {});
      return;
    }
    if (sess) {
      setTeamId(sess.team_id);
      setSeatFilter("all");
    }
    selectSession(session);
    if (noteId) await api.markRead(noteId).catch(() => {});
    await refresh().catch((e) => setErr(String(e)));
  }

  useEffect(() => {
    refresh()
      .then(() => {
        if (isDeskDemo()) selectSession("s-ada");
      })
      .catch((e) => {
        if (!String(e).includes("invoke")) setErr(String(e));
      });
    if (isDeskDemo()) return;
    const unsubs: Array<() => void> = [];
    listen<Snapshot>("desk://snapshot", (ev) => {
      snapRef.current = ev.payload;
      setSnap(ev.payload);
      setTeamId((cur) => cur ?? ev.payload.teams[0]?.id ?? null);
    })
      .then((fn) => unsubs.push(fn))
      .catch(() => {});
    listen<Jump>("desk://jump", (ev) => {
      const { session_id, notification_id } = ev.payload;
      if (session_id) void jumpTo(session_id, notification_id || undefined);
    })
      .then((fn) => unsubs.push(fn))
      .catch(() => {});
    listen<boolean>("desk://visible", (ev) => {
      document.documentElement.classList.toggle("paused", !ev.payload);
    })
      .then((fn) => unsubs.push(fn))
      .catch(() => {});
    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "over") setDropOn(true);
        if (event.payload.type === "leave") setDropOn(false);
        if (event.payload.type === "drop") {
          setDropOn(false);
          Promise.all(event.payload.paths.map((p) => api.stageAttachment(p)))
            .then((staged) => setFiles((cur) => [...cur, ...staged]))
            .catch((e) => setErr(String(e)));
        }
      })
      .then((fn) => unsubs.push(fn))
      .catch(() => {});
    return () => unsubs.forEach((fn) => fn());
  }, []);

  useEffect(() => {
    if (!ask) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setAsk(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ask]);

  useEffect(() => {
    if (!sessionId) {
      setMessages([]);
      setHasOlder(false);
      return;
    }
    loadLatest(sessionId).catch((e) => setErr(String(e)));
  }, [sessionId, selected?.seat, selected?.run_id]);

  useEffect(() => {
    if (loadingOlderRef.current) {
      loadingOlderRef.current = false;
      return;
    }
    const el = logRef.current;
    if (!el) return;
    if (typeof el.scrollTo === "function") {
      el.scrollTo(0, el.scrollHeight);
    } else {
      el.scrollTop = el.scrollHeight;
    }
  }, [messages, selected?.streaming_text]);

  function startTeam() {
    setErr(null);
    setAsk({
      kind: "team",
      name: "实验室",
      provider: snap?.provider ?? "fake",
      workspace: "",
      model: "",
      roleDescription: "",
      avatar: "",
    });
  }

  function startEmployee() {
    if (!teamId) {
      setErr("先建一个团队");
      return;
    }
    if (readyClis.length === 0) {
      setErr(snap?.probes?.length ? "没有可用的 CLI，先点「检查 CLI」" : "先点「检查 CLI」");
      return;
    }
    setErr(null);
    const existingNames = new Set(teamSessions.map((s) => s.name));
    const candidates = ["Ada", "Ben", "Cara", "Dan", "Eva", "Finn", "Grace", "Hank"];
    const defaultName =
      candidates.find((n) => !existingNames.has(n)) ?? `员工-${teamSessions.length + 1}`;
    const defaultProvider = readyClis[0].name;
    const { defaultModel } = getModelsForProvider(defaultProvider, snap?.probes);
    setAsk({
      kind: "seat",
      name: defaultName,
      provider: defaultProvider,
      workspace: "",
      model: defaultModel,
      roleDescription: "",
      avatar: "",
    });
  }

  async function pickWorkspaceDir() {
    if (!ask) return;
    try {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked === "string" && picked) {
        setAsk((cur) => (cur ? { ...cur, workspace: picked } : null));
      }
    } catch (e) {
      setErr(String(e));
    }
  }

  async function probeClis() {
    setErr(null);
    setProbing(true);
    try {
      await api.probe();
      await refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setProbing(false);
    }
  }

  async function clockIn(id: string) {
    if (clockingIds[id]) return;
    setErr(null);
    setClockingIds((cur) => ({ ...cur, [id]: true }));
    try {
      const view = await Promise.race([
        api.clockIn(id),
        new Promise<never>((_, reject) =>
          setTimeout(
            () =>
              reject(
                new Error(
                  "到岗响应超时，请在员工档案中检查绑定的 CLI 来源是否已登录",
                ),
              ),
            15000,
          ),
        ),
      ]);
      if (view && snapRef.current) {
        const next: Snapshot = {
          ...snapRef.current,
          sessions: snapRef.current.sessions.map((s) => (s.id === id ? view : s)),
        };
        snapRef.current = next;
        setSnap(next);
      }
      await refresh();
      if (sessionIdRef.current === id) await loadLatest(id);
    } catch (e) {
      setErr(String(e));
      if (sessionIdRef.current === id) {
        await loadLatest(id).catch(() => {});
      }
    } finally {
      setClockingIds((cur) => {
        const next = { ...cur };
        delete next[id];
        return next;
      });
    }
  }

  async function handleDeleteEmployee(id: string) {
    if (removingIds[id]) return;
    setErr(null);
    setRemovingIds((cur) => ({ ...cur, [id]: true }));
    try {
      await api.deleteSession(id);
      delete drafts.current[id];
      if (sessionIdRef.current === id) {
        sessionIdRef.current = null;
        setSessionId(null);
        setMessages([]);
      }
      await refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setRemovingIds((cur) => {
        const next = { ...cur };
        delete next[id];
        return next;
      });
    }
  }

  async function handleSaveProfile() {
    if (!selected || savingProfile) return;
    setErr(null);
    setSavingProfile(true);
    try {
      const lockedModel =
        selected.model ||
        getModelsForProvider(selected.provider, snap?.probes).defaultModel;
      const view = await api.updateSessionProfile(selected.id, {
        name: profileForm.name.trim() || selected.name,
        provider: selected.provider,
        model: lockedModel,
        roleDescription: profileForm.roleDescription.trim() || "通用研发与代码协作",
        avatar: profileForm.avatar,
      });
      if (view && snapRef.current) {
        const next: Snapshot = {
          ...snapRef.current,
          sessions: snapRef.current.sessions.map((s) => (s.id === selected.id ? view : s)),
        };
        snapRef.current = next;
        setSnap(next);
      }
      setEditingProfile(false);
      await refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setSavingProfile(false);
    }
  }

  async function installAdapter(name: string) {
    setErr(null);
    setInstalling(name);
    try {
      await api.installAdapter(name);
      await refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setInstalling(null);
    }
  }

  async function confirmAsk() {
    if (!ask) return;
    const name = ask.name.trim();
    if (!name) return;
    const currentAsk = ask;
    setAsk(null);
    try {
      if (currentAsk.kind === "team") {
        const team = await api.createTeam(name);
        setTeamId(team.id);
      } else if (teamId) {
        const { defaultModel } = getModelsForProvider(
          currentAsk.provider,
          snap?.probes,
        );
        const chosenModel = currentAsk.model.trim();
        const isNonDefaultModel = Boolean(
          chosenModel && chosenModel !== defaultModel,
        );
        const hasCustomProfile = Boolean(
          isNonDefaultModel ||
            currentAsk.roleDescription.trim() ||
            currentAsk.avatar.trim(),
        );
        const created = hasCustomProfile
          ? await api.createSession(
              teamId,
              name,
              currentAsk.workspace.trim(),
              currentAsk.provider,
              {
                model: chosenModel || undefined,
                roleDescription: currentAsk.roleDescription.trim() || undefined,
                avatar: currentAsk.avatar.trim() || undefined,
              },
            )
          : await api.createSession(
              teamId,
              name,
              currentAsk.workspace.trim(),
              currentAsk.provider,
            );
        if (created?.id) {
          selectSession(created.id);
        }
      }
      await refresh();
    } catch (e) {
      setErr(String(e));
    }
  }

  async function send(replace = false) {
    const text = draft.trim() || (files.length > 0 ? "请查看附件" : "");
    if (!selected || !text || sending) return;
    setErr(null);
    setSending(true);
    const ids = files.map((f) => f.id);
    try {
      if (replace) {
        await api.stopAndReplace(selected.id, text, ids);
      } else {
        await api.submitMessage(selected.id, text, ids);
      }
      drafts.current[selected.id] = "";
      draftRef.current = "";
      setDraft("");
      setFiles([]);
      await loadLatest(selected.id);
      await refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setSending(false);
    }
  }

  async function handleCancelRun(runId: string) {
    setErr(null);
    try {
      await api.cancelRun(runId);
      await refresh();
      if (sessionId) await loadLatest(sessionId);
    } catch (e) {
      setErr(String(e));
    }
  }

  async function handleResolveApproval(runId: string, allow: boolean) {
    setErr(null);
    try {
      await api.resolveApproval(runId, allow);
      await refresh();
      if (sessionId) await loadLatest(sessionId);
    } catch (e) {
      setErr(String(e));
    }
  }

  async function handleResumeQueue() {
    setErr(null);
    try {
      await api.resumeQueue();
      await refresh();
    } catch (e) {
      setErr(String(e));
    }
  }

  async function handleMarkAllRead() {
    setErr(null);
    try {
      await api.markAllRead?.();
      await refresh();
    } catch (e) {
      setErr(String(e));
    }
  }

  async function attach() {
    setErr(null);
    try {
      const picked = await open({ multiple: true });
      if (!picked) return;
      const paths = Array.isArray(picked) ? picked : [picked];
      const staged = await Promise.all(paths.map((p) => api.stageAttachment(p)));
      setFiles((cur) => [...cur, ...staged]);
    } catch (e) {
      setErr(String(e));
    }
  }

  async function onPaste(e: ClipboardEvent) {
    const items = [...(e.clipboardData?.items ?? [])];
    const blobs = items
      .filter((it) => it.kind === "file")
      .map((it) => it.getAsFile())
      .filter((f): f is File => !!f);
    if (blobs.length === 0) return;
    e.preventDefault();
    try {
      for (const f of blobs) {
        const buf = new Uint8Array(await f.arrayBuffer());
        const staged = await api.stageBytes(
          f.name || `paste-${Date.now()}.png`,
          f.type || "image/png",
          Array.from(buf),
        );
        setFiles((cur) => [...cur, staged]);
      }
    } catch (e) {
      setErr(String(e));
    }
  }

  async function loadOlder() {
    if (!sessionId || messages.length === 0) return;
    const first = messages[0];
    const el = logRef.current;
    const prevHeight = el?.scrollHeight ?? 0;
    try {
      const older = await api.listMessages(sessionId, {
        limit: PAGE,
        beforeCreatedAt: first.created_at,
        beforeId: first.id,
      });
      loadingOlderRef.current = true;
      setHasOlder(older.length === PAGE);
      setMessages((cur) => [...older, ...cur]);
      requestAnimationFrame(() => {
        if (el) {
          el.scrollTop = Math.max(0, el.scrollHeight - prevHeight);
        }
      });
    } catch (e) {
      setErr(String(e));
    }
  }

  function copyWorkspacePath(path: string) {
    navigator.clipboard?.writeText(path).catch(() => {});
    setCopiedPath(true);
    setTimeout(() => setCopiedPath(false), 1600);
  }

  const canSend = Boolean((draft.trim() || files.length > 0) && !sending);
  const canCancelSelected = Boolean(
    selected?.run_id &&
      (selected.seat === "working" ||
        selected.seat === "queued" ||
        selected.seat === "awaiting_approval"),
  );

  const selectedIndex = selected
    ? teamSessions.findIndex((x) => x.id === selected.id)
    : 0;
  const selectedChar = selected
    ? pickCharacter(selected, selectedIndex >= 0 ? selectedIndex : 0)
    : "sebastian";
  const selectedProbe = selected
    ? snap?.probes.find((p) => p.name === selected.provider)
    : null;
  const selectedModelDisplay = selected
    ? formatModelDisplay(selected.provider, selected.model, snap?.probes)
    : "default";
  const selectedRole =
    selected?.role_description?.trim() || "通用研发与代码协作";

  const askModels = useMemo(() => {
    if (!ask || ask.kind !== "seat") return [];
    return getModelsForProvider(ask.provider, snap?.probes).options;
  }, [ask?.kind, ask?.provider, snap?.probes]);

  return (
    <div className={`shell ${dropOn ? "drop" : ""}`}>
      <aside className="rail">
        <header className="rail-header">
          <div className="brand">
            <span className="brand-junimo" aria-hidden="true" />
            <h1>Desk</h1>
          </div>
          <span className="provider" title="默认 ACP Provider">
            {CLI_LABEL[snap?.provider ?? ""] ?? snap?.provider ?? "…"}
          </span>
        </header>

        <div className="rail-section">
          <div className="sec-head">
            <h2>CLI</h2>
            <span className="sec-meta" aria-hidden="true">
              {readyClis.length > 0 ? `${readyClis.length} 就绪` : "待探测"}
            </span>
          </div>
          <button
            type="button"
            className="probe-btn"
            onClick={() => void probeClis()}
            disabled={probing}
          >
            {probing ? "检查中…" : "检查 CLI"}
          </button>
          <ul className="probes">
            {(snap?.probes ?? []).map((p) => (
              <li key={p.name} className={p.available ? "ok" : "bad"}>
                <div className="probe-row">
                  <span className="status-dot" aria-hidden="true" />
                  <b>{CLI_LABEL[p.name] ?? p.name}</b>
                </div>
                <small>
                  {p.available ? "可用" : "不可用"} · {p.detail}
                  {p.available && p.models && p.models.length > 0
                    ? ` · ${p.models.length} 个模型`
                    : ""}
                </small>
                {!p.available && p.adapter_package && (
                  <button
                    type="button"
                    disabled={installing !== null || probing}
                    onClick={() => void installAdapter(p.name)}
                  >
                    {installing === p.name ? "安装中…" : "安装 ACP"}
                  </button>
                )}
              </li>
            ))}
            {!probing && (snap?.probes ?? []).length === 0 && (
              <li className="hint">尚未检查。会扫描本机已装 CLI，并对每个做一次最短 ACP 验证。</li>
            )}
          </ul>
        </div>

        <div className="rail-section">
          <div className="sec-head">
            <h2>团队</h2>
            <span className="sec-meta" aria-hidden="true">
              {snap?.teams.length ?? 0}
            </span>
          </div>
          <button type="button" className="team-add-btn" onClick={startTeam}>
            新建团队
          </button>
          <ul className="teams">
            {snap?.teams.map((t) => {
              const count = snap.sessions.filter((s) => s.team_id === t.id).length;
              const busyCount = snap.sessions.filter(
                (s) =>
                  s.team_id === t.id &&
                  (s.seat === "working" || s.seat === "awaiting_approval"),
              ).length;
              return (
                <li key={t.id}>
                  <button
                    type="button"
                    className={t.id === teamId ? "on" : ""}
                    onClick={() => {
                      setTeamId(t.id);
                      setSeatFilter("all");
                    }}
                  >
                    <span>{t.name}</span>
                    <span className="team-badges" aria-hidden="true">
                      {busyCount > 0 && <i className="team-busy-dot" />}
                      <small className="team-count">{count}</small>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        </div>

        <div className="rail-section notices-section">
          <div className="sec-head">
            <h2>
              通知 {notices.length > 0 && <small className="badge-count">{notices.length}</small>}
            </h2>
            {notices.length > 1 && (
              <button
                type="button"
                className="link-btn"
                onClick={() => void handleMarkAllRead()}
                title="全部标为已读"
              >
                全部已读
              </button>
            )}
          </div>
          <ul className="notices">
            {notices.map((n) => {
              const who = snap?.sessions.find((s) => s.id === n.session_id)?.name;
              return (
                <li key={n.id} className={`${n.unread ? "unread" : ""} kind-${n.kind}`}>
                  <button type="button" onClick={() => void jumpTo(n.session_id, n.id)}>
                    <b>{n.title}</b>
                    <div className="notice-meta" aria-hidden="true">
                      {who && <small className="notice-who">{who}</small>}
                      {n.created_at > 0 && <small>{formatTime(n.created_at)}</small>}
                    </div>
                  </button>
                </li>
              );
            })}
            {notices.length === 0 && <li className="hint">没有未读通知</li>}
          </ul>
        </div>

        {snap?.dispatch_paused && (
          <button type="button" className="resume" onClick={() => void handleResumeQueue()}>
            恢复排队
          </button>
        )}
      </aside>

      <main className="office">
        <div className="office-bar">
          <div className="office-title-group">
            <h2>{currentTeam ? `${currentTeam.name} · 办公室` : "办公室"}</h2>
            {officeStats.total > 0 && (
              <div className="office-pills" aria-hidden="true">
                <span className="stat-pill">共 {officeStats.total} 席</span>
                {officeStats.working > 0 && (
                  <span className="stat-pill busy">{officeStats.working} 执行中</span>
                )}
                {officeStats.approval > 0 && (
                  <span className="stat-pill warn">{officeStats.approval} 待审批</span>
                )}
                {officeStats.queued > 0 && (
                  <span className="stat-pill">{officeStats.queued} 排队</span>
                )}
              </div>
            )}
          </div>
          <div className="office-controls">
            {officeStats.total > 1 && (
              <div className="filter-tabs" role="group" aria-label="工位筛选">
                <button
                  type="button"
                  className={seatFilter === "all" ? "on" : ""}
                  onClick={() => setSeatFilter("all")}
                >
                  全部
                </button>
                <button
                  type="button"
                  className={seatFilter === "active" ? "on" : ""}
                  onClick={() => setSeatFilter("active")}
                >
                  忙碌
                </button>
                <button
                  type="button"
                  className={seatFilter === "attention" ? "on" : ""}
                  onClick={() => setSeatFilter("attention")}
                >
                  需关注
                </button>
              </div>
            )}
            <button type="button" className="primary add-seat-btn" onClick={startEmployee}>
              添加工位
            </button>
          </div>
        </div>

        <div className="room">
          <div className="wall" aria-hidden="true">
            <i className="pane" />
            <i className="pane pane-right" />
            <i className="board" />
            <i className="robin-builder" title="Robin 正在维护办公室" />
            <i className="shelf" />
            <i className="plant" />
            <i className="plant plant-left" />
          </div>
          <div className="floor">
            <div className="rug" aria-hidden="true" />
            <i
              className={`office-cat ${officeStats.working > 0 ? "awake" : "sleeping"}`}
              aria-hidden="true"
              title="办公室橘猫"
            />
            <i
              className={`office-junimo ${officeStats.working > 0 ? "dancing" : ""}`}
              aria-hidden="true"
              title="祝尼魔助手"
            />
            {sessions.map((s, idx) => {
              const charName = pickCharacter(s, idx);
              const isClocking = Boolean(clockingIds[s.id]);
              const emoteCls = emoteRowClass(s.seat, isClocking);
              const roleLabel = s.role_description?.trim() || "通用研发与代码协作";
              return (
                <div key={s.id} className="desk-wrap">
                  <button
                    type="button"
                    className={`desk ${s.seat} ${isClocking ? "arriving" : ""} ${s.id === sessionId ? "sel" : ""}`}
                    onClick={() => selectSession(s.id)}
                  >
                    <i className="desk-surface" aria-hidden="true" />
                    <i className="lamp" aria-hidden="true" />
                    <i className="monitor" aria-hidden="true">
                      <span className="monitor-code" />
                    </i>
                    <i className="keyboard" aria-hidden="true" />
                    <i className="mug" aria-hidden="true" />
                    <i className={`sv-emote ${emoteCls}`} aria-hidden="true" />
                    <i
                      className={`sv-char char-${charName} pose-${isClocking ? "arriving" : s.seat}`}
                      aria-hidden="true"
                    />
                    {s.seat === "working" && (
                      <i className="sv-junimo-helper" aria-hidden="true" />
                    )}
                    {(s.queued_count ?? 0) > 0 && (
                      <span className="queue-badge" aria-hidden="true">
                        +{s.queued_count} 排队
                      </span>
                    )}
                    <div className="desk-info">
                      <strong>{s.name}</strong>
                      <span className="desk-role" aria-hidden="true">
                        {roleLabel}
                      </span>
                      <div className="desk-meta">
                        <em>{isClocking ? "到岗中…" : SEAT_LABEL[s.seat] ?? s.seat}</em>
                        <small>{CLI_LABEL[s.provider] ?? s.provider}</small>
                      </div>
                    </div>
                  </button>
                </div>
              );
            })}
            {teamSessions.length === 0 && (
              <div className="empty-room">
                <div className="empty-junimo-art" aria-hidden="true" />
                <p className="empty">还没有员工。左侧建团队，再添加工位。</p>
              </div>
            )}
            {teamSessions.length > 0 && sessions.length === 0 && (
              <div className="empty-room">
                <p className="empty">当前筛选条件下没有工位。</p>
                <button type="button" onClick={() => setSeatFilter("all")}>
                  显示全部工位
                </button>
              </div>
            )}
          </div>
        </div>
      </main>

      <section className="chat">
        {selected ? (
          <>
            <header className="chat-header">
              <div className="chat-identity">
                <div className="chat-title-row">
                  <span className={`seat-badge ${selected.seat}`} aria-hidden="true">
                    {clockingIds[selected.id]
                      ? "状态 · 到岗中"
                      : `状态 · ${SEAT_LABEL[selected.seat] ?? selected.seat}`}
                  </span>
                  <h2>{selected.name}</h2>
                  {(selected.queued_count ?? 0) > 0 && (
                    <span className="chat-queue-pill" aria-hidden="true">
                      队列 {selected.queued_count}
                    </span>
                  )}
                </div>
                <small
                  className="workspace-meta"
                  onClick={() => copyWorkspacePath(selected.workspace_path)}
                  title="点击复制工作目录路径"
                >
                  {CLI_LABEL[selected.provider] ?? selected.provider} · {selectedModelDisplay} ·{" "}
                  {selected.workspace_path}
                  {copiedPath ? " (已复制)" : ""}
                </small>
              </div>
              <div className="chat-actions">
                <button
                  type="button"
                  className={profileOpen ? "on" : ""}
                  onClick={() => setProfileOpen((v) => !v)}
                >
                  员工档案
                </button>
                {canCancelSelected && (
                  <button
                    type="button"
                    className="danger-outline"
                    onClick={() => void handleCancelRun(selected.run_id!)}
                  >
                    取消
                  </button>
                )}
              </div>
            </header>

            {profileOpen && (
              <section className="employee-profile" aria-label="员工档案">
                <div className="profile-top">
                  <div className="profile-photo-frame">
                    <i
                      className={`sv-portrait portrait-${editingProfile ? profileForm.avatar || selectedChar : selectedChar} expr-${selected.seat}`}
                      role="img"
                      aria-label={`${selected.name} 员工照片`}
                    />
                    <span className={`profile-status-dot ${selected.seat}`} aria-hidden="true" />
                  </div>
                  <div className="profile-meta-grid">
                    <div className="profile-meta-item">
                      <span className="meta-label">来源 (CLI · 已锁定)</span>
                      <strong className="meta-val">
                        {CLI_LABEL[selected.provider] ?? selected.provider}
                        {selectedProbe?.detail ? ` (${selectedProbe.detail})` : ""}
                      </strong>
                    </div>
                    <div className="profile-meta-item">
                      <span className="meta-label">模型 (已锁定)</span>
                      <strong className="meta-val" title={selected.model}>
                        {selectedModelDisplay}
                      </strong>
                    </div>
                    <div className="profile-meta-item">
                      <span className="meta-label">建立时间</span>
                      <strong className="meta-val">{formatCreatedAt(selected.created_at)}</strong>
                    </div>
                    <div className="profile-meta-item full-span">
                      <span className="meta-label">负责的工作</span>
                      <strong className="meta-val role-val">{selectedRole}</strong>
                    </div>
                  </div>
                </div>

                {selected.last_error && (selected.seat === "error" || selected.seat === "away") && (
                  <div className="profile-fault-box" aria-live="polite">
                    <span>
                      故障诊断：{selected.last_error.split("\n")[0]}
                    </span>
                  </div>
                )}

                {editingProfile ? (
                  <div className="profile-editor">
                    <div className="editor-grid">
                      <label>
                        姓名
                        <input
                          aria-label="档案姓名"
                          value={profileForm.name}
                          onChange={(e) =>
                            setProfileForm({ ...profileForm, name: e.target.value })
                          }
                        />
                      </label>
                      <label>
                        照片形象
                        <select
                          aria-label="档案照片"
                          value={profileForm.avatar}
                          onChange={(e) =>
                            setProfileForm({ ...profileForm, avatar: e.target.value })
                          }
                        >
                          {STARDEW_CAST.map((c) => (
                            <option key={c.id} value={c.id}>
                              {c.label}
                            </option>
                          ))}
                        </select>
                      </label>
                    </div>
                    <label className="editor-full-label">
                      负责的工作
                      <input
                        aria-label="档案职责"
                        list="profile-role-presets"
                        value={profileForm.roleDescription}
                        placeholder="描述该员工负责的模块与职责"
                        onChange={(e) =>
                          setProfileForm({
                            ...profileForm,
                            roleDescription: e.target.value,
                          })
                        }
                      />
                      <datalist id="profile-role-presets">
                        {ROLE_PRESETS.map((r) => (
                          <option key={r} value={r} />
                        ))}
                      </datalist>
                    </label>
                    <small className="hint">
                      注：员工的 CLI 来源与模型在建档后已锁定，不可中途切换。
                    </small>
                    <div className="profile-editor-actions">
                      <button type="button" onClick={() => setEditingProfile(false)}>
                        取消编辑
                      </button>
                      <button
                        type="button"
                        className="primary"
                        disabled={savingProfile}
                        onClick={() => void handleSaveProfile()}
                      >
                        {savingProfile ? "保存中…" : "保存档案"}
                      </button>
                    </div>
                  </div>
                ) : (
                  <div className="profile-actions-bar">
                    <button
                      type="button"
                      className={
                        selected.seat === "away" || selected.seat === "error"
                          ? "primary"
                          : ""
                      }
                      disabled={Boolean(clockingIds[selected.id]) || selected.seat === "working"}
                      onClick={() => void clockIn(selected.id)}
                    >
                      {clockingIds[selected.id]
                        ? "到岗中…"
                        : selected.seat === "away" || selected.seat === "error"
                          ? "恢复上班"
                          : "重新到岗"}
                    </button>
                    <button
                      type="button"
                      onClick={() => setEditingProfile(true)}
                      disabled={selected.seat === "working"}
                    >
                      编辑档案
                    </button>
                    <button
                      type="button"
                      className="danger-outline"
                      disabled={Boolean(removingIds[selected.id])}
                      onClick={() => void handleDeleteEmployee(selected.id)}
                    >
                      {removingIds[selected.id]
                        ? "清除中…"
                        : selected.seat === "error" || selected.seat === "away"
                          ? "清除故障员工"
                          : "移除员工"}
                    </button>
                  </div>
                )}
              </section>
            )}

            {selected.seat === "awaiting_approval" && selected.run_id && (
              <div className="approve" role="region" aria-label="原生审批请求">
                <div className="approve-text">
                  <span className="approve-dot" aria-hidden="true" />
                  <span>
                    等待原生审批
                    {selected.pending_tool ? ` · ${selected.pending_tool}` : ""}
                  </span>
                </div>
                <div className="approve-btns">
                  <button
                    type="button"
                    className="primary"
                    onClick={() => void handleResolveApproval(selected.run_id!, true)}
                  >
                    批准
                  </button>
                  <button
                    type="button"
                    className="danger-outline"
                    onClick={() => void handleResolveApproval(selected.run_id!, false)}
                  >
                    拒绝
                  </button>
                </div>
              </div>
            )}

            <div className="log" ref={logRef}>
              {hasOlder && (
                <button type="button" className="older" onClick={() => void loadOlder()}>
                  加载更早
                </button>
              )}
              {messages.length === 0 && selected.seat !== "working" && (
                <div className="chat-welcome">
                  <p className="welcome-title">
                    与 <b>{selected.name}</b> 开始协作
                  </p>
                  <p className="hint">
                    职责：{selectedRole} · 支持多轮长会话记忆、拖拽上传附件与粘贴截图
                  </p>
                  <div className="quick-prompts">
                    {QUICK_PROMPTS.map((q) => (
                      <button
                        key={q}
                        type="button"
                        className="prompt-chip"
                        onClick={() => {
                          draftRef.current = q;
                          setDraft(q);
                        }}
                      >
                        {q}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              {messages.map((m) => (
                <article key={m.id} className={`msg ${m.role}`}>
                  <div className="msg-head">
                    <b>
                      {m.role === "user"
                        ? "你"
                        : m.role === "error"
                          ? "错误"
                          : selected.name}
                    </b>
                    {m.created_at > 0 && (
                      <time aria-hidden="true">{formatTime(m.created_at)}</time>
                    )}
                  </div>
                  <MessageContent content={m.content} attachments={m.attachments} />
                </article>
              ))}
              {selected.seat === "working" && (
                <article className="msg assistant streaming" aria-live="polite">
                  <div className="msg-head">
                    <b>{selected.name}</b>
                    <span className="typing-dots" aria-hidden="true">
                      <i />
                      <i />
                      <i />
                    </span>
                  </div>
                  <MessageContent
                    content={selected.streaming_text || "正在思考与执行…"}
                  />
                </article>
              )}
            </div>

            {files.length > 0 && (
              <div className="chips">
                {files.map((f) => (
                  <span key={f.id} className="chip">
                    {f.mime.startsWith("image/") && f.stored_path && (
                      <LocalOrRemoteImage
                        src={f.stored_path}
                        alt={f.original_name}
                        className="chip-thumb"
                      />
                    )}
                    <span className="chip-name">{f.original_name}</span>
                    {f.size > 0 && (
                      <small className="chip-size" aria-hidden="true">
                        {formatBytes(f.size)}
                      </small>
                    )}
                    <button
                      type="button"
                      aria-label={`移除附件 ${f.original_name}`}
                      onClick={() => setFiles((cur) => cur.filter((x) => x.id !== f.id))}
                    >
                      ×
                    </button>
                  </span>
                ))}
              </div>
            )}

            <footer className="chat-footer">
              <textarea
                value={draft}
                placeholder="给这位员工发消息… 可粘贴截图或拖入文件"
                onChange={(e) => {
                  draftRef.current = e.target.value;
                  setDraft(e.target.value);
                }}
                onPaste={(e) => void onPaste(e)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                    e.preventDefault();
                    void send(false);
                  }
                }}
              />
              <div className="footer-bar">
                <span className="shortcut-hint" aria-hidden="true">
                  ⌘/Ctrl + Enter 发送
                </span>
                <div className="row">
                  <button type="button" onClick={() => void attach()}>
                    附件
                  </button>
                  <button
                    type="button"
                    onClick={() => void send(true)}
                    disabled={!canSend}
                    title="取消当前正在执行的任务，并将此消息排到队列最前"
                  >
                    停止并修改
                  </button>
                  <button
                    type="button"
                    className="primary"
                    onClick={() => void send(false)}
                    disabled={!canSend}
                  >
                    {sending ? "发送中…" : "发送"}
                  </button>
                </div>
              </div>
            </footer>
          </>
        ) : (
          <div className="chat-empty-state">
            <div className="empty-junimo-art" aria-hidden="true" />
            <p className="empty">点工位打开聊天。</p>
            <p className="hint" aria-hidden="true">
              选择一位办公室里的员工查看员工档案、恢复到岗、清除故障员工或派发新任务
            </p>
          </div>
        )}
        {err && (
          <div className="err-banner" role="alert">
            <p className="err">{err}</p>
            <button
              type="button"
              className="err-close"
              aria-label="关闭错误提示"
              onClick={() => setErr(null)}
            >
              ×
            </button>
          </div>
        )}
      </section>

      {ask && (
        <div
          className="ask"
          role="dialog"
          aria-modal="true"
          aria-label={ask.kind === "team" ? "新建团队" : "添加工位"}
          onClick={(e) => {
            if (e.target === e.currentTarget) setAsk(null);
          }}
        >
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void confirmAsk();
            }}
          >
            <h3 className="ask-title">
              {ask.kind === "team" ? "新建团队" : "添加新工位 · 建立员工档案"}
            </h3>
            <label>
              {ask.kind === "team" ? "团队名称" : "员工名称"}
              <input
                autoFocus
                value={ask.name}
                placeholder={ask.kind === "team" ? "例如：核心研发组" : "例如：Ada"}
                onChange={(e) => setAsk({ ...ask, name: e.target.value })}
              />
            </label>
            {ask.kind === "seat" && (
              <>
                <div className="ask-two-col">
                  <label>
                    CLI
                    <select
                      value={ask.provider}
                      onChange={(e) => {
                        const nextProvider = e.target.value;
                        const { defaultModel } = getModelsForProvider(
                          nextProvider,
                          snap?.probes,
                        );
                        setAsk({
                          ...ask,
                          provider: nextProvider,
                          model: defaultModel,
                        });
                      }}
                    >
                      {readyClis.map((p) => (
                        <option key={p.name} value={p.name}>
                          {CLI_LABEL[p.name] ?? p.name}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    模型（建档后锁定）
                    <select
                      aria-label="模型"
                      value={ask.model}
                      onChange={(e) => setAsk({ ...ask, model: e.target.value })}
                    >
                      {askModels.map((m) => (
                        <option key={m.id} value={m.id}>
                          {m.label}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
                <div className="ask-two-col">
                  <label>
                    照片形象
                    <select
                      value={ask.avatar}
                      onChange={(e) => setAsk({ ...ask, avatar: e.target.value })}
                    >
                      <option value="">自动匹配星露谷角色</option>
                      {STARDEW_CAST.map((c) => (
                        <option key={c.id} value={c.id}>
                          {c.label}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    负责的工作（可选）
                    <input
                      value={ask.roleDescription}
                      placeholder="例如：核心架构与代码审查"
                      onChange={(e) =>
                        setAsk({ ...ask, roleDescription: e.target.value })
                      }
                    />
                  </label>
                </div>
                <label>
                  工作目录（可选）
                  <div className="ws-picker-row">
                    <input
                      value={ask.workspace}
                      placeholder="留空则自动创建独立工作目录"
                      onChange={(e) => setAsk({ ...ask, workspace: e.target.value })}
                    />
                    <button type="button" onClick={() => void pickWorkspaceDir()}>
                      浏览…
                    </button>
                  </div>
                </label>
                <p className="hint">
                  该员工创建后将绑定所选 CLI 与已核实的模型（后续不可切换）。工作目录会自动创建。
                </p>
              </>
            )}
            <div className="row">
              <button type="button" onClick={() => setAsk(null)}>
                取消
              </button>
              <button type="submit" className="primary" disabled={!ask.name.trim()}>
                确定
              </button>
            </div>
          </form>
        </div>
      )}
    </div>
  );
}
