# steerBox Documentation Map

## 文档目的

本文件用于说明 `steerBox` 文档体系如何拆分、如何阅读、以及哪些文档负责主线、哪些文档负责横向支撑能力。

本文件不替代任何已有文档，只负责组织关系。

## 阅读顺序

建议按以下顺序阅读：

1. `README.md`
2. `docs/ARCHITECTURE_DIRECTION.md`
3. `docs/TRACKS_AND_CAPABILITIES.md`
4. `docs/PHASE_1_IMPLEMENTATION_SCOPE.md`
5. `docs/PHASE_1_DESIGN.md`
6. `docs/PHASE_1_CONTRACTS.md`
7. `docs/PHASE_1_TODO.md`
8. `docs/PHASE_1_VALIDATION_PLAN.md`
9. `docs/HARNESS_PRINCIPLES.md`
10. `docs/STORAGE_AND_RECOVERY_DISCUSSION.md`
11. `docs/MEMORY_STORAGE_AND_CONSOLIDATION.md`
12. `docs/GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md`
13. `docs/AGENT_ORCHESTRATION_PATTERNS.md`
14. `docs/CONTROL_PLANE_DISCUSSION.md`
15. `docs/GOAL_ALIGNMENT_CHECKS.md`
16. `docs/LOOP_TRAJECTORY_CHECKS.md`
17. `docs/HUMAN_STEERING_MODES.md`
18. `docs/AI_AGENT_EFFICIENCY_ENGINEERING.md`
19. `docs/PLUGIN_AND_EXTENSION_ARCHITECTURE_DISCUSSION.md`
20. `docs/MODEL_PROMPT_AND_CHECKPOINT_POLICY.md`
21. `docs/PROMPT_CHECKPOINT_TEMPLATES.md`
22. `docs/CHECKPOINT_POLICY_ALGORITHM.md`
23. `docs/SIDE_EFFECT_LEDGER_DISCUSSION.md`
24. `docs/DRIFT_GUARD_DISCUSSION.md`
25. `docs/REFERENCE_FRAMEWORKS_AND_OBSERVABILITY.md`
26. `docs/AGENT_EVOLUTION_READING_LIST.md`
27. `docs/AGENT_EVOLUTION_INNOVATION_SYNTHESIS.md`
28. `docs/ARCHITECTURE_QUESTIONS.md`
29. `docs/HARNESS_SOURCES.md`

## 文档分层

### 1. 项目入口

- `README.md`
- `README_CN.md`
- `README_EN.md`

职责：

- 给出项目定位
- 保留两条主线
- 保持简短
- 链接到更深入的 docs

README 不应该承载完整架构细节。

### 2. 总体方向

- `docs/ARCHITECTURE_DIRECTION.md`

职责：

- 说明为什么是一个项目
- 说明为什么保留软件开发智能体与安全智能体两条主线
- 说明通用 harness 与领域能力的边界
- 说明当前不做什么

该文档是上位方向，不是最终实现规格。

### 3. 主线与横向能力关系

- `docs/TRACKS_AND_CAPABILITIES.md`

职责：

- 拆分主线场景、通用 harness 核心、横向支撑能力
- 避免把 goal、loop、memory、efficiency、tool/skill/MCP、auto-evolution 误写成第三条业务主线
- 给未来目录和模块边界提供依据

### 3.1 第一阶段实现范围

- `docs/PHASE_1_IMPLEMENTATION_SCOPE.md`

职责：

- 把长期讨论稿收敛成第一阶段可执行边界
- 明确第一阶段必须实现、暂不实现、只做占位的能力
- 定义第一阶段成功标准
- 防止开工时被未来能力拖成过度复杂方案

该文档负责回答：现在可以开始实现什么，以及哪些未来设计不能进入第一阶段交付标准。

### 3.2 第一阶段开发执行文档

- `docs/PHASE_1_DESIGN.md`
- `docs/PHASE_1_TODO.md`
- `docs/PHASE_1_VALIDATION_PLAN.md`

职责：

- 将第一阶段讨论稿转换成可开发设计
- 给出模块边界、核心 schema、存储布局、运行流程和非目标
- 拆分可执行 TODO 与验收条件
- 定义端到端验证用例和通过标准

该组文档负责回答：具体怎么开始开发、按什么顺序开发、如何证明第一阶段真的可用。

### 3.3 第一阶段实现契约

