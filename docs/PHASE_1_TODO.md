# Phase 1 TODO

## 文档目的

本文件是第一阶段开发任务清单。

它服务于实际开工，不承载长期理论设计。任务完成标准必须可验证。

## 当前阶段目标

第一阶段目标：

```text
实现 shared harness core 的最小闭环，
并用软件开发智能体垂直切片验证 goal、loop、event、checkpoint、policy、side-effect、trace 是否能协同工作。
```

## 任务分组

### P0. 开工前整理

- [ ] 阅读并确认 PHASE_1_CONTRACTS.md 是 schema、event、state 和 projection 的 Phase 1 规范源
- [ ] 确认当前文档变更已提交或明确保留在工作区
- [ ] 确认第一阶段只按 `PHASE_1_IMPLEMENTATION_SCOPE.md` 和 `PHASE_1_DESIGN.md` 开发
- [ ] 确认第一阶段不创建持续运维、安全评估与漏洞研究、安全运营的完整闭环
- [ ] 确认运行数据目录不会污染源码提交

验收：

- `git status --short` 输出已检查
- 第一阶段边界无歧义

### P1. 项目骨架

- [x] 选择第一阶段主语言和构建工具（Rust + Cargo）
- [x] 创建最小源码目录
- [x] 创建最小测试目录
- [x] 创建最小 CLI 或 runtime 入口
- [x] 添加基础格式化 / test / build 命令（当前提供 fmt、test、build）

验收：

- 能运行 `build` 或等价命令
- 能运行最小测试
- README 或开发说明中记录命令

### P2. Core Schema

- [x] 定义 EventEnvelope、event_id、sequence_no、correlation_id 和 causation_id
- [ ] 定义 Portfolio、Project、Task、TaskRun、Attempt 和 TaskDependency
- [x] 定义 Task 状态及合法迁移表
- [x] 定义最小 BoardProjection / BoardTaskCard
- [x] 定义 ToolUseErrorEvent、LessonCandidate 和最小 FailureCategory（schema 与最小 `tool_call_failed` 关联事件已有测试；独立 ToolCallRecord 持久化仍待接线）

- [x] 定义 `GoalContract`
- [x] 定义 `LoopRun`
- [x] 定义 `LoopStep`
- [x] 定义 `ToolCallRecord`（schema 已有序列化测试；实际 tool executor 记录仍待接线）
- [ ] 定义 `PolicyDecision`
- [ ] 定义 `SideEffectEvent`
- [x] 定义 `CheckpointMetadata`
- [ ] 定义 `VerificationResult`
- [x] 定义 `TraceRecord`
- [x] 定义 `ProviderProfile`、`EndpointProfile`、`ModelProfile`（已有注册/序列化及关联验证）
- [ ] 定义 `PromptProfile`、`AgentRolePrompt`、`TaskPromptPack`、`ResolvedPromptPack`（仅 PromptProfile 已实现，其余 pack 尚未实现）
- [x] 定义 `RouteDecision` 和 `ModelCallRecord`（字段结构和最小语义校验已有测试；完整调用审计接线仍待完成）

验收：

- schema 有单元测试或序列化测试
- 字段能覆盖第一阶段端到端流程
- schema 有版本字段或预留版本策略

### P3. Local Storage

- [x] 实现从 EventLog 重建 loop/step current state 的最小 StateStore fixture
- [x] 实现 StateStore JSON 持久化与 reopen 恢复测试
- [x] 从 EventLog 重建 BoardProjection 的最小 fixture（只读 CLI 输出；完整依赖/Attempt 投影仍待实现）

- [ ] 初始化 `events.sqlite`
- [ ] 初始化 `state.sqlite`
- [x] 实现 append-only event write（JSONL 第一切片）
- [ ] 实现 current state update
- [x] 实现 TraceRecord JSONL append/read 最小切片
- [x] 实现 run summary 写入（dev markdown 成功/失败两条路径均写入 `runs/<loop_run_id>/summary.json`）

- [x] 实现 RuntimeConfig JSON 解析、`/temp/steerbox` 路径校验和未知字段拒绝

