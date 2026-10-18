# Phase 1 Implementation Scope

## 文档目的

本文件用于把 `steerBox` 第一阶段实现范围收敛成可执行边界。

它不替代已有讨论稿。已有文档继续保存长期设计、参考资料和未来能力；本文件只回答第一阶段真正要做什么、暂不做什么、哪些只做占位，以及如何判断第一阶段成功。

## 核心判断

第一阶段不应该做成只能演示的 demo。

但第一阶段也不应该一次性实现完整的自适应多模型路由、插件市场、向量记忆、图记忆、dream learning、自动进化、四个专业方向的完整闭环和通用 Manus-like agent。

更合理的目标是：

```text
用最小但真实可扩展的 harness core，
跑通一个软件开发智能体垂直切片，
同时为持续运维、安全评估与漏洞研究、安全运营和长期演进能力保留清晰接口。
```

换句话说：

- 第一阶段要验证 `steerBox` 的核心控制模型是否成立
- 第一阶段优先服务软件开发智能体
- 其他三个方向先保留 track 边界、授权/policy 边界和 side-effect 边界
- 通用全功能智能体暂不作为第一阶段目标

### 当前可执行切片

已实现的本地切片包括 Rust/Cargo 骨架、核心领域对象、`RuntimeConfig`、`/temp/steerbox/<project_ref>` 路径约束、JSONL `EventLog`、只读 observation loop、`LocalFixtureModelAdapter`，以及接入 `LoopController` 的 `ToolRegistry` / `PolicyGate`。受控只读执行器支持固定 argv 的 git status、固定字符串搜索、有界文件读取和 Markdown 链接检查；结构化动作不接受任意 shell 字符串。受审批的 Markdown 修改另有 workspace-edit 路径，记录修改前后 hash、验证结果和副作用；开发 CLI 生成 `RunSummary`，验证失败返回非零。

controller 的 `execute_workspace_edit_action()` 必须先配置 `with_checkpoint_store(...)` 与 `with_audit_stores(trace_path, side_effect_path)`；policy Allow/显式 approval 不替代审计前置条件。接口先处理生命周期和待处理命令，缺 store、Paused/terminal 或 mutation 前审计错误时不调用写执行器。低风险自主模式仍可不询问人类；bounded Markdown controller 现在可通过 `with_snapshot_store(...)` opt in，在 mutation 前 capture，并由 trusted caller 显式调用 restore；默认路径仍不自动 capture，verification failure 不自动 rollback，也不提供跨进程恢复或对所有 writer/CLI 的统一接线。

OpenAI-compatible 与 Anthropic non-stream adapter、注入式 HTTP transport、本地 credential resolver 边界、冻结的 `ResolvedPromptPack` 和 opt-in `ModelCallStore` 审计已有 fake/localhost 测试，不等于完成获准的真实帐号/live provider 验收。`UserCommandInbox` 在 controller 安全边界应用 pause/resume/cancel，但同步模型调用期间不读取命令。单个 bounded Markdown 文件已有 snapshot primitive，并已接入 controller 的 opt-in capture/显式 restore：restore 需要 trusted caller approval、controller-run binding、snapshot integrity 和 expected current content hash；正文仍在 workspace 外私有 record，不进入公共事件/提示上下文。该接线不是自动 rollback、统一恢复/replay、后台 worker / 运行中接管、修订恢复、TaskRun/Attempt 或进程/外部副作用恢复；任意文件写入 / Bash 和完整记忆仍未完成，当前不能称为完整可用的模型连接型开发 Agent。

### 长期运行目标与 Phase 1 实现边界

模型路径已有本地规则路由到 `ModelAdapterRegistry` 的 LoopController 接线，EventLog 保存路由及关联的调用事件；High risk 或 Restricted privacy 路由在没有审批 resolver 时拒绝调用。静态 `ResolvedPromptPack`、opt-in provider bridge 和 `ModelCallStore` 审计已有 fake/localhost 请求验证，覆盖成功 resolved 调用的 provider/endpoint/model/prompt 绑定、wire hash、usage/finish_reason 和 fail-closed 路径。opt-in resolved controller 的 adapter-error 分支另写独立 `ModelCallFailureRecord` 和 failure JSONL，不改变成功 `ModelCallRecord` 合同、不保存 provider 原始错误正文；无法取得的 hash、output artifact、finish reason、usage/cost 保留为 `None`，不伪造成功事实。failure audit 写入失败则返回错误；该覆盖不延伸到所有前置 binding/approval、routed 或工具失败路径，也不是通用 DLP 保证。routed fixture/model-adapter fallback 已创建新的父链 Attempt，记录实际事件证据并共享累计预算；不以模型切换清零 no-progress，不可恢复失败或额度耗尽后运行进入失败终态。此结果不覆盖 resolved provider fallback、retry/resume/fork 或跨进程恢复。