- `docs/PHASE_1_CONTRACTS.md`

职责：

- 作为 Phase 1 的规范源，固定公共 ID、schema version、object version 和 EventEnvelope
- 固定 Portfolio、Project、Goal、Task、TaskRun、Attempt、TaskDependency 的关系与最小字段
- 固定 Task 状态机、Attempt lineage、succeeded 前置条件和幂等 / version conflict 规则
- 固定 EventLog、StateStore、BoardProjection、Verification、Artifact、Evidence 的事实源与重建边界
- 固定 failure taxonomy、schema migration、auth_ref 和 secret/redaction 规则

该文档负责回答：实现时哪些字段、事件、状态迁移和投影规则不能靠各模块自行解释。若与长期讨论稿冲突，Phase 1 先以本契约为准，再补同步修订。

### 4. 外部参考原则

- `docs/HARNESS_PRINCIPLES.md`
- `docs/HARNESS_SOURCES.md`

职责：

- 提炼官方 harness 参考文档
- 保留来源
- 区分官方结论、推断、行动建议

这些文档是学习和校准材料，不应覆盖 `steerBox` 原有项目方向。

### 5. 存储、恢复与长期运行

- `docs/STORAGE_AND_RECOVERY_DISCUSSION.md`

职责：

- supervisor
- state ownership
- event log
- SQLite / JSONL 分层
- checkpoint / restore
- side-effect ledger
- 长时运行恢复策略

该文档负责高可靠长期运行底座。

### 5.1 多层记忆、存储与后台整理

- `docs/MEMORY_STORAGE_AND_CONSOLIDATION.md`

职责：

- 定义 event log、current state、checkpoint、working memory、episodic memory、semantic memory、procedural memory、code index、graph index 和 cold archive 的边界
- 说明同步热路径写入与异步后台 consolidation 的分工
- 说明向量存储适合语义检索，但不能替代事实源、审计、权限、checkpoint 或确定性关系查询
- 给出从本地可靠存储到 vector / graph / dream learning / enterprise memory 的递进实现路线

该文档负责回答：agent 如何长期记得准、查得全、越用越稳，同时避免记忆污染和不可审计。

### 6. Goal 与 Loop Engineering

- `docs/GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md`

职责：

- 定义 `GoalContract`
- 定义 `LoopController`
- 定义 observe / goal alignment checkpoint / plan / act / record / verify / reflect / update / recovery checkpoint decision / next action 循环
- 说明 policy、tool、model、budget、verification、recovery 控制门

该文档负责回答：agent 如何围绕明确目标高效闭环推进。

### 6.1 Agent 编排模式

- `docs/AGENT_ORCHESTRATION_PATTERNS.md`

职责：

- 整理 single agent、ReAct、plan-execute、reflection、tree/graph of thoughts、router、supervisor、多智能体、事件触发 loop 等主流模式
- 说明这些模式是 `LoopController` 之上的可插拔策略，不替代 harness core
- 定义 `LoopPattern`、`OrchestrationPolicy`、`AgentRole`、`HandoffContract` 等抽象
- 明确第一阶段只实现简单模式，复杂多 agent / supervisor / tree search 先占位

该文档负责回答：未来先进 Agent 编排能力如何进入 steerBox，而不把第一阶段做复杂。

### 6.2 Portfolio、任务编排与控制面

- `docs/CONTROL_PLANE_DISCUSSION.md`

职责：

- 定义 Portfolio、Project、Goal、Task、TaskRun / Attempt、TaskDependency、CrossProjectLink、Artifact、Evidence
- 区分 Supervisor、Scheduler、Orchestrator、Worker 和 Control Plane 的职责
- 定义 backlog / ready / queued / running / waiting / verifying / succeeded 等领域状态
- 将领域状态投影为 TODO / DOING / DONE / ATTENTION 看板列
- 规定并行执行的 workspace lease、ownership、conflict / join、retry、cancel 和 backpressure
- 规定 CLI / TUI / JSON / Web adapter 共享 BoardProjection，控制面写命令必须审计
- 规定跨项目进化通过 SystemEvolutionTask、evidence、regression、rollback 和 approval 提升
- 将 RouteDecision、ResolvedPromptPack、ModelCallRecord 关联到每个 TaskRun

该文档负责回答：命令行 agent 如何组织多个项目、多个任务和并行进度，并让看板、验证、问题和进化状态可审计、可恢复、可协同。

