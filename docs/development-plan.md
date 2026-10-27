# Desk 开发方案

版本：v0.2 · 日期：2026-10-21 · 状态：实施基线改为 ACP 主路径。本阶段不实现 `agent -p` / print 模式。M0 证据见 [兼容矩阵](./compat-matrix.md) 与 [M0 证据](./m0-evidence.md)。

配套文档：[测试方案](./test-plan.md)。本文面向项目开发者和承担实现任务的 coding agent。文中接口、目录和性能预算是设计目标，不表示已经实现或测试通过。

## 1. 产品目标与范围

Desk 是本地桌面应用，将多个 CLI Agent 会话组织为团队，以简约二维像素办公室呈现员工工作状态。用户可以进入团队、选择员工、聊天、发送截图或文件，并通过通知回到对应任务。

首版包含：

- Team、Project/Workspace、Session 的创建、配置和查看。
- 一个真实 CLI 的 ACP 适配器；另有测试用假 ACP CLI。
- 长 ACP 会话：`session/new` 或 `session/load`，用明确 session ID 恢复，不用 latest/continue。
- 不同 session 有限并行、同一 session 串行。
- 流式对话、附件、取消、追加要求、执行历史。
- 等待审批、等待输入、错误和完成通知。
- 本地持久化、应用退出后的状态恢复、托盘运行。
- 原创二维像素办公室、工位状态和聊天侧栏。

首版不包含跨团队通信、远程执行服务、完整终端模拟器、所有 CLI 能力的 GUI 映射、复杂游戏系统，以及应用完全退出后继续执行的独立守护进程。

默认先交付 macOS；Linux/Windows 保留架构兼容性，但不能在未验证时宣称支持。首个真实 CLI 在 M0 按本地可用账号、额度和兼容性选定，候选为 Gemini CLI、Cursor CLI。

## 2. 技术选择

| 部分 | 选择 | 目的 |
| --- | --- | --- |
| 桌面壳 | Tauri 2 | 使用系统 WebView，控制桌面壳开销 |
| 前端 | React + TypeScript + Vite | 办公室、聊天、附件和通知界面 |
| 后端 | Rust + Tokio | 异步子进程、调度、事件和取消 |
| 数据 | SQLite + 本地附件目录 | 会话、执行、消息、通知持久化 |
| 内部通信 | Tauri commands + events | 命令下行、事件上行 |
| 场景 | 静态图层 + CSS/精灵动画 | 首版避免完整游戏引擎 |

后端核心独立为 Rust crate，不依赖 Tauri。先通过测试程序完成闭环，之后 Tauri 只包装同一套应用服务。首版不引入 Node sidecar、HTTP 服务、Redis 或外部消息队列。

## 3. 架构与执行链路

```text
React 界面 / 后端测试程序
         │ commands
         ▼
应用服务：会话、消息、附件、审批、通知
         │
         ▼
调度器 → 执行器 → ACP 客户端 → provider ACP 子进程
         ▲                         │ JSON-RPC NDJSON / stderr
         └──── 规范化事件与状态归并 ─┘
                    │
              SQLite + 事件订阅
                    │
              GUI / 系统通知
```

一个员工对应一条长 ACP 会话（一个子进程）。一条用户消息是 `session/prompt`，不是再拉起 `-p`。

一次提交的处理顺序：

1. 校验 session、工作目录和附件能力；将附件快照保存至托管目录。
2. 在事务内保存用户消息与 queued Run；提交失败则不向 ACP 发 prompt，清理无引用附件。
3. 调度器获取全局槽位和 session 独占权，将 Run 标记为 starting。
4. 若该员工还没有 ACP 进程，适配器按参数数组启动 `agent acp` / `gemini --acp` / 假 CLI；已有进程则复用。
5. `initialize`（必要时 `authenticate`）后 `session/new` 或 `session/load`；禁止 latest/continue。
6. 发送 `session/prompt`；异步消费 stdout 上的 JSON-RPC，持续排空 stderr，监听 `session/update`、`session/request_permission` 和取消。
7. 审批请求阻塞原进程：Desk 发 JSON-RPC 响应后同一会话继续。取消发 `session/cancel`，再按需回收进程。
8. 归并 prompt 结果与进程状态，原子保存 Run 终态与对应通知，发布事件。

事件等待不会重复调用模型，也不以定时轮询 CLI 获取状态。ACP 进程在会话期间保持存活。stdout/stderr 必须持续排空，避免管道背压阻塞 CLI。本阶段不实现 print/headless `-p` 适配器。

## 4. 领域模型

