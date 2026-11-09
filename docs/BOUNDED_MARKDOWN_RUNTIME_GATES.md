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

为保留可信 library 调用契约，audit/runtime 可与 workspace 同目录，但不能与授权
可编辑文件或其它 store/lock 发生别名。未来用户 CLI 的 runtime 外置要求须单独明确，
不能把这里的 library 检查说成整个 workspace 与 runtime 已隔离。

## 已知限制与后续安全门槛

不执行 workspace edit **不等于零持久化副作用**：进入 controller 后，拒绝/失败仍
会尝试写入 event/model 等审计。公开纯预检的无写入承诺不适用于实际 run。

这些检查是离散时点的 metadata 快照，不构成原子 OS 隔离；共享 event/model/audit
stores 中仍有按路径 append 的实现。如果同 OS 用户进程或不可信 callback 在检查
后替换 runtime 路径，后续失败审计不能据此获得端到端防别名保证。本片没有修改
所有共享 store 的写入机制，不能声称可抵御该威胁。**未来接入 live 写入 CLI 前必须
重新审计并解决实际写入端的路径安全边界**，不能用这份 preflight 作为放行证明。

Unix 身份门禁之外的平台 fail-closed；本轮不宣称 Windows/macOS 实测。可写性、
历史内容完整性、完整 TaskRun/Attempt/预算恢复、快照/自动补偿、后台调度、真实
provider E2E 和 Phase 1 整体验收均未由本片完成。

验证入口：`cargo test --locked --offline --test provider_markdown_gates --test provider_markdown_runner -- --test-threads=1`。
实际结果及独立审核见 `PHASE_1_VALIDATION_PLAN.md`，提交/集成收据另记持久任务台账。