Memory 最小实现：

- [ ] 定义 `MemoryRecord`、`MemoryCandidate`、`MemoryPromotion` 的 Phase 1 子集
- [ ] 定义 `WorkingMemory` 与 project-scoped candidate 的可见性边界
- [ ] 实现 candidate 的 provenance、confidence、lifecycle state 和 source event refs
- [ ] 实现按 project / goal / task 的确定性 metadata retrieval
- [ ] 将 memory 写入、检索和 promotion decision 写入 EventLog
- [ ] 明确 Phase 1 不启用 vector index、graph index、global auto-promotion 或 cross-project implicit visibility
- [ ] 定义 `EvidencePack`、`WikiPage`、`WikiRevision` 的 Phase 1 子集
- [ ] 实现通过验证结果生成项目范围 `WikiPageCandidate`
- [ ] 实现 wiki-first 查询、source citation、expiry 和 revalidation 事件
- [ ] 实现低风险项目页自动发布，高风险/global/cross-project 只生成 candidate

验收：

- 写入一轮 fake loop 后，event/state/trace 都能查询
- 进程重启后能读取 current state
- trace 文件可被 `tail` / `rg` 阅读
- candidate 可从 source event 反查，且 project scope 过滤不会返回其他项目的记录
- memory candidate 不会自动变为 global policy、skill 或 model routing
- Wiki 页面可以反查 EvidencePack/source ref，过期页面不会直接支撑高风险动作

### P4. ToolRegistry

- [x] 定义 `ToolProfile`
- [x] 注册只读工具
- [x] 注册 shell / command 工具的受控包装（当前仅固定 argv 的只读 `git status`；不接受 shell 字符串）
- [x] 为每个工具配置 side-effect class
- [x] 为每个工具绑定 risk level
- [x] 读取 baseline tool recipes

验收：

- 未注册工具不能调用
- 工具调用前能查到 side-effect class
- `rg`、git status、Markdown link check 至少有 recipe

### P5. PolicyGate

- [x] 实现工具调用前 policy check（LoopController 先通过 PolicyGate，才调用只读执行器）
- [ ] 实现文件写入前 policy check（Markdown CLI 已有最小审批切片；统一 controller 写动作尚未接线）
- [x] 实现高风险动作升级到 human approval
- [x] 实现 forbidden tool 阻断
- [x] 实现 policy decision event 记录

验收：

- 低风险只读工具允许执行
- 未授权写操作被阻断或要求审批
- policy decision 可在 event log 中复盘

### P6. LoopController

- [ ] 支持 Task 状态合法迁移和非法迁移拒绝

- [x] 实现 `single_agent_loop` skeleton（只读 observation loop；无模型、工具和文件副作用）
- [x] 实现 `react_tool_loop` 的最小 step 流程（策略检查 + 受控只读 `git status` 执行）
- [ ] 实现 `plan_execute_loop` 的最小阶段状态
- [ ] 支持 step start / complete / fail
- [ ] 支持 stop condition
- [ ] 支持失败后 reflection summary
- [x] 实现 LoopBudget 与 NoProgressTracker 的领域级停止规则
- [x] 实现 `UserCommandInbox` 持久化、command_id 幂等和按 loop 的 pending 读取；LoopController 已在安全边界应用 `pause` / `resume` / `cancel`，`takeover` 和 side-effect 不确定分支仍待完成
- [ ] 实现 `GoalRevision` / `PlanRevision` 与旧计划失效
- [ ] 实现 `InterruptRequest` 的安全边界和 `side_effect_uncertain` 分支
- [x] 实现 `LoopBudget` 与 `NoProgressObservation`，达到上限后停止自动重试（model loop 已接入重复动作检测）

验收：

- 能跑完一个无文件修改的只读 loop
- 能跑完一个有文件修改和验证的 dev loop
- 每个 step 都有 event 和 trace
- 插话不会清零 TaskRun budget、Attempt lineage、retry 或 no-progress counter
- 重复动作、重复失败和重复计划能触发 `no_progress_detected`

### P7. RecoveryCheckpoint