| 实体 | 关键字段与职责 |
| --- | --- |
| Team | id、name；逻辑分组，对应一个办公室区域 |
| Project | id、name、root_path；项目登记信息 |
| Workspace | id、project_id、path、isolation_mode；实际执行目录 |
| Session | id、team_id、workspace_id、provider、cli_session_id、name；一个员工的持续会话 |
| Run | id、session_id、message_id、source、priority、execution_state、business_state、时间与错误信息；一次提交对应的执行 |
| Message | id、session_id、run_id、role、content、attachments；可追溯对话 |
| Attachment | id、original_name、mime、size、managed_path；提交时的文件快照 |
| Approval | id、run_id、provider_request_id、status、可恢复方式；授权请求 |
| Notification | id、session_id、run_id、kind、priority、summary、read_at、dedupe_key；持久化提醒 |

员工对应 Session，员工当前工作对应 Run；CLI Session ID 是外部恢复句柄，不作为 Desk 的主键。恢复必须使用指定 ID，不使用 latest/continue 一类可能串线的默认选择。

同一项目可以有多个 session。写代码的 session 默认使用预先准备好的独立 Git worktree；MVP 先支持绑定已有 worktree，自动创建与清理可以后续补充。共享写目录作为显式选项，并提示冲突可能性。非 Git 项目允许使用独立普通目录。

## 5. 执行状态与 agent 自报状态

### 5.1 执行状态由 Desk 维护

```text
queued → starting → running → succeeded / failed
   │         │         │
   └─────────┴─────────┴→ cancelling → cancelled
starting/running/cancelling → interrupted（应用异常中断后恢复）
```

- 启动失败直接进入 failed。
- queued Run 可直接 cancelled，无须创建进程。
- 运行中的取消需终止受管理进程及相关子进程，回收退出状态后再释放 session 独占权。
- 不能确认进程停止时，保持 session 受阻，不能冒险启动下一轮。
- 正常退出不自动表示业务任务完成；成功条件由适配器结合该 CLI 的结束协议确定。
- 协议声明成功而退出码非零时，执行结果记 failed，保留业务声明和诊断。

### 5.2 业务状态优先由 agent 声明

业务状态：unknown、replied、completed、awaiting_input、awaiting_approval、blocked。它们不替代执行状态。

首版在提示约定中要求 agent 输出版本化状态块；适配器仅从完整的 assistant 消息中识别，不从工具输出或用户文本识别。例如：

```json
{"desk_status_version":1,"status":"completed","summary":"已完成登录页面并执行检查"}
```

具体定界格式在 M0 确定并用 fixture 固化；该约定是尽力支持，不是可信控制协议。缺失、格式错误或未知版本降级为 unknown，保留原文；正常结束时通知“本轮已结束”。已回复和已完成分别显示，completed 文案注明来源为 agent 自报，不表示 Desk 独立验证。

awaiting_approval 的自报只产生待处理提醒；只有原生 CLI 授权事件及有效请求标识，才能产生可点击执行的批准操作。任何自报文本都不能授予权限或执行系统命令。

## 6. 调度与打断

- 全局并发上限默认 2，可配置为 1 或更高。
- 同一 session 最多一个活动 Run，包括 starting、running、cancelling，以及原进程仍在等待审批的 Run。
- 每个 session 按提交顺序处理；手动优先只在各 session 的队首候选之间生效，避免重排同一对话。
- 手动候选优先于自动候选，同级按入队顺序；从可执行候选中选择，不因某个受阻 session 阻塞整个队列。
- 不抢占已启动任务；自动任务可能长期等待，首版展示等待状态，不承诺公平时限。
- 等待审批的原进程继续占用并发槽位；并发上限 2 时另一个槽位仍可工作。全部槽位被占用时，需要用户处理审批或取消，不偷偷超额启动。

发送时若员工忙碌，默认追加到队列。用户选择“停止并修改”时，先取消当前 Run；确认停止后，将新要求排在该 session 其他未启动消息之前，界面显示更新后的顺序。未确认停止则不启动替代调用。已产生的文件修改不会回滚。

## 7. CLI 适配器契约

适配器负责差异，不承担持久化或全局调度。最小职责：

- probe：检查路径、版本、认证可用性和 ACP 能力。
- spawn_or_attach：启动或复用 ACP 子进程（参数数组，不用 shell 拼接）。
- session_new / session_load / prompt / cancel：映射到 ACP 方法。
- respond_approval：回答 `session/request_permission`，这是主路径而不是可选项。

能力字段至少包含：resume、streaming、image_input、file_reference、native_approval。每项能力记录 supported / unsupported / unverified，不能把未验证当成支持。

