import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { api, type Snapshot } from "./api";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: () => Promise.reject(new Error("no webview")),
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));
vi.mock("./api", async () => {
  const actual = await vi.importActual<typeof import("./api")>("./api");
  return {
    ...actual,
    api: {
      snapshot: vi.fn(),
      pump: vi.fn(),
      listMessages: vi.fn(),
      createTeam: vi.fn(),
      createSession: vi.fn(),
      submitMessage: vi.fn(),
      cancelRun: vi.fn(),
      stopAndReplace: vi.fn(),
      resolveApproval: vi.fn(),
      stageAttachment: vi.fn(),
      stageBytes: vi.fn(),
      readImageDataUrl: vi.fn(),
      markRead: vi.fn(),
      markAllRead: vi.fn(),
      resumeQueue: vi.fn(),
      probe: vi.fn(),
      installAdapter: vi.fn(),
      clockIn: vi.fn(),
      setSessionProvider: vi.fn(),
      updateSessionProfile: vi.fn(),
      deleteSession: vi.fn(),
    },
  };
});

function emptySnap(): Snapshot {
  return {
    teams: [],
    sessions: [],
    notifications: [],
    dispatch_paused: false,
    provider: "fake",
    providers: ["fake"],
    probes: [],
  };
}

function labSnap(): Snapshot {
  return {
    teams: [{ id: "t1", name: "实验室" }],
    sessions: [
      {
        id: "s1",
        team_id: "t1",
        name: "Ada",
        workspace_path: "/tmp/ada",
        provider: "fake",
        cli_session_id: "fake-1",
        seat: "idle",
        run_id: null,
      },
      {
        id: "s2",
        team_id: "t1",
        name: "Ben",
        workspace_path: "/tmp/ben",
        provider: "fake",
        cli_session_id: "fake-2",
        seat: "working",
        run_id: "r2",
      },
    ],
    notifications: [
      {
        id: "n1",
        session_id: "s2",
        run_id: "r2",
        kind: "approval",
        priority: 0,
        title: "待审批: write",
        unread: true,
        dedupe_key: "run:r2:approval",
        created_at: 1,
      },
    ],
    dispatch_paused: false,
    provider: "fake",
    providers: ["cursor", "fake"],
    probes: [],
  };
}

