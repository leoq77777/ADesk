# 后端验收记录

日期：2026-10-21 · OS：Darwin 25.4.0 arm64 · 构建：debug workspace · provider：fake CLI（除另行标明）

`cargo test --workspace`：38 通过 / 0 失败  
`desk-harness` 全部 `--demo` 与 `reply` / `approve` 场景：通过

| ID | 结果 | 证据 |
| --- | --- | --- |
| U01 | 通过 | `roundtrip::u01_session_load_keeps_id` |
| U02 | 通过 | `roundtrip::u02_prompt_keeps_quotes_and_cjk` |
| U03 / U04 | 通过 | `desk-core` codec 分块 / EOF / 畸形 / 超长 |
| U07 | 通过 | `client::u07_permission_only_from_request_permission` |
| U09 / U10 / U11 | 通过 | scheduler 串行、手动优先、取消幂等 |
| U12 | 通过 | `stop_and_replace_runs_before_older_queue` |
| U15 | 通过 | `request_id_is_idempotent` |
| P01 | 通过 | `p01_reply_round` + harness `reply` |
| P06 | 通过 | `p06_cancel_hanging_prompt` + `--demo cancel` |
| P09 | 通过 | 两槽挂起、第三排队后放行 |
| P11 / P12 | 通过 | 审批阻塞后继续；`--demo approve` `approvals=1` |
| P13 | 通过 | 附件读快照不读源文件；`stage_bytes` 同等 |
| P16 / P19 | 通过 | 崩溃标 interrupted 并暂停队列；第二实例 flock |
| R 假 CLI 替代 | 通过 | `--demo two/resume/attach/image/cancel` |
| F 桌面原生点选 | 受阻 | Computer Use 只能抓到被挡的占位窗，无法代替人工看 Desk 窗口 |
| F01 / F04 / F06 | 通过 | `frontend` vitest：空办公室、建团队、草稿隔离、发送、通知跳转 |
| CLI probe | 通过 | `probe_fake_is_available` / `probe_missing_bin_blocks_create`；vitest：先检查再选 CLI 建员工 |
| N01–N06 | 未实测 | 见 `docs/m4-perf.md` |
| R08 / R09 | 未执行 | 需真实 Cursor 额度，未在本轮消耗 |