统一事件至少包含：session_bound、assistant_delta、assistant_message、activity、business_status、approval_requested、diagnostic、provider_result。事件携带 session_id、run_id、顺序号和时间；原始 CLI 信息仅供诊断。

调用要求：参数数组启动 ACP 进程，禁止用 shell 拼接用户文本；prompt 作为 JSON-RPC 参数传递，不经 argv 拼接；固定会话 cwd；限制日志与内存缓冲；错误信息区分未安装、认证失效、额度/网络、协议错误、启动失败和退出失败。默认不自动重试可能产生文件修改的 Run，避免重复副作用。

超时策略：默认不因长时间无输出而取消任务；总执行时限和审批等待时限可配置，默认关闭。触发已配置时限时进入取消流程并记录 timeout 原因。优雅停止默认等待 5 秒后升级终止；升级后再等待 5 秒仍无法确认退出则标记受阻，不释放该 session 独占权。具体进程终止操作由平台执行器实现。

首个适配器准入条件：ACP 新建/恢复会话、流式 update、原进程 `session/request_permission`、图片/文件读取、取消。M0 已确认 Cursor `agent acp` 可发出审批请求。兼容矩阵记录 OS、可执行文件路径、CLI 版本、认证方式、能力证据；CLI 升级后重新运行冒烟测试。`-p` 不作为本阶段实现或准入路径。

## 8. 附件、审批与通知

### 附件

支持文件选择、拖拽与粘贴截图；首版不要求自行实现系统级截图工具。发送前复制为不可变快照，保存原文件名、类型、大小和托管路径。默认单文件上限 20 MiB、单条消息总量上限 50 MiB，作为可配置初始限制。

适配器将快照转换为 CLI 能识别的原生附件输入或路径引用，并确认工作目录和权限允许读取。仅“路径存在”不视为附件支持通过。不支持的类型在入队前提示。附件清理不得删除仍被历史消息引用的文件。

### 审批

区分三类：原生事件且可在原进程恢复；原生事件但需 CLI 专属处理；仅 agent 声明等待审批。后两类提供详情和取消入口，不展示虚假的批准按钮。首版不靠重新 resume 猜测恢复一个未完成工具调用。

批准/拒绝前校验 Run 仍活动、请求仍 pending；同一请求只处理一次，过期操作返回明确结果。拒绝后的行为按适配器能力定义并测试。

### 通知

优先级：等待审批 > 错误/受阻 > 等待输入 > 普通回复/完成。同级按创建时间，持久化已读状态；高优先级不等于执行队列抢占。默认不使用阻塞模态框，采用系统通知、应用内通知列表与工位标记。

dedupe_key 使用 run_id、事件种类、审批请求 ID 或终态版本；数据库唯一约束保证重复事件不重复创建通知。系统弹出是尽力投递：重启恢复应用内通知，但不重放所有旧系统弹窗。点击通知通过内部标识定位 team/session/run，不接受任意外部命令。

## 9. 持久化与应用生命周期

保存对话、Run 状态、业务声明、审批、通知和附件引用；activity、动画、可重建的文字增量不要求逐条保存。流式消息定期批量落盘，终态前完成最终写入。事件消费者慢时合并增量，关键状态先持久化再通知，不能因 GUI 断开停止读取 CLI。

关闭窗口默认隐藏至托盘，执行继续；真正退出时停止接收新任务，取消活动 Run 并等待回收。无法在退出期限内完成的执行在下次启动时标为 interrupted，不自动重跑。

异常退出后，将遗留 starting/running/cancelling 标记 interrupted，pending 审批标为失效；原 queued Run 保留但暂停派发，待用户明确继续。若无法确认旧进程树已停止，相应 session 保持受阻，提示人工处理。单实例运行防止两个 Desk 同时调度同一数据库。首版不依赖仅凭 PID 杀死未知进程。

前端每次启动先订阅事件并缓冲，再读取带 revision 的状态快照，随后应用更新的事件；发现缺口或接收滞后则重新取快照。通知和历史可以重新查询，无须持久化全部实时事件。

## 10. 后端接口与界面

| 应用接口 | 返回或行为 |
| --- | --- |
| create_team / register_project / create_session | 创建团队、登记项目、绑定工作目录与 provider |
| get_snapshot / list_messages | 获取状态与分页历史 |
| stage_attachment | 创建附件快照与元数据 |
| submit_message | 接收 request_id，返回 message_id、run_id；重复 request_id 返回原结果 |
| cancel_run / stop_and_replace | 取消或停止后改派，明确报告是否已停止 |
| resolve_approval | 校验审批能力、请求有效性和操作去重 |
| list_notifications / mark_read | 查询提醒与持久化已读状态 |
| subscribe_events | 订阅结构化更新，不暴露原始进程控制给前端 |

