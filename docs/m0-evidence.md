# Desk M0 验收证据

记录时间：2026-10-21T23:40:02Z
对应：[开发方案](./development-plan.md) · [测试方案](./test-plan.md) · [兼容矩阵](./compat-matrix.md)

本文件不是发布证明。状态只能是：通过 / 失败 / 受限 / 未执行 / 环境故障。

## 用例

| ID | 状态 | 实际 |
| --- | --- | --- |
| R01 | 通过 | session_id=1a0c6ec1-9acd-4e4e-8eaf-5786bf36b81f; reply contains pong; exit=0 |
| R02 | 通过 | resume=5ddf5634-a18a-41b2-bd01-e15c005d730a; reply='M0R02-G02OLJIX'; used_latest=False |
| R03 | 通过 | sid_a=55945c8e-9088-41ce-9d19-54ecae872427; sid_b=81cb709b-e244-4342-9dde-5ad0b75c63f5; crossed=False; a='M0A-QEPGNP'; b='M0B-28KC89' |
| R04 | 通过 | token_in_reply=True; reply='我先读取 `secret-note.txt`，然后只返回文件原文。T4-ek8hz6mvj4'; token_in_argv=False |
| R05-retest | 失败 | text_ok=False; shape_ok=True; reply='我先查看 `probe.png` 里实际画了什么，再按你要求的 JSON 格式回报文字和图形数量。```json\n{"text":"7477AA","shapes":[{"kind":"circle","count":1},{"kind":"triangle","count":1}]}\n```' |
| R05 | 失败 | text_ok=False; shape_ok=True; reply='我先查看 `probe.png` 里实际画出的文字和图形，再按要求整理成 JSON。```json\n{"text":"000002","shapes":[{"kind":"square","count":1},{"kind":"triangle","count":1}]}\n```' |
| R06 | 通过 | alive=True; sid=9a53d0e3-6452-4e01-8953-c0d4bc6f5e6c; rollback_claim=False |
| R07-ACP | 通过 | permission_seen=True; session_id=0c7baf49-327f-45a6-83c3-d25307b0ea4b; exit=None |
| R07 | 通过 | print_mode_without_force wrote_file=True; timed_out_or_cancelled=False; exit=0 |
| R10 | 通过 | wrote=True; exit=0 |

## M0 出口判断

- 通过/受限/未执行/环境故障计数：通过 8 / 全部 10
- 方案要求的 R01–R07、R10 阻断项：无
- 退出条件：新建/恢复/附件/取消证据齐全，并确认可用集成路径。
- **当前结论：有条件通过。** R05/R05-retest 文字断言失败，但是测试图缺字形；形状两次正确且存在 readToolCall。图片能力记 supported，用例本身保持失败。

脱敏 fixture 在 `tests/fixtures/`，按 provider/版本分目录。

## R05 调查

两次文字断言失败，不覆盖原记录。原因是测试图生成器只定义了 `0-9/A-C`，`H`/`E` 等字母被画成 `A`。
复测期望 `747HE`，落盘 PNG 实际接近 `747AA`；模型回复 `7477AA`，形状 circle+triangle 正确，并有 `readToolCall` 读取 `probe.png`。
因此图片读取能力成立，R05 用例因 oracle 无效保持失败。生成器已补全字形，未再消耗第三次真实额度。

