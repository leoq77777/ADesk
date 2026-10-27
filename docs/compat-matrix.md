# Desk 兼容矩阵

版本：v0.1 · 日期：2026-10-21 · 由 `scripts/m0/run.py` 根据实测生成。

选定 provider：**cursor-cli**。未验证项不得按支持对外宣称。

## 本机探测

- OS: Darwin 25.4.0 / arm64
- Cursor CLI: `~/.local/bin/agent` · `2026.05.01-eea359f` · 认证 `authenticated`
- Gemini CLI: 未安装
- 其他已登录 CLI（未选用）：Claude Code at `/opt/homebrew/bin/claude`

## Cursor CLI 能力

| 能力 | 结论 |
| --- | --- |
| resume | supported |
| streaming | supported |
| image_input | supported |
| file_reference | supported |
| native_approval | supported |
| writable | supported |
| cancel | supported |
| isolation | supported |

## 审批限制

- `agent -p`：启动时用 `--force` / allowlist / sandbox 决定能否执行，不是原进程往返审批。
- `agent acp`：官方存在 `session/request_permission` 阻塞协议；以 R07-ACP 实测为准。本次已观察到该请求。
- `agent -p` 不带 `--force` 时，本机仍写出了 `need-approval.txt`。print 模式不能当成“工位点批准”。
- 交互式 `agent` 的 TTY y/n 不作为 Desk 接入路径。
- R05 两次文字失败已调查：生成器缺 H/E 等字形，复测图实际像素接近 `747AA`，模型读成 `7477AA`，形状两次正确。见 `tests/fixtures/cursor/2026.05.01-eea359f/R05-retest.png`。

## Gemini CLI

本机未安装，整行记 `unverified` / `not_installed`，不静默记通过。