routed 路径的 EventLog 或 ProgressSink 故障会将仍 Running 的 Attempt/TaskRun/LoopRun 在当前进程内封为失败，防止再次调用模型；保留原故障，不把 progress 故障当成模型可重试错误，不覆盖已完成/取消/暂停状态。失败日志无法写入时不伪造 evidence refs，也不声称终态已持久化或可跨进程恢复。此封口不扩展 non-routed/resolved 路径的故障保证。

只读模型动作已接入 `PolicyGate → ReadOnlyToolExecutor → verification`。任意文件写入/Bash、真实 profile/live provider 和完整恢复仍未验收。`ModelRegistry` / `RuntimeConfig` 的 auth_ref 与 endpoint 认证材料校验、本地 `Redactor` 和 provider 出站脱敏均是已测规则边界，不是任意敏感正文的通用 DLP 保证。

`MemoryCandidate` / `MemoryCatalog` 已提供 project/goal/task 隔离、来源/provenance/confidence/lifecycle 校验、schema v1 校验、本地摘要脱敏及确定性候选查询（`tests/memory.rs`）。返回的仍是候选 metadata，不等于 accepted/active `MemoryRecord`；候选 JSONL 持久化和 retrieval EventLog 子切片已有，正式 promotion、PolicyGate、写入/promotion 审计及 Wiki/EvidencePack 链尚未完成，也不自动生成 global memory 或跨项目权限。

`MemoryCandidate::validate_source_events` 另提供显式来源身份校验：调用方传入已获授权的事件集合，检查每个引用唯一存在且 project/goal/task 与候选 scope 相符。不读取日志或 payload，不证明事件真实持久化、payload 真实性或来源可信度；`MemoryCatalog::insert` 不自动执行此校验，校验通过也不 promotion 或授予权限。

`MemoryStore` 已提供候选 metadata 的 append-only JSONL 持久化和 `read_catalog()`：锁内逐行校验并复用 `MemoryCatalog::insert`，物理行号错误不回显摘要正文，锁内拒绝重复 candidate ID。缺失 JSONL 的 read 不创建该日志，但锁操作可能创建父目录和 `.lock`；当前整份加载 catalog，不是分页或向量检索。它不解析 source events、不自动 promotion、不改变 lifecycle 或授予运行时权限。

`ReadOnlyLoopController::retrieve_memory_candidates()` 在生命周期安全边界之后校验 project 与指定 goal/task 必须匹配当前运行，从 store 加载 catalog 并返回候选 metadata（含候选摘要）。`memory_retrieval_requested/filtered/used` 仅记录 scope、候选 IDs/count 和 `returned_metadata_only`，不复制摘要或 source payload；`used` 不代表模型实际使用。Paused/terminal 或审计写入失败时不返回候选；三个事件不是原子事务，失败可留下前置事件。该显式接口不读取 source payload、不自动注入 prompt、不执行 promotion 或 PolicyGate 授权。

`GoalRevision` / `PlanRevision` 已提供 schema、父 revision、失效和 stale-plan 判定的最小领域契约；它们只表示 metadata 和旧计划边界，不接入后台 worker、takeover、自动重规划、模型审批恢复或跨进程恢复。

`CheckpointStore` 可流式扫描 JSONL，查询 loop 或 loop/step 最后追加的 checkpoint，并根据已记录的 SHA-256 边界和 recoverability 给出只读恢复建议。查询传播日志解码错误，只保留最新匹配记录；仍需扫描全日志，单条记录大小仍影响内存。该结果不读取或重算 workspace hash、不验证快照、不授权或执行恢复，不能替代跨进程恢复闭环。