### 7. 目标对齐检查点

- `docs/GOAL_ALIGNMENT_CHECKS.md`

职责：

- 区分 `RecoveryCheckpoint` 和 `GoalAlignmentCheckpoint`
- 定义长程自主 loop 中用于检查和纠正 root goal、stage goal、stage chain 偏离的机制
- 定义 aligned / weakly_aligned / drifting / off_goal / unclear 等方向判断

该文档负责回答：agent 如何在长程任务中持续确认自己没有偏离最初目标。

### 7.1 Loop 轨迹审查

- `docs/LOOP_TRAJECTORY_CHECKS.md`

职责：

- 定义 `LoopTrajectoryCheck`，用于审查 step 1 到 step N 的执行路径是否合理
- 区分 `LoopTrajectoryCheck`、`GoalAlignmentCheckpoint`、`DriftGuard` 和 `RecoveryCheckpoint`
- 检查累积目标漂移、scope creep、check gaming、错误假设延续、重复失败和副作用链不清
- 说明 exit condition 通过并不等于执行轨迹合理

该文档负责回答：agent 每一步看似合理时，整体路径是否已经逐渐跑偏。

### 7.2 人类驾驭与介入模式

- `docs/HUMAN_STEERING_MODES.md`

职责：

- 区分 `autonomous_loop`、`human_steerable_loop`、`human_in_the_loop`、`manual_takeover`
- 说明 `human-steerable` 与主流 HITL、approval workflow、interrupt/resume、handoff 等术语的关系
- 定义人类观察、审批、纠偏、暂停、恢复和接管如何进入 `GoalContract`、`LoopController`、trace 与 audit
- 说明不同风险等级下如何选择和切换人机协作模式

该文档负责回答：agent 自动推进时，人类如何持续掌舵而不是只能事后补救。

### 8. AI 智能体效率工程

- `docs/AI_AGENT_EFFICIENCY_ENGINEERING.md`

职责：

- 定义效率工程不是单纯省 token
- 说明时间、速度、质量、代码实践、token、系统资源、人工介入、恢复效率之间的关系
- 给出第一阶段建议指标

该文档负责回答：如何在最短时间、最高质量、最小总成本下达成目标。

### 9. 插件与扩展架构

- `docs/PLUGIN_AND_EXTENSION_ARCHITECTURE_DISCUSSION.md`

职责：

- 说明未来如何接入主流模型、工具、skills、MCP、memory、retrieval、evaluator、policy、UI/control plane
- 说明插件不是信任边界
- 说明 capability、权限、审计、版本、回滚与兼容性要求

该文档负责回答：框架如何支持未来主流思想和插件生态接入。

### 10. 模型提示与检查点策略

- `docs/MODEL_PROMPT_AND_CHECKPOINT_POLICY.md`

职责：

- 定义多模型 `ModelProfile`、`PromptProfile`、`TaskPromptPack`
- 说明系统提示符多态化如何服务不同强弱模型
- 定义 `ProviderProfile`、`EndpointProfile`、`AgentRolePrompt`、`ResolvedPromptPack`
- 定义 `RoutePolicy`、`RouteDecision`、`ModelCallRecord`、`ModelEvaluation`
- 规定成本 / 延迟 / 质量 / 风险 / 隐私约束下的规则式路由
- 区分模型切换、能力兼容 fallback 和多模型协作
- 规定 prompt 版本、路由理由、调用结果和验证状态必须进入 trace
- 规定 `auth_ref` 只能引用外部 secret，secret 不进入 registry、prompt、trace 或 event log
- 说明固定检查点、事件触发检查点、风险触发检查点和防跑偏机制

该文档负责回答：不同模型如何用不同提示高遵循执行，如何按任务与预算选模型，如何协作或安全切换，以及全自主 loop 如何避免跑偏和破坏性改写。

关联文档：

- `docs/AGENT_ORCHESTRATION_PATTERNS.md`：路由、supervisor/orchestrator、handoff 与多 agent 编排模式
- `docs/AI_AGENT_EFFICIENCY_ENGINEERING.md`：ModelBudget、成本 / 延迟 / 质量指标
- `docs/AGENT_EVOLUTION_INNOVATION_SYNTHESIS.md`：模型画像、PromptProfile、评估和受治理进化
- `docs/PHASE_1_IMPLEMENTATION_SCOPE.md`：第一阶段真实最小接入与后续动态能力边界

