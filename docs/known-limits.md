# 已知限制

首版范围与未完成项。未验证能力不得按支持对外宣称。

## Provider

- 本阶段走 ACP。检查会扫描本机已装的预置名单（Cursor / Gemini / Codex / Claude / Antigravity），并对每个做 initialize + session/new。
- 个人版 Gemini Code Assist 已被 Google 停用（报错会指向 Antigravity）。`agy` 没有 `--acp`，要用独立的 `agy_acp_server`。
- Codex / Claude 本体不讲 ACP。检查后可点「安装 ACP」装目录里登记过的适配器，再自动复检。
- 系统自带的 `/usr/sbin/gpt` 不是 OpenAI CLI，不会探测。
- 假 CLI 只在 `DESK_PROVIDER=fake` 时出现。不实现 `-p`。
- 工作目录写权限以兼容矩阵为准；共享写目录可能互相覆盖，界面会绑定用户选中的目录，不会自动建 worktree。

## 审批

- 只有原生 `session/request_permission` 才出现批准/拒绝。agent 自报文案不会变成可点批准。
- 系统通知是尽力投递：权限被拒时应用内通知和工位灯仍可用。
- macOS 上通知点击依赖系统是否回报 tap；应用内通知列表始终可跳转。

## 桌面

- 关闭窗口默认进托盘，任务继续。托盘「退出」才停应用。
- 像素办公室是 CSS 静态房间 + 少量工位帧动画；隐藏窗口后动画暂停。
- N01–N06 资源预算见 [M4 测量](./m4-perf.md)，尚未实测，不能当作已通过。
- 安装包（M5）尚未打。当前入口是 `cargo run -p desk-app`。

## 未做

- R08 / R09 真实 CLI 冒烟尚未补报告。
- `crates/desk-adapters` 尚未从 core 拆出。
- 第二个 provider、跨团队通信、独立守护进程都不在首版。