`WorkspaceSnapshotStore` 是独立的 Unix-only 单文件 primitive：显式 capture/restore 一个 GoalContract scope 内既有的 UTF-8 `.md`/`.markdown` 单链接普通文件。正文上限为 1 MiB，序列化私有 record 上限为 6 MiB + 64 KiB；拒绝 traversal、symlink、非普通文件和不匹配的 project/goal/canonical workspace 绑定。store 位于 workspace 外，新目录/记录使用 0700/0600，现有 store/record 必须属于 effective user 且无 group/other 权限；不静默修正不安全权限，非 Unix fail-closed。私有 record 含原文，公共 `SnapshotMetadata`、返回结果与错误不包含正文，正文不自动进入 prompt、EventLog、trace、summary 或 model-call store；调用方仍须在 capture 前分类和授权数据。本地 owner/权限检查不是加密或同 OS 用户/root 隔离。

显式 restore 的 `expected_after_hash` 必须由 trusted caller 提供；approval 只由 controller 唯一 restore 入口通过 PolicyGate、metadata-only ApprovalStore（强制 `ApprovalScope::Once`）和 SideEffectLedger 完成，底层 `WorkspaceSnapshotStore` 不接收 approval 布尔值。恢复前复验 scope/path、snapshot hash/size 和当前文件 content hash，不匹配则拒绝，不覆盖该已检测到的人工后续修改。snapshot 的 `before_hash` 与 `expected_after_hash` 为 `SHA256(file bytes)`，不能混用旧 controller checkpoint 的 `SHA256(relative_path + NUL + file bytes)`。body hash 是完整性检查，不是认证，不能抵御同 OS 用户同时修改正文/hash；after-hash 匹配仅说明当前文件符合 trusted caller 的预期，不证明 harness 曾验证该状态。恢复复用精确条件替换并保留当前目标 Unix permission mode，不保留 inode、owner、ACL 或 xattrs；sync/rename 不构成恶意并发 writer 锁或完整 crash transaction，replacement 后 sync 报错时文件可能已改变，重试前必须检查。该 primitive 已接入 bounded Markdown controller 的 opt-in capture/authorized restore：只有配置 snapshot store 时才在 mutation 前 capture，restore 不由 verification failure 自动触发，也不作为模型工具或事件授权路径；不恢复 TaskRun/Attempt/预算、进程、跨进程状态或外部副作用，不代表完整 Phase 1 恢复已完成。

架构目标是四个专业方向都支持 7x24 常驻和多年可恢复生命周期；这不是 Phase 1 已实现能力的声明。Phase 1 的目标边界包含本地单机、顺序执行、持久化事件、任务状态和最小 `RecoveryCheckpoint`。当前代码已实现 JSON StateStore 持久化 reopen、Trace/Checkpoint/SideEffect JSONL 记录、只读 BoardProjection fixture、静态 GoalAlignment/DriftGuard 报告以及 Markdown 文件修改/链接验证开发 loop；完整恢复闭环、运行中用户控制面和真实模型 provider 仍是未完成验收项。

Phase 1 不实现分布式 worker、高可用故障转移、完整安全运营常驻服务、自动渗透测试执行链或自动安全处置闭环。后续方向必须复用共享 Harness 的任务、证据、授权、审批和恢复接口，而不能另起一套不可审计的常驻循环。

## 第一阶段必须实现

### 1. Harness Core 最小运行底座

第一阶段必须有一个通用核心，而不是直接写死某个开发 agent。

最小对象包括：

- `GoalContract`
- `LoopController`
- `ToolRegistry`
- `PolicyGate`
- `EventLog`
- `TraceRecord`
- `RecoveryCheckpoint`
- `SideEffectLedger`
- `HumanSteeringMode`

这些对象可以先是简单 schema 和本地实现，但边界必须清楚。

### 2. 软件开发智能体垂直切片

第一阶段主验证对象建议是软件开发智能体。

原因：

- 容易构造真实任务
- 容易验证结果
- 文件 diff、测试、lint、review、rollback 都有明确证据
- 比安全响应动作更适合作为早期闭环验证场景

最小能力包括：

- 读取仓库上下文
- 生成或维护 `GoalContract`
- 执行有限工具调用
- 修改文件
- 运行验证命令
- 记录每轮事件和工具调用
- 记录修改前后边界
- 失败后产生可审查的下一步建议

### 3. 持久化事件日志

第一阶段必须把关键执行过程落盘。

最小记录包括：

