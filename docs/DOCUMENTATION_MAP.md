# steerBox Documentation Map

## 文档目的

本文件用于说明 `steerBox` 文档体系如何拆分、如何阅读、以及哪些文档负责主线、哪些文档负责横向支撑能力。

本文件不替代任何已有文档，只负责组织关系。

## 阅读顺序

建议按以下顺序阅读：

1. `README.md`
2. `docs/ARCHITECTURE_DIRECTION.md`
3. `docs/TRACKS_AND_CAPABILITIES.md`
4. `docs/HARNESS_PRINCIPLES.md`
5. `docs/STORAGE_AND_RECOVERY_DISCUSSION.md`
6. `docs/GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md`
7. `docs/GOAL_ALIGNMENT_CHECKS.md`
8. `docs/AI_AGENT_EFFICIENCY_ENGINEERING.md`
9. `docs/PLUGIN_AND_EXTENSION_ARCHITECTURE_DISCUSSION.md`
10. `docs/MODEL_PROMPT_AND_CHECKPOINT_POLICY.md`
11. `docs/PROMPT_CHECKPOINT_TEMPLATES.md`
12. `docs/CHECKPOINT_POLICY_ALGORITHM.md`
13. `docs/SIDE_EFFECT_LEDGER_DISCUSSION.md`
14. `docs/DRIFT_GUARD_DISCUSSION.md`
15. `docs/REFERENCE_FRAMEWORKS_AND_OBSERVABILITY.md`
16. `docs/AGENT_EVOLUTION_READING_LIST.md`
17. `docs/AGENT_EVOLUTION_INNOVATION_SYNTHESIS.md`
18. `docs/ARCHITECTURE_QUESTIONS.md`
19. `docs/HARNESS_SOURCES.md`

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

### 6. Goal 与 Loop Engineering

- `docs/GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md`

职责：

- 定义 `GoalContract`
- 定义 `LoopController`
- 定义 observe / goal alignment checkpoint / plan / act / record / verify / reflect / update / recovery checkpoint decision / next action 循环
- 说明 policy、tool、model、budget、verification、recovery 控制门

该文档负责回答：agent 如何围绕明确目标高效闭环推进。

### 7. 目标对齐检查点

- `docs/GOAL_ALIGNMENT_CHECKS.md`

职责：

- 区分 `RecoveryCheckpoint` 和 `GoalAlignmentCheckpoint`
- 定义长程自主 loop 中用于检查和纠正 root goal、stage goal、stage chain 偏离的机制
- 定义 aligned / weakly_aligned / drifting / off_goal / unclear 等方向判断

该文档负责回答：agent 如何在长程任务中持续确认自己没有偏离最初目标。

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
- 说明固定检查点、事件触发检查点、风险触发检查点和防跑偏机制

该文档负责回答：不同模型如何用不同提示高遵循执行，以及全自主 loop 如何避免跑偏和破坏性改写。

### 11. Prompt / Checkpoint / DriftGuard 模板

- `docs/PROMPT_CHECKPOINT_TEMPLATES.md`

职责：

- 提供 `ModelProfile`、`PromptProfile`、`TaskPromptPack`、`CheckpointPolicy`、`DriftGuard` 模板
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
- `docs/CONTROL_PLANE_DISCUSSION.md`

这些缺口不影响当前文档体系成立，但进入实现前需要逐步补齐。