describe("Desk 前端", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    vi.mocked(api.snapshot).mockReset();
    vi.mocked(api.listMessages).mockReset();
    vi.mocked(api.createTeam).mockReset();
    vi.mocked(api.createSession).mockReset();
    vi.mocked(api.submitMessage).mockReset();
    vi.mocked(api.markRead).mockReset();
    vi.mocked(api.probe).mockReset();
    vi.mocked(api.installAdapter).mockReset();
    vi.mocked(api.clockIn).mockReset();
    vi.mocked(api.updateSessionProfile).mockReset();
    vi.mocked(api.deleteSession).mockReset();
    vi.mocked(api.readImageDataUrl).mockReset();
    vi.mocked(api.readImageDataUrl).mockResolvedValue("data:image/png;base64,iVBORw0KGgo=");
    vi.mocked(api.listMessages).mockResolvedValue([]);
  });

  it("F01 空办公室提示建团队", async () => {
    vi.mocked(api.snapshot).mockResolvedValue(emptySnap());
    render(<App />);
    expect(await screen.findByText(/还没有员工/)).toBeInTheDocument();
    expect(screen.getByText("新建团队")).toBeInTheDocument();
  });

  it("F01 建团队后出现名称", async () => {
    vi.mocked(api.snapshot)
      .mockResolvedValueOnce(emptySnap())
      .mockResolvedValue(labSnap());
    vi.mocked(api.createTeam).mockResolvedValue({ id: "t1", name: "实验室" });
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "新建团队" }));
    const name = await screen.findByLabelText("团队名称");
    await userEvent.clear(name);
    await userEvent.type(name, "实验室");
    await userEvent.click(screen.getByRole("button", { name: "确定" }));
    await waitFor(() => expect(api.createTeam).toHaveBeenCalledWith("实验室"));
    expect(await screen.findByRole("button", { name: "实验室" })).toBeInTheDocument();
  });

  it("F01 切换工位草稿不串线", async () => {
    vi.mocked(api.snapshot).mockResolvedValue(labSnap());
    vi.mocked(api.listMessages)
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([
        {
          id: "m1",
          session_id: "s1",
          run_id: null,
          role: "user",
          content: "给 Ada",
          request_id: null,
          created_at: 1,
        },
      ])
      .mockResolvedValue([]);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: /Ada/ }));
    const box = await screen.findByPlaceholderText(/给这位员工发消息/);
    await userEvent.type(box, "只给Ada");
    await userEvent.click(screen.getByRole("button", { name: /Ben/ }));
    expect(await screen.findByPlaceholderText(/给这位员工发消息/)).toHaveValue("");
    await userEvent.click(screen.getByRole("button", { name: /Ada/ }));
    expect(await screen.findByPlaceholderText(/给这位员工发消息/)).toHaveValue("只给Ada");
  });

  it("F04 发送走 submitMessage", async () => {
    vi.mocked(api.snapshot).mockResolvedValue(labSnap());
    vi.mocked(api.submitMessage).mockResolvedValue("run-1");
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: /Ada/ }));
    await userEvent.type(await screen.findByPlaceholderText(/给这位员工发消息/), "hello");
    await userEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() =>
      expect(api.submitMessage).toHaveBeenCalledWith("s1", "hello", []),
    );
  });

  it("F06 点通知跳到对应员工并标已读", async () => {
    vi.mocked(api.snapshot).mockResolvedValue(labSnap());
    vi.mocked(api.markRead).mockResolvedValue();
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: /待审批: write/ }));
    await waitFor(() => expect(api.markRead).toHaveBeenCalledWith("n1"));
    expect(await screen.findByRole("heading", { name: "Ben" })).toBeInTheDocument();
    expect(screen.getByText("取消")).toBeInTheDocument();
  });

  it("下班工位在办公室显示为空位", async () => {
    vi.mocked(api.snapshot).mockResolvedValue({
      ...labSnap(),
      sessions: [
        {
          ...labSnap().sessions[0],
          seat: "away",
          run_id: "r-away",
        },
      ],
    });
    render(<App />);
    expect(await screen.findByText("下班")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Ada/ })).toHaveClass("away");
    expect(screen.getAllByRole("button", { name: "恢复上班" }).length).toBeGreaterThan(0);
  });

  it("点恢复上班会叫员工重新到岗", async () => {
    vi.mocked(api.snapshot)
      .mockResolvedValueOnce({
        ...labSnap(),
        sessions: [{ ...labSnap().sessions[0], seat: "away", run_id: "r-away" }],
      })
      .mockResolvedValue({
        ...labSnap(),
        sessions: [{ ...labSnap().sessions[0], seat: "idle" }],
      });
    vi.mocked(api.clockIn).mockResolvedValue({
      ...labSnap().sessions[0],
      seat: "idle",
    });
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "恢复上班" }));
    await waitFor(() => expect(api.clockIn).toHaveBeenCalledWith("s1"));
    expect(await screen.findByText("空闲")).toBeInTheDocument();
  });

  it("添加工位前必须先检查 CLI", async () => {
    vi.mocked(api.snapshot).mockResolvedValue(labSnap());
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "添加工位" }));
    expect(await screen.findByText(/先点「检查 CLI」/)).toBeInTheDocument();
  });

  it("检查 CLI 后可以选可用 CLI 建员工", async () => {
    const probed: Snapshot = {
      ...labSnap(),
      probes: [{ name: "fake", available: true, detail: "desk-fake-cli", adapter_package: null }],
    };
    vi.mocked(api.snapshot).mockResolvedValueOnce(labSnap()).mockResolvedValue(probed);
    vi.mocked(api.probe).mockResolvedValue(probed.probes);
    vi.mocked(api.createSession).mockResolvedValue({
      id: "s3",
      team_id: "t1",
      name: "Cara",
      workspace_path: "/tmp/cara",
      provider: "fake",
      cli_session_id: null,
      seat: "idle",
      run_id: null,
    });
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "检查 CLI" }));
    await waitFor(() => expect(api.probe).toHaveBeenCalled());
    expect(await screen.findByText(/可用/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "添加工位" }));
    expect(await screen.findByLabelText("员工名称")).toBeInTheDocument();
    expect(screen.getByLabelText("CLI")).toHaveValue("fake");
    const name = screen.getByLabelText("员工名称");
    await userEvent.clear(name);
    await userEvent.type(name, "Cara");
    await userEvent.click(screen.getByRole("button", { name: "确定" }));
    await waitFor(() =>
      expect(api.createSession).toHaveBeenCalledWith("t1", "Cara", "", "fake"),
    );
  });

  it("非原生 ACP 的 CLI 可以一键安装适配器", async () => {
    const before: Snapshot = {
      ...labSnap(),
      probes: [
        {
          name: "codex",
          available: false,
          detail: "已安装 Codex，缺少 ACP 适配器",
          adapter_package: "@agentclientprotocol/codex-acp",
        },
      ],
    };
    const after: Snapshot = {
      ...labSnap(),
      probes: [
        {
          name: "codex",
          available: true,
          detail: "codex-acp",
          adapter_package: null,
        },
      ],
    };
    vi.mocked(api.snapshot).mockResolvedValueOnce(before).mockResolvedValue(after);
    vi.mocked(api.installAdapter).mockResolvedValue(after.probes);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "安装 ACP" }));
    await waitFor(() => expect(api.installAdapter).toHaveBeenCalledWith("codex"));
    expect(await screen.findByText(/可用/)).toBeInTheDocument();
  });

  it("点通知跳转不会把当前员工草稿串到目标员工", async () => {
    vi.mocked(api.snapshot).mockResolvedValue(labSnap());
    vi.mocked(api.markRead).mockResolvedValue();
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: /Ada/ }));
    const box = await screen.findByPlaceholderText(/给这位员工发消息/);
    await userEvent.type(box, "Ada专属草稿");
    await userEvent.click(screen.getByRole("button", { name: /待审批: write/ }));
    expect(await screen.findByRole("heading", { name: "Ben" })).toBeInTheDocument();
    expect(await screen.findByPlaceholderText(/给这位员工发消息/)).toHaveValue("");
    await userEvent.click(screen.getByRole("button", { name: /Ada/ }));
    expect(await screen.findByPlaceholderText(/给这位员工发消息/)).toHaveValue("Ada专属草稿");
  });

  it("执行中展示实时流式输出，待审批展示具体工具名", async () => {
    const snap: Snapshot = {
      ...labSnap(),
      sessions: [
        {
          ...labSnap().sessions[0],
          seat: "awaiting_approval",
          run_id: "r1",
          pending_tool: "uname",
        },
        {
          ...labSnap().sessions[1],
          seat: "working",
          run_id: "r2",
          streaming_text: "正在分析日志片段...",
        },
      ],
    };
    vi.mocked(api.snapshot).mockResolvedValue(snap);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: /Ben/ }));
    expect(await screen.findByText("正在分析日志片段...")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /Ada/ }));
    expect(await screen.findByText(/等待原生审批 · uname/)).toBeInTheDocument();
  });

  it("恢复上班失败时不会卡在到岗中，并展示错误提示", async () => {
    vi.mocked(api.snapshot).mockResolvedValue({
      ...labSnap(),
      sessions: [{ ...labSnap().sessions[0], seat: "error", run_id: "r-err" }],
    });
    vi.mocked(api.clockIn).mockRejectedValue(new Error("CLI 握手超时"));
    render(<App />);
    const btn = await screen.findByRole("button", { name: "恢复上班" });
    await userEvent.click(btn);
    expect(await screen.findByText(/CLI 握手超时/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "恢复上班" })).not.toBeDisabled();
  });

  it("员工档案展示照片、来源、模型、建立时间、负责的工作，并支持编辑与清除故障员工", async () => {
    const faultySnap: Snapshot = {
      ...labSnap(),
      sessions: [
        {
          ...labSnap().sessions[0],
          seat: "error",
          run_id: "r-err",
          last_error: "quota exceeded: retry tomorrow",
          avatar: "sebastian",
          model: "claude-sonnet-4.5",
          role_description: "核心架构与后端开发",
          created_at: 1792650000000,
        },
      ],
    };
    vi.mocked(api.snapshot)
      .mockResolvedValueOnce(faultySnap)
      .mockResolvedValueOnce(faultySnap)
      .mockResolvedValue({ ...labSnap(), sessions: [] });
    vi.mocked(api.updateSessionProfile).mockResolvedValue({
      ...faultySnap.sessions[0],
      role_description: "自动化测试与排障",
      model: "gpt-5.4",
    });
    vi.mocked(api.deleteSession).mockResolvedValue();

    render(<App />);
    const profileRegion = await screen.findByRole("region", { name: "员工档案" });
    expect(profileRegion).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Ada 员工照片" })).toBeInTheDocument();
    expect(screen.getByText("claude-sonnet-4.5")).toBeInTheDocument();
    expect(screen.getAllByText("核心架构与后端开发").length).toBeGreaterThan(0);
    expect(screen.getByText(/故障诊断：quota exceeded/)).toBeInTheDocument();

    // 编辑档案并保存（模型与来源已锁定，不可编辑）
    await userEvent.click(screen.getByRole("button", { name: "编辑档案" }));
    expect(screen.queryByLabelText("档案模型")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("档案来源")).not.toBeInTheDocument();
    const roleInput = screen.getByLabelText("档案职责");
    await userEvent.clear(roleInput);
    await userEvent.type(roleInput, "自动化测试与排障");
    await userEvent.click(screen.getByRole("button", { name: "保存档案" }));
    await waitFor(() =>
      expect(api.updateSessionProfile).toHaveBeenCalledWith(
        "s1",
        expect.objectContaining({
          model: "claude-sonnet-4.5",
          roleDescription: "自动化测试与排障",
        }),
      ),
    );

    // 清除故障员工
    await userEvent.click(screen.getByRole("button", { name: "清除故障员工" }));
    await waitFor(() => expect(api.deleteSession).toHaveBeenCalledWith("s1"));
    expect(await screen.findByText(/还没有员工/)).toBeInTheDocument();
  });

  it("聊天框正确渲染 Markdown 代码块、列表与图片附件", async () => {
    vi.mocked(api.snapshot).mockResolvedValue(labSnap());
    vi.mocked(api.listMessages).mockResolvedValue([
      {
        id: "m-md",
        session_id: "s1",
        run_id: "r1",
        role: "assistant",
        content:
          "# 诊断报告\n\n请检查以下项目：\n- **核心模块** `service.rs`\n\n```rust\nfn main() {}\n```",
        request_id: null,
        created_at: 1792650000000,
        attachments: [
          {
            id: "a-img",
            original_name: "screenshot.png",
            mime: "image/png",
            size: 1024,
            stored_path: "/tmp/screenshot.png",
          },
        ],
      },
    ]);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: /Ada/ }));
    expect(await screen.findByRole("heading", { name: "诊断报告" })).toBeInTheDocument();
    expect(screen.getByText("核心模块").tagName).toBe("STRONG");
    expect(screen.getByText("service.rs").tagName).toBe("CODE");
    expect(screen.getByText("fn main() {}")).toBeInTheDocument();
    const img = await screen.findByAltText("screenshot.png");
    expect(img).toHaveAttribute("src", "data:image/png;base64,iVBORw0KGgo=");
  });

  it("新建员工时仅展示 CLI 探测核实过的可选模型", async () => {
    const probed: Snapshot = {
      ...labSnap(),
      probes: [
        {
          name: "codex",
          available: true,
          detail: "codex-acp 2.2.2",
          adapter_package: null,
          models: [
            { id: "gpt-6.1-sol", label: "6.1 Sol (gpt-6.1-sol)" },
            { id: "gpt-6-astra", label: "6 Astra (gpt-6-astra)" },
          ],
          default_model: "gpt-6.1-sol",
        },
      ],
    };
    vi.mocked(api.snapshot).mockResolvedValue(probed);
    vi.mocked(api.createSession).mockResolvedValue({
      id: "s9",
      team_id: "t1",
      name: "Cara",
      workspace_path: "/tmp/cara",
      provider: "codex",
      cli_session_id: null,
      seat: "idle",
      run_id: null,
      model: "gpt-6-astra",
    });

    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "添加工位" }));
    const modelSelect = await screen.findByLabelText("模型");
    expect(modelSelect).toHaveValue("gpt-6.1-sol");
    await userEvent.selectOptions(modelSelect, "gpt-6-astra");
    await userEvent.click(screen.getByRole("button", { name: "确定" }));
    await waitFor(() =>
      expect(api.createSession).toHaveBeenCalledWith(
        "t1",
        "Cara",
        "",
        "codex",
        expect.objectContaining({ model: "gpt-6-astra" }),
      ),
    );
  });
});