- `session_id`
- `loop_run_id`
- `step_id`
- `message_id`
- `tool_call_id`
- tool 输入输出摘要
- 文件修改摘要
- 验证命令与结果
- checkpoint 引用
- side effect 分类

事件日志是审计、恢复、轨迹检查、记忆候选和后续自动进化的基础。

### 4. Recovery Checkpoint 最小实现

第一阶段不需要 OS 级完整 checkpoint。

必须实现的是：

- 任务状态 checkpoint
- loop 状态 checkpoint
- 文件 diff 或 workspace 修改边界
- checkpoint metadata
- checkpoint 与 event log 的关联

第一阶段重点不是“能恢复一切”，而是明确知道：

- 哪些能恢复
- 哪些不能恢复
- 为什么不能恢复
- 哪些外部副作用需要人工处理或补偿

### 5. Goal Alignment Checkpoint 最小实现

第一阶段必须有方向检查。

最小实现可以是规则化检查或人工可读报告，而不是复杂 evaluator。

至少检查：

- 当前动作是否服务 root goal
- 当前阶段是否服务 stage goal
- 新增修改是否超出 scope
- 是否把辅助问题升级成主任务
- 是否需要人类确认

### 6. DriftGuard 最小实现

第一阶段必须防止 agent 把项目改乱。

最小实现包括：

- 检查修改文件范围
- 检查 diff 规模
- 检查是否触碰核心文档、配置或规则
- 检查是否出现大规模重命名、重构或风格 churn
- 高风险 diff 要求人工确认或 checkpoint

### 7. Human Steering Mode 最小实现

第一阶段至少支持以下模式语义：

- `human_steerable_loop`
- `human_in_the_loop`
- `manual_takeover`

可以暂不实现完整 UI，但事件和状态中必须记录当前模式。

默认建议：

```text
normal dev task -> human_steerable_loop
high-risk file/tool action -> human_in_the_loop
manual intervention -> manual_takeover
```

### 8. Context Hygiene 最小实现

第一阶段不要求完整自动 context pruning。

但必须从一开始记录：

- 哪些上下文被使用
- 哪些上下文被判断为关键
- 哪些上下文可能污染当前任务
- compact / summary 的来源和置信度

这为后续精准上下文清理和低 token 运行打基础。

### 9. Baseline Tool Recipes

第一阶段应预置少量高频工具正确用法。

目标不是做完整 skill 系统，而是避免 agent 在基础工具上反复犯低级错误。

第一批建议覆盖：

- shell 安全引用
- `rg` 固定字符串搜索
- git 状态检查
- 最小 diff 查看
- 测试命令记录
- Markdown 链接检查

### 10. 多模型接入最小垂直切片

第一阶段不能把多模型只写成未来愿景，至少要实现可验证的静态接入边界：

- ProviderProfile、EndpointProfile、ModelProfile、PromptProfile、AgentRolePrompt、TaskPromptPack、ResolvedPromptPack schema
- 静态 ModelRegistry 和 PromptRegistry，允许配置 cheap/fast、strong/slow、specialized、evaluator 或 local profile
- 至少一个真实 model adapter；其他模型可以是未连接 live endpoint 的可验证 fixture
- 基于任务类型、能力、风险、成本和延迟约束的规则式 RoutePolicy
- 每次路由和调用写入 RouteDecision、ModelCallRecord、ResolvedPromptPack 版本和验证状态
- fallback 必须能力兼容、显式记录并重新验证；高风险写操作不能静默降级
- registry、prompt、trace、event log 和 checkpoint 只保存 auth_ref，不保存 API key 或 token

### 10.1 执行策略选择器最小切片

执行策略与 ModelRouter 分工不同：前者选择执行约束，后者选择具体模型。当前策略切片使用显式 TaskFeatures、既有 domain RiskLevel 和固定策略元数据，不推断自然语言任务属性，也不创建多 agent runtime。

- 校验不确定性和预算倍率的有限值/范围；非法输入或倍率不足返回错误，不静默降级安全要求
- GoalContract 风险是下限；High/Critical、Production、不可逆或显式审批条件均不能被策略降级取消
- 库接口显式启用；`strategy_selected` / `strategy_adjusted` 关联项目、goal、task_run、loop、特征、理由、基础/前次/有效预算和策略修订号
- 使用整数分子/分母计算倍率并向下取整，分配不突破原授权和当前上限，保留八个消耗计数；三类进展/失败 guard 只取较小值，不乘倍率；同实例调整不清零独立 tracker 和 action fingerprint
- 策略路径在执行前约束 step/model/tool 预算；PolicyGate 仍独立生效。审批未解析时零动作，pause/resume 不算审批
- 验证频率、交叉模型验证及专业策略只是需求元数据，不能撤销 GoalContract verification_plan；实际验证调度、TOML/自定义策略注册、策略恢复和多 agent 编排尚未实现

