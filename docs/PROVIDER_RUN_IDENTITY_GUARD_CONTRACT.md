# Provider Run 历史身份门禁

## 目的与范围

在可信宿主串行使用同一组 caller-owned 日志时，拒绝复用已经出现的运行身份，避免先发送 provider 请求、再由审计或状态投影发现重复。

本契约限定于 OpenAI-compatible foreground、Anthropic foreground，以及两协议共用的 bounded Markdown runner。不是完整恢复协议，也不是跨进程 exactly-once 保证；当前验证证据见 `PHASE_1_VALIDATION_PLAN.md`。

## 启动前检查

foreground 在协议校验之后、工具日志打开与 controller 启动之前检查；Markdown 保留既有全部路径安全检查在先，再检查身份。拒绝时不新增 `loop_started`、模型调用或工具动作，也不修改目标 Markdown 和业务审计日志。ModelCallStore 扫描可能创建锁文件，不承诺零磁盘写入。

以下任一事实占用本次身份：

| 来源 | 拒绝条件 |
|---|---|
| 任意 EventEnvelope | 顶层 `loop_run_id` 或 `task_run_id` 与请求对应身份相同 |
| 明确类型的事件聚合 | `aggregate_type=loop_run/task_run` 且 `aggregate_id` 与对应身份相同，即使可选 lineage 字段为空 |
| 成功或失败 ModelCallRecord | `task_run_id` 相同，或 `model_call_id` 精确等于下一次 `call-{loop_run_id}-step-1` |

不按完成态过滤事件：只有 started 的中断残留也必须拒绝。不从 payload、event ID、correlation ID 或字符串前缀猜身份；其他类型聚合的同名 ID 不构成冲突。

历史按行扫描，不把整份日志收集到 Vec，不循环从头分页扫描。模型成功/失败记录仍执行既有合约校验。逐行扫描不是单行字节硬上限，文件大小和锁等待仍无新增上限。

## 拒绝行为

- 身份冲突返回固定 `DuplicateRunIdentity`，不携带历史身份或正文。
- guard 中读取、JSON 解码或模型记录合约校验失败，返回固定 `RunHistoryUnavailable`；不保留原 source，不向 Display/Debug/source 链传播历史正文。
- 不自动换身份、不删除历史、不从 usage 推断恢复账本，不调用凭据 resolver 或发送模型请求。
- 已报告 token 门禁关闭、零额度或正额度均不能绕过身份检查。

调用者在进入 runner 前执行的 `EventLog::open()` 仍可能先行失败，其诊断不在此 guard 的固定错误覆盖范围。协议和 Markdown 路径错误也保留既有优先级。

## 明确不保证的边界

- EventLog 没有跨进程原子身份认领；检查与启动之间存在 TOCTOU。ModelCallStore 的单次读取锁不覆盖整个运行。
- 删除、截断、更换路径或丢失日志会失去历史证据；仅保留孤立 tool/checkpoint/trace/side-effect 文件不在本片检查范围。可信调用者必须保留原日志集合。
- provider 已发送、审计未落盘时崩溃，远端是否执行或计费仍未知；不得把缺少完成记录当成可安全自动重发的依据。
- 每次 runner 仍新建 TaskRun，本片不恢复 TaskRun/Attempt/位置或累计预算，不提供跨运行 token 配额。
- 本身份门禁切片本身不解决 StateStore 的跨 loop 同名步骤问题；后续独立投影根修复见 `LOOP_SCOPED_STEP_PROJECTION_CONTRACT.md`。两者合并仍不等于多次 CLI 运行、live provider 和完整任务恢复已全部验收。

## 验收入口

`tests/provider_run_identity.rs` 使用合成事件、模型审计、fake transport 和 unavailable 计数 resolver，验证重复身份的零发送与业务文件不变、新身份可用和损坏历史安全失败。协议及路径检查先于 guard 的顺序由源码审核确认；既有路径安全回归保持通过，但没有新增组合优先级测试。它不是 live provider、并发原子认领或完整恢复验收。最终测试数量、命令与结果以验证计划中的已执行证据为准。