上述是概念接口，具体 Rust 类型在 M1 固化。Tauri 层负责参数校验和调用包装，核心业务保持可由测试程序直接调用。

submit_message 的 request_id 在提交方重试时保持不变；后端保存请求内容指纹，同 ID 同内容返回原结果，同 ID 不同内容返回冲突错误。幂等提交不代表 CLI 的外部文件操作可以自动重试。

界面采用左侧团队导航、中间办公室、右侧员工聊天。点击工位直接打开聊天，保留团队视野。工位显示排队、工作、待审批、等待输入、错误、空闲；详细信息来自真实事件，无法识别时只显示“执行中”。

美术使用原创俯视二维像素素材，借鉴温暖配色、简洁空间与小人物表现，不复制现成素材。房间大部分静态；员工仅用少量帧动画。隐藏窗口后停止场景动画；没有活动动画时不维持连续渲染循环。聊天历史分页或虚拟列表，避免长会话持续增加 DOM 和内存。

## 11. 建议目录

```text
crates/desk-core/src/      # ACP 编解码/客户端、model、scheduler、runner、storage
crates/desk-adapters/src/  # 各 provider 的 ACP 启动参数（M2 接 Cursor）
crates/desk-harness/       # 无 GUI 的后端验收程序
crates/desk-fake-cli/      # 可控的假 ACP agent
src-tauri/                # Tauri commands、托盘、系统通知
frontend/                 # 办公室、聊天、通知和设置
tests/fixtures/           # 按 provider/版本区分的脱敏输出
docs/                     # 开发方案、测试方案、兼容矩阵与验收证据
```

## 12. 实施阶段与退出条件

| 阶段 | 交付物 | 退出条件 |
| --- | --- | --- |
| M0 CLI 验证 | 首个 provider 选择、版本矩阵、真实输出样本、审批限制说明 | 新建/恢复/附件/取消证据齐全，确认可用集成路径 |
| M1 核心与假 CLI | ACP 编解码、ACP 客户端、假 ACP CLI、一轮 prompt/审批/取消 | U03/U04/U07 与 P01/P06/P11/P12 的 ACP 对应项通过；harness 无 GUI 完成一轮消息 |
| M2 后端闭环 | 调度、SQLite、附件、通知、恢复、真实适配器、harness | 两员工并行、恢复、附件、取消、通知和异常恢复验收通过 |
| M3 桌面功能 | Tauri 包装、团队导航、聊天、附件、托盘和通知跳转 | 使用同一后端接口完成桌面端到端验收 |
| M4 办公室与资源优化 | 像素房间、员工状态动画、性能报告 | 状态准确，隐藏窗口无持续动画，满足约定资源预算 |
| M5 首版交付 | 安装包、使用说明、兼容矩阵、已知限制 | 测试方案全部发布门槛满足 |

进入前端开发前必须完成 M2，不能以假 CLI 测试替代真实 CLI 证据。第二个 provider 在首版闭环稳定后接入，并复用相同契约测试。

## 13. 资源预算与待验证事项

初始工程预算（需在 M0 记录参考机器，M4 实测后确认）：无任务且窗口隐藏时 Desk 自身进程树平均 CPU < 1 个逻辑核的 1%，常驻内存 < 200 MiB；窗口可见、10 个工位活动时目标 30 FPS；后端收到终态到保存通知 p95 ≤ 500 ms。内存预算包括 Desk 与其 WebView，不包括外部 agent CLI；另报告含 CLI 的整体占用，不能混用统计口径。

不承诺 CLI 启动延迟或系统通知实际出现时间；分别测量并报告。所有性能结果记录 OS、硬件、构建模式、CLI 版本和测试场景。

实施前仍需验证：首个 CLI 与版本；附件原生输入方式；审批可恢复程度；macOS 进程树取消实现；CLI 自动升级带来的协议变化。发现能力缺口时更新兼容矩阵和对应测试，不静默模拟支持。

## 14. 官方参考

- [Tauri 架构](https://v2.tauri.app/concept/architecture/)
- [Tokio 异步进程](https://docs.rs/tokio/latest/tokio/process/index.html)
- [Cursor ACP](https://cursor.com/docs/cli/acp)
- [Gemini CLI ACP](https://geminicli.com/docs/cli/acp-mode/)
- [Agent Client Protocol](https://agentclientprotocol.com/)

官方资料用于确认能力方向；具体命令、格式和行为以 M0 针对所选安装版本取得的测试证据为准。