旧 controller `start()` 和 fixture CLI 不自动使用策略；新建 start_with_strategy 也不是跨实例恢复入口。已有预算对象不等于 Attempt/GoalRevision/验证失败统计八维运行时全部完成。

### 11. Portfolio、Task 与 BoardProjection 最小切片

第一阶段只实现单 Portfolio、单 Project、本地顺序执行 fixture，但必须先固定未来可扩展的身份和投影边界：

- 定义 Portfolio、Project、Task、TaskRun、Attempt、TaskDependency schema
- 定义 Task 领域状态及合法迁移
- 从 EventLog + StateStore 重建 BoardProjection
- 提供只读 CLI / JSON board fixture
- 不实现真实多项目并行调度、worktree lease、分布式 worker 或交互式 TUI/Web

Phase 1 的对象、事件、状态迁移、幂等、版本冲突、投影和敏感数据规则以 PHASE_1_CONTRACTS.md 为准。

### 12. User steering 与有限 Loop

第一阶段必须实现可审计的插话入口和停止条件，即使暂不提供交互式 TUI/Web：

- 新输入先持久化到 `UserCommandInbox`，控制指令优先于普通 steer；
- 在模型返回、工具完成、LoopStep 完成、验证开始、checkpoint、heartbeat 和高风险动作前读取 inbox；
- 相关输入创建 `GoalRevision` 并使旧 `PlanRevision` 失效；无关输入创建独立 Task；
- `pause`、`cancel`、`takeover` 和拒绝高风险动作不能跳过 `PolicyGate`；side effect 不确定时进入 `waiting_human` 或 `blocked`；
- `LoopBudget` 至少限制 step、tool call、model call、Attempt、GoalRevision、连续 verification failure、无进展步数和相同动作重复次数；
- 达到上限必须写事件并停止自动重试，不能通过插话或重新规划清零预算和无进展计数。

这里的“有进展”只接受状态、evidence、文件边界、verification 结果或新失败原因变化；重复计划、重复动作和重复失败不计进展。

当前实现状态：fixture/resolved model step 已验证基于 `model_id + goal_id + observation` fingerprint 的重复请求检测，并在达到无进展预算时停止；完整 action payload、plan revision、失败分类和跨进程恢复尚未接线。

## 第一阶段只做占位

以下能力第一阶段必须在 schema 或接口上留位置，但不要求完整实现：

- 历史表现驱动的自适应多模型路由
- 动态插件安装
- MCP 自动发现与治理
- 向量记忆库
- 图记忆库
- dream learning
- 自动 skill 生成
- 自动 policy 进化
- 跨项目全局记忆
- 完整交互式 TUI/Web control plane
- 持续运维、安全评估与漏洞研究、安全运营的完整运营闭环
- 通用 Manus-like agent

占位的要求是：

- 不写死未来无法扩展的结构
- 不把未来能力伪装成当前已实现
- 不让占位阻塞第一阶段交付

## 第一阶段明确不实现

第一阶段不实现：

- 插件市场
- 企业级多租户
- 分布式调度
- 完整 sandbox / process checkpoint
- 自动渗透测试执行链
- 自动安全处置闭环
- 跨多个真实项目的自动联调
- 完整向量检索和图推理系统
- 完整 self-evolution runtime

这些能力保留在长期方向中，但不进入第一阶段交付标准。长期方向的正式划分和验收依赖见 `AGENT_RND_ROADMAP.md`、`CONTINUOUS_OPERATIONS_AGENT_DESIGN.md`、`SECURITY_ASSESSMENT_AGENT_DESIGN.md` 和 `SECURITY_OPERATIONS_AGENT_DESIGN.md`。

## 第一阶段推荐技术边界

技术选择不在本文件中最终锁定。

但第一阶段实现应遵守以下边界：

