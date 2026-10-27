# Desk

本地桌面应用：把 CLI agent 会话做成办公室里的员工。主路径是 **ACP**，本阶段不实现 `-p`。

## 使用

1. `source "$HOME/.cargo/env" && npm --prefix frontend run build && cargo run -p desk-app`
2. 先点「检查 CLI」，再「新建团队」/「添加工位」，并给这个员工选一个可用 CLI。工作目录会自动建。
3. 点工位打开聊天；`⌘/Ctrl+Enter` 发送。可拖入文件或粘贴截图。
4. 员工忙碌时默认排队；「停止并修改」会先取消当前轮再插队。
5. 待审批只出现在原生 ACP 授权请求上。关闭窗口进托盘，系统通知或左侧列表可跳回对应工位。
6. 托盘「退出」才真正退出。崩溃恢复后排队会暂停，需点「恢复排队」。

「检查 CLI」会扫描本机已装的 Cursor / Gemini / Codex / Claude / Antigravity，并对每个做 initialize + session/new。个人版 Gemini 已被 Google 切到 Antigravity。假 CLI 只用于开发：`DESK_PROVIDER=fake cargo run -p desk-app`。

已知限制见 [docs/known-limits.md](docs/known-limits.md)。兼容矩阵见 [docs/compat-matrix.md](docs/compat-matrix.md)。

## M3 / M4

桌面壳已接同一套 `App`：三栏（团队 / 办公室 / 聊天）、托盘隐藏、系统通知点击跳转、拖拽与粘贴附件、聊天分页。办公室为原创像素房间；窗口隐藏后停止工位动画。

## M1 / M2

```bash
source "$HOME/.cargo/env"
cargo test --workspace
cargo run -p desk-harness -- --scenario reply --prompt "hello"
cargo run -p desk-harness -- --scenario approve --prompt "uname"
cargo run -p desk-harness -- --demo two
cargo run -p desk-harness -- --demo resume
cargo run -p desk-harness -- --demo attach
cargo run -p desk-harness -- --demo schedule
cargo run -p desk-harness -- --demo approve
cargo run -p desk-harness -- --demo image
cargo run -p desk-harness -- --demo cancel
cargo run -p desk-harness -- --provider cursor --prompt "Reply with exactly the word pong."
cd frontend && npm install && npm test && npm run build && cd ..
cargo run -p desk-app
# 或: cargo tauri dev --manifest-path src-tauri/Cargo.toml
# DESK_PROVIDER=cursor 使用已登录的 Cursor CLI
```