- [x] 实现 checkpoint metadata JSONL 写入
- [ ] 记录 checkpoint 与 goal / loop / step 的关系
- [ ] 记录 checkpoint 前后的文件 diff boundary
- [ ] 记录 checkpoint 覆盖范围
- [ ] 记录不可恢复 side effect 引用

验收：

- 文件修改前后能生成 checkpoint metadata
- 能说明某次修改是否可恢复
- checkpoint 不声称恢复未记录的外部动作

### P8. SideEffectLedger

- [x] 定义最小 side-effect record schema
- [ ] 实现 tool call 到 side-effect event 的记录
- [ ] 支持 read-only、本地可逆、部分可逆、可补偿、不可逆、人工处理分类
- [ ] 高风险副作用要求 policy gate

验收：

- 每个 tool call 都有 side-effect 记录或明确 none
- 不可逆动作默认不能自动执行
- restore / replay 前能查询 side-effect boundary

### P9. GoalAlignmentCheck

- [x] 实现 root/stage goal 对齐检查报告
- [x] 检查 scope / out_of_scope
- [ ] 检查是否需要 human intervention
- [ ] 将检查结果写入 event log

验收：

- 故意偏离任务时能产生 warning 或 block 建议
- 检查结果能关联到 step_id

### P10. DriftGuard

- [x] 检查修改文件数量
- [x] 检查 diff 行数
- [x] 检查核心文件触碰
- [ ] 检查大规模重命名 / 删除
- [x] 输出 allow / warn / require_checkpoint / require_human_approval / block

验收：

- 小范围文档修改允许
- 大范围无关修改触发 warning 或 approval
- 核心配置修改触发更高风险决策

### P11. Baseline Tool Recipes

- [x] ToolUseErrorEvent 能关联原始 tool call、failure category 和 LessonCandidate（通过 `tool_call_failed` 事件 payload；recipe candidate 仍待实现）

- [ ] 写入 shell safe quoting recipe
- [ ] 写入 `rg -F` recipe
- [ ] 写入 git status / diff read-only recipe
- [ ] 写入 Markdown link check recipe
- [ ] 写入 file write verification recipe

验收：

- ToolRegistry 能读取 recipe
- tool call 记录能引用 recipe id
- 失败工具调用能生成 recipe candidate

### P12. Model Registry、Prompt Resolution 与规则路由

- [x] 实现静态 `ModelRegistry` 和 `PromptRegistry`（注册及 registry JSON 反序列化重验标识/关联，prompt 版本/role/task 查询已有测试；模型路由已使用 registry，完整 prompt resolution 尚未接线）
- [x] 建立 provider-neutral OpenAI-compatible adapter、fake transport 和严格错误分类（真实网络 transport、凭据解析和 LoopController 接线仍待实现）
- [x] 建立 provider-to-`ModelAdapter` bridge（仅接受严格 `decision`/`summary` JSON envelope；自然语言不猜测）
- [ ] 接入至少一个真实网络 model provider
- [x] 接入 secret-free `LocalFixtureModelAdapter`，并记录结构化 model decision 事件
- [ ] 配置 cheap/fast、strong/slow、specialized、evaluator 或 local profile fixture
- [x] 实现按 task type、agent role、能力、风险、成本和延迟过滤与排序的规则式路由，并通过 `ModelAdapterRegistry` 接入 LoopController（需审批时 fail-closed；审批恢复尚未实现）
- [x] 定义、校验并持久化 `RouteDecision` 到 EventLog；routed 模型调用的开始/成功/失败事件关联 route ID
- [x] 按 provider、model、role、task 定义最小 prompt binding 查询（完整 `ResolvedPromptPack` 解析仍待实现）
- [x] 定义并校验 `ModelCallRecord`、prompt 版本和验证状态，提供独立 append-only `ModelCallStore`（尚未将实际调用和 resolved prompt 写入该 store）
- [ ] 实现能力兼容、非静默的 fallback 事件
- [ ] 验证 registry、prompt、trace、event log 和 checkpoint 不保存认证明文

验收：