- 本地优先
- 可审计优先
- SQLite / JSONL 足够时不引入复杂外部依赖
- 文件系统 artifact 可读可迁移
- schema 先稳定，优化后置
- 外部副作用默认保守处理

推荐最小存储组合：

```text
events.sqlite      -> durable event log
state.sqlite       -> current session / loop state
traces/*.jsonl     -> readable trace stream
checkpoints/       -> checkpoint metadata and artifacts
recipes/           -> baseline tool recipes
```

## 第一阶段成功标准

第一阶段成功不以“agent 看起来很智能”为标准。

成功标准应是：

1. 能运行一个真实软件开发任务闭环
2. 能明确记录目标、步骤、工具调用、修改、验证和结果
3. 能在关键点生成或引用 checkpoint
4. 能区分可恢复修改和不可恢复副作用
5. 能发现明显目标偏离或 scope creep
6. 能在高风险动作前要求人类介入
7. 能把失败工具调用沉淀为 tool recipe candidate
8. 能用事件日志复盘一次完整任务
9. 能说明哪些能力只是占位，哪些已经实现

如果这些成立，即使没有复杂 UI、向量记忆、自适应多模型路由和后三个专业方向的完整闭环，第一阶段也是成功的。

## 第一阶段建议实现顺序

建议顺序：

1. 定义核心 schema：goal、loop、event、tool call、checkpoint、side effect
2. 实现本地 event log 和 trace 写入
3. 实现最小 ToolRegistry 和 PolicyGate
4. 实现软件开发任务 loop skeleton
5. 接入文件修改和验证命令记录
6. 实现 RecoveryCheckpoint metadata 和 diff boundary
7. 实现 GoalAlignmentCheckpoint 报告
8. 实现 DriftGuard 最小检查
9. 实现 baseline tool recipes
10. 用一个真实开发任务做端到端验证

## 契约优先说明

实现顺序以 PHASE_1_CONTRACTS.md 为第一入口；如果本文件中的长期建议与契约冲突，先修正文档，再开始代码实现。

第一阶段的完成定义还必须包括：Task 状态迁移可验证、BoardProjection 可由 EventLog 重建、重复 mutating command 不重复产生副作用。

## 与其他文档的关系

- `ARCHITECTURE_DIRECTION.md` 定义项目方向
- `TRACKS_AND_CAPABILITIES.md` 定义四个专业方向与横向能力关系
- `AGENT_RND_ROADMAP.md` 定义四个方向的长期优先级、依赖和验收阶段
- `CONTINUOUS_OPERATIONS_AGENT_DESIGN.md` 定义持续运维方向
- `SECURITY_ASSESSMENT_AGENT_DESIGN.md` 定义安全评估与漏洞研究方向
- `SECURITY_OPERATIONS_AGENT_DESIGN.md` 定义安全运营方向
- `STORAGE_AND_RECOVERY_DISCUSSION.md` 定义长期运行和恢复底座
- `MEMORY_STORAGE_AND_CONSOLIDATION.md` 定义多层记忆和存储演进
- `MEMORY_SCOPE_AND_LIFECYCLE_CONTRACTS.md` 定义记忆 scope、provenance、lifecycle、promotion 和可见性
- `RAG_WIKI_KNOWLEDGE_COMPILATION.md` 定义 RAG 证据到项目 Wiki 派生视图的编译、回链、过期和分级发布
- `GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md` 定义 goal / loop 模型
- `GOAL_ALIGNMENT_CHECKS.md` 定义方向检查
- `DRIFT_GUARD_DISCUSSION.md` 定义防破坏性漂移
- `SIDE_EFFECT_LEDGER_DISCUSSION.md` 定义副作用边界
- `HUMAN_STEERING_MODES.md` 定义人类驾驭模式

本文件是这些讨论稿进入实现前的第一阶段收敛层。

## 当前结论

`steerBox` 当前文档已经足够支撑进入第一阶段开发。

该结论以 `PHASE_1_CONTRACTS.md`、Scope、Design、TODO、Validation、Documentation Map 和 Architecture Questions 已完成同步为前提；在本轮同步提交落地前，只能视为待闭合的设计结论。

但开工时必须以本文件为实现边界，不能直接按全部长期讨论稿展开。

第一阶段的正确姿势是：

```text
small core,
real loop,
durable evidence,
recoverable boundary,
human-steerable control,
future-ready interfaces.
```
