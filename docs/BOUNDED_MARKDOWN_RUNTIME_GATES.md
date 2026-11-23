# 受控 Markdown runner 的运行前门禁

2026-11-09。本片是已有单动作 Markdown library runner 的安全底座，不新增 CLI，
不放开通用文件/Bash、多步写入、自动重试、fallback 或审批策略。

## 调用顺序

可信宿主构造 `BoundedMarkdownRunConfig` 和 `ProviderRunRequest`，然后调用：

```rust,ignore
let prepared = ForegroundProviderRun::prepare_markdown(&config, request, run)?;
let paths = prepared.validate_runtime_paths(&events_path, &model_calls_path, Some(&state_path))?;
// 只有前置检查通过后，调用方才应打开 EventLog / ModelCallStore。
let events = EventLog::open(events_path)?;
let calls = ModelCallStore::open(model_calls_path)?;
let outcome = prepared.run_with_resolver(resolver, events, calls, progress_sink)?;
// paths 独立于已消费的 prepared，可在宿主后续写入 state 前重新检查。
paths.revalidate()?;
```

示例说明 API 顺序，不是完整程序或 live 验收记录。`state_path` 仅在宿主传入时检查，
runner 自身不打开或写入 StateStore。返回的 check 不是写入许可，也不执行状态投影。

| 检查点 | 内容 |
|---|---|
| prepare | 冻结 canonical workspace 与 Unix device/inode；授权目标为 1..256 个不同的、现存单链接普通 Markdown 文件 |
| 公开纯路径检查 | 十条实际 store/lock 目的地，加可选 state；父目录存在、无 traversal/symlink、类型正确、无规范路径/inode 别名或额外硬链接 |
| 实际运行前 | 重验冻结 scope/运行路径；沿用历史身份拒绝，之后才打开 tool store/进入 controller |
| 模型发送前 | 再检查一次，覆盖 progress callback 后工作区被替换的情况 |
| 模型返回后 | 成功与错误响应均复核；成功响应若门禁失败，拒绝接受 action，并保留已知安全 usage/hash 证据；已有 provider 错误仍保留原错误证据 |

十条目的地是：events、checkpoint、trace、side-effect、tool calls、tool lock，以及
model success/failure 和二者的 lock。model 四路径由 `ModelCallStore::storage_paths_for`
推导；不虚构 event/checkpoint/trace/ledger 的锁。

公开检查只读取文件系统元数据，不打开 stores、不读取正文、不创建文件/目录、不解析
凭据、不联网，不预留运行身份。合法文件内容修改不会使 check 自动失效；文件内容
一致性仍由原有 expected-content 检查和写后验证负责。本片不冻结整棵目录树的内容。

## 执行与协议边界

`run_with_resolver` 依据已冻结的 profile 分派既有 OpenAI-compatible 或 Anthropic
HTTPS transport。非法路径/工作区或历史身份在 client 创建前拒绝；零 reported-token
预算在实际模型调用前停止，不解析凭据。没有新增凭据存储/环境扫描，也不把真实
认证材料放进模型上下文。合成拒绝型 resolver 测试不代表真实账号可用。

模型仍只能提交一次 `edit_markdown`，必须在授权范围内，并带同文件的
`markdown_link_check`。沿用 PolicyGate、显式宿主 approval、checkpoint、side-effect
ledger 和写后验证；拒绝不扩大授权，验证失败不伪报完成或自动 rollback。

2026-11-23 读取门禁增量：controller 对 `expected`、`replacement` 各新增 2 MiB
字节上限，在 snapshot capture、checkpoint 和 mutation 前拒绝超限输入。
前述拒绝仍发生在 policy/approval 处理之后，不承诺零审计或不消费已有 Once 审批。
checkpoint hash 读取也限 2 MiB，保留 `SHA256(relative_path + NUL + file bytes)` 格式；先解析
workspace 根别名，再用 pinned parent 和实际 regular-file FD 读取，不跟随打开过程中的
symlink，保留普通 hardlink 兼容。Markdown tool verifier 在同一 2 MiB 预算内完整读取，
超限或无效 UTF-8 返回 `OutputCollectionFailed`，不验证截断前缀后报成功。`read_file`
仍是 16 KiB 展示截断；独立 snapshot 的 1 MiB 限额不变。这不是所有 editor 或工具的
统一限额，也不限制模型响应解析前的内存；实现和验收状态见本片外置收据。

为保留可信 library 调用契约，audit/runtime 可与 workspace 同目录，但不能与授权
可编辑文件或其它 store/lock 发生别名。未来用户 CLI 的 runtime 外置要求须单独明确，
不能把这里的 library 检查说成整个 workspace 与 runtime 已隔离。

## 已知限制与后续安全门槛

不执行 workspace edit **不等于零持久化副作用**：进入 controller 后，拒绝/失败仍
会尝试写入 event/model 等审计。公开纯预检的无写入承诺不适用于实际 run。

这些检查是离散时点的 metadata 快照，不构成原子 OS 隔离。原门禁切片未修改共享
store I/O；其后各 store、workspace 和 snapshot 已分片接入实际 FD/pinned parent 的
单次安全操作，不能再将旧的按路径 append 描述当作当前全部实现。上述增量仍不锁定
整个 run 的目录身份或读取期间的字节，不提供跨阶段 hash/edit/verify 原子性、writer
CAS、自动重试或恢复。**未来接入 live 写入 CLI 前仍须复审实际执行链并完成获准验收**，
不能用这份 preflight 或单次 FD 安全作为端到端放行证明。

Unix 身份门禁之外的平台 fail-closed；本轮不宣称 Windows/macOS 实测。可写性、
历史内容完整性、完整 TaskRun/Attempt/预算恢复、快照/自动补偿、后台调度、真实
provider E2E 和 Phase 1 整体验收均未由本片完成。

验证入口：`cargo test --locked --offline --test provider_markdown_gates --test provider_markdown_runner -- --test-threads=1`。
实际结果及独立审核见 `PHASE_1_VALIDATION_PLAN.md`，提交/集成收据另记持久任务台账。