- 低风险只读任务可路由到 cheap/fast profile
- 高风险或高质量任务不会只按最低成本选择模型
- 切换模型会创建新的 attempt / route decision，而不是静默覆盖
- 每次调用能反查 provider、endpoint、model、prompt 版本、路由理由和验证结果
- 仅保存 `auth_ref`；测试 fixture 不包含真实 API key 或 token

### P12.1 执行策略选择器

- [x] 定义九个策略标识、显式 TaskFeatures 和纯规则选择器；风险复用 domain RiskLevel，有限值/范围校验及倍率不足拒绝已有回归
- [x] 实现安全预算分配：整数比值运算分配、既有授权/当前上限 clamp、消耗保留、进展/失败 guard 不放宽
- [x] 接入 opt-in `start_with_strategy` / `select_strategy`，记录选择/调整及审批介入事件；GoalContract 风险下限、审批前零动作、step/model/tool 预算和调整不补额度已有测试
- [x] pause/resume 不解除审批，不清零消耗或重复动作历史；PolicyGate forbidden/unregistered/allowed_tools 约束保持生效
- [ ] 自定义策略配置注册与验证调度接线（当前只有固定配置和验证频率元数据，不撤销 goal 验证要求）
- [ ] 策略状态/修订、预算和重复动作历史的完整恢复

验收：`tests/strategy_selector.rs`、`tests/budget_calculator.rs`、`tests/loop_strategy.rs`、`tests/loop_strategy_controls.rs`、`tests/strategy_audit_failure.rs`；策略主审计写入失败保留旧策略、额度和revision，同实例恢复原日志路径后下一修订仍连续；补充审批事件写入失败暂无独立注入测试。审批 resolver 尚未实现。协作和专业领域策略仅是元数据，不代表多 agent 或专业领域 runtime。旧 start/fixture CLI 不自动选择策略；新 start_with_strategy 不是跨实例恢复入口。

### P13. Dev Agent Vertical Slice

- [x] 输出只读 BoardProjection / JSON board fixture

- [x] 定义一个小型真实开发任务（Markdown 修改与链接验证）
- [x] 创建 goal
- [x] 加载上下文
- [x] 执行文件修改
- [x] 运行验证命令
- [x] 记录 checkpoint 和 side effect
- [x] 输出 run summary

验收：

- 任务能端到端完成
- event log 可复盘
- trace 可读
- checkpoint metadata 存在
- 验证结果明确

### P14. Documentation Update

- [x] 更新 PHASE_1_CONTRACTS.md 的引用（当前实现状态在本 TODO、Scope 和 Validation Plan 中维护）

- [x] 更新 README 或开发说明中的运行命令
- [x] 更新第一阶段完成状态
- [x] 记录已实现能力和占位能力
- [x] 记录已知限制
- [x] 明确运行数据写入 `/temp/steerbox/<project_ref>/.steerbox/`
- [x] 增加 metadata-only execution history/query 投影、`execution list/show/page` CLI 和 detached snapshot 分页（底层仍使用 `read_all()`，不是 live cursor）
- [x] 明确用户插话、计划失效和死循环停止契约

验收：

- 用户能按文档跑最小验证
- 文档不把占位能力写成已实现能力

## 第一阶段暂不做 TODO

以下不要在第一阶段拆任务：

- 历史表现驱动的自适应多模型路由
- 多智能体 supervisor-workers
- 向量记忆检索
- 图记忆推理
- 自动 skill 生成
- 安全响应自动处置
- 插件市场
- 交互式 TUI/Web control plane
- 分布式 worker

## 完成定义

第一阶段完成必须同时满足：

- 一个真实开发任务端到端完成
- 事件日志完整
- trace 可读
- checkpoint metadata 可查
- side-effect ledger 可查
- policy decision 可查
- goal alignment 或 drift guard 至少触发一次有效检查
- 验证命令有明确结果
- 至少一次模型路由、prompt 解析和调用记录能通过 event / trace 复盘
- Task 状态迁移和 BoardProjection 重建至少各有一次通过记录
- 失败工具调用至少生成一次可审查 LessonCandidate