### 11. Prompt / Checkpoint / DriftGuard 模板

- `docs/PROMPT_CHECKPOINT_TEMPLATES.md`

职责：

- 提供 `ModelProfile`、`PromptProfile`、`TaskPromptPack`、`CheckpointPolicy`、`DriftGuard` 模板
- 提供 Provider / Endpoint / AgentRole / ResolvedPrompt / Route / ModelCall 模板
- 提供固定检查项、事件触发检查项和 prompt 组合记录模板
- 为第一阶段 schema 和静态 registry 实现提供输入

该文档负责回答：这些策略对象第一阶段应长什么样。

### 12. Checkpoint 决策算法

- `docs/CHECKPOINT_POLICY_ALGORITHM.md`

职责：

- 定义自主 loop 中 fixed/event/risk/operator checkpoint 的决策算法
- 定义 checkpoint 层级、restore 策略和成本指标
- 汇总 checkpoint/rollback 相关论文精华与可用状态

该文档负责回答：自主推进时什么时候 checkpoint、保存什么、如何恢复。

### 13. Side Effect Ledger

- `docs/SIDE_EFFECT_LEDGER_DISCUSSION.md`

职责：

- 定义外部副作用记录、重放、补偿和恢复协调
- 防止 restore 后重复执行不可逆动作
- 服务安全智能体、运维智能体和开发智能体的高风险动作审计

该文档负责回答：外部动作发生后，checkpoint/restore 如何保持安全。

### 14. Drift Guard

- `docs/DRIFT_GUARD_DISCUSSION.md`

职责：

- 定义自主 loop 防跑偏、防过度重构、防破坏成熟代码的机制
- 定义 scope、architecture、quality、security、style/churn drift 检查
- 与 CheckpointPolicy、PromptProfile、Evaluator 联动

该文档负责回答：agent 如何避免用“合理理由”把项目改乱。

### 15. 参考框架与可观测性

- `docs/REFERENCE_FRAMEWORKS_AND_OBSERVABILITY.md`

职责：

- 记录 LangChain / LangGraph / LangSmith 等外部框架中值得吸收的工程能力
- 重点关注 tracing、observability、evaluation、persistence、interrupt/resume
- 明确这些参考不改变 `steerBox` 原有方向

该文档负责回答：成熟框架中哪些优点应该被吸收到 steerBox。

### 16. 智能体进化资料与现场自进化

- `docs/AGENT_EVOLUTION_READING_LIST.md`

职责：

- 整理国际与大陆/中文生态中关于智能体进化、harness 进化、skill/tool/memory 进化的高价值资料
- 判断实时现场自进化与已有研究的关系
- 为 `steerBox` 后续 Real-time Operational Self-Evolution 设计提供输入

该文档负责回答：智能体进化领域应该学习什么，以及现场自进化是否有独立工程价值。

### 17. 智能体进化创新吸收

- `docs/AGENT_EVOLUTION_INNOVATION_SYNTHESIS.md`

职责：

- 提炼高价值资料中“对方怎么做、创新点是什么”
- 将外部创新点转译为 `steerBox` 的 core object、plugin、event、evaluator 和 metric
- 形成开放融合路线，指导后续功能设计

该文档负责回答：外部资料的精华如何变成 `steerBox` 可实现、可验证、可扩展的能力。

### 18. 开放问题

- `docs/ARCHITECTURE_QUESTIONS.md`

职责：

- 保留尚未定稿的问题
- 防止讨论稿被误解为最终结论
- 后续每次设计收敛时更新问题状态

## 不覆盖原则

后续新增文档时，应遵守以下原则：

- 不用新文档覆盖旧文档的主线方向
- 不把参考文档当成项目方向决策者
- 不把讨论稿写成已实现能力
- 不把横向能力写成新的业务主线
- 不把未来愿景写成当前功能
- 不删除旧讨论，必要时通过新文档说明取舍和演进

## 当前文档缺口

后续可继续补充但当前尚未完成的文档：

- `docs/MEMORY_AND_DREAM_LEARNING_DISCUSSION.md`
- `docs/CODE_LOGIC_INDEX_DISCUSSION.md`
- `docs/SECURITY_MODEL_DISCUSSION.md`

这些缺口不影响当前文档体系成立，但进入实现前需要逐步补齐。
