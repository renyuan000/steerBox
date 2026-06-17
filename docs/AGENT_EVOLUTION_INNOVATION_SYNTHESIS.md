# Agent Evolution Innovation Synthesis

## 文档目的

本文件把高价值文章和论文中的方法拆成三层：

1. 对方是怎么做的
2. 关键创新点是什么
3. `steerBox` 应如何开放式融合成自己的功能

本文件不是论文复述，也不是最终实现规格。它用于指导后续架构设计和功能拆解。

## 总体吸收策略

`steerBox` 不应简单复刻任何一个框架或论文。

更合理的吸收方式是：

```text
外部创新点 -> steerBox 抽象对象 -> 插件/策略/事件/指标 -> 可验证实现
```

也就是说，每个外部方法都要被转译成：

- 一个清晰的 core object
- 一个可插拔 extension point
- 一个可审计 event
- 一个可验证 evaluator
- 一个可回滚 evolution proposal
- 一个可量化 efficiency metric

## 创新吸收矩阵

### 1. OpenAI Harness Engineering

来源：

- https://openai.com/index/harness-engineering/

对方怎么做：

- 把 repository 知识作为 system of record
- 把 AGENTS.md 作为入口和目录，而不是巨型规则百科
- 把 logs、metrics、traces、UI、debugging surface 暴露给 agent
- 用机械化约束、lint、CI、结构测试和 doc gardening 防止规则漂移
- 接受 agent 会复制坏模式，因此需要持续清理 entropy

关键创新点：

- 把 agent 效果从 prompt 问题转成 repo/harness/feedback 问题
- 把人类角色从写代码转成 steering、审查和环境设计
- 把文档、规则、工具、观测、验证统一成 agent 可见的工程系统

steerBox 融合方式：

- `RepoKnowledgeIndex`: 把 docs、rules、plans、decisions、quality state 纳入可检索知识
- `AgentReadableDocs`: 约束 README/AGENTS.md 只做入口，深层知识进入 docs
- `MechanicalEnforcementPlugin`: 将规则逐步转成 lint、schema、policy gate、CI/evaluator
- `EntropyGarbageCollector`: 周期性发现坏模式、重复文档、过期规则、AI slop
- `TraceVisibleRuntime`: 让 agent 能读取任务运行态、日志、trace、指标和验证结果

第一阶段可做：

- 文档地图
- rules/docs 分层
- trace/event 基础结构
- evaluator 结果写入文档或事件库

### 2. Anthropic Long-Running Harnesses

来源：

- https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents
- https://www.anthropic.com/engineering/harness-design-long-running-apps

对方怎么做：

- 用 initializer、启动脚本、进度文件和 git history 帮后续 agent 继续任务
- 把长任务拆成 sprint 或 contract
- 让 generator 和 evaluator 协商完成标准
- 用 evaluator 检查结果，而不是让 generator 自己宽松打分
- 根据模型能力调整 context reset、compaction 和 evaluator 策略

关键创新点：

- 长任务不是一个大 prompt，而是一系列可交接、可验证、可恢复的阶段
- `done` 必须先定义
- evaluator 不是固定模式，而是根据任务风险和模型能力动态启用

steerBox 融合方式：

- `GoalContract`: objective、scope、success_criteria、constraints、risk_level、verification_plan
- `HandoffArtifact`: 当前状态、完成项、未完成项、风险、验证结果、下一步
- `EvaluatorPolicy`: 根据 risk/model/cost 决定 evaluator 是否开启
- `SprintLikeLoop`: 把长任务拆成可验证 loop segment，而不是无限运行
- `ContinuationProtocol`: 新 agent / subagent 启动时读取状态、事件、checkpoint、handoff

第一阶段可做：

- GoalContract schema 草案
- LoopController 状态机
- VerificationResult 结构
- HandoffArtifact JSON/Markdown 双格式

### 3. LangSmith / LangGraph

来源：

- https://docs.langchain.com/langsmith/observability
- https://docs.langchain.com/langsmith/evaluation
- https://docs.langchain.com/oss/python/langgraph/persistence
- https://docs.langchain.com/oss/python/langgraph/interrupts

对方怎么做：

- LangSmith 把 trace、run、dataset、experiment、evaluator、feedback、online/offline evaluation 产品化
- LangGraph 用 checkpointer 保存 thread state，用 store 保存跨 thread 数据
- LangGraph interrupt 支持执行中暂停、等待人类输入、恢复执行
- persistence、interrupt、evaluation 都和 runtime 集成，而不是外部日志系统

关键创新点：

- observability 与 evaluation 是开发工作流的一部分
- checkpoint/store 分离，使短期状态与长期记忆边界清晰
- interrupt/resume 是 runtime primitive，不是 UI 附属功能

steerBox 融合方式：

- `TraceStore`: run/loop/tool/model/policy/evaluator/checkpoint trace
- `EvaluationDataset`: 历史任务、失败样例、攻击样本、安全日志样例
- `ExperimentRunner`: 对比 model routing、loop policy、tool registry、memory policy
- `CheckpointStore`: 线程/任务级恢复点
- `LongTermStore`: 跨任务记忆与知识
- `InterruptEvent`: 人工审批、人工输入、人工接管的结构化事件

第一阶段可做：

- Trace schema 草案
- EvaluationResult schema
- InterruptEvent schema
- 将 offline/online evaluation 写入文档规划

### 4. MCP

来源：

- https://modelcontextprotocol.io/specification/2025-06-18

对方怎么做：

- 定义 host / client / server 结构
- server 暴露 tools、resources、prompts
- client 可提供 roots、sampling、elicitation 等能力
- 官方安全原则强调用户同意、数据隐私和工具安全

关键创新点：

- 把工具和资源接入标准化
- 将 agent 能力外部化成可连接的 server
- 让 tool/resource/prompt 成为可协议化暴露的能力

steerBox 融合方式：

- `McpRegistry`: 注册 MCP server、capability、权限、启动成本、健康状态
- `McpPolicyGate`: MCP 调用前检查权限、数据范围、风险和 side effect
- `McpTraceEvent`: 记录 server、tool、input summary、output summary、latency、error
- `McpCostMonitor`: 空闲成本、启动成本、调用成本进入效率指标
- `McpPoisoningDefense`: 防 tool description injection、resource poisoning、越权访问

第一阶段可做：

- MCP manifest schema
- MCP server 默认 disabled
- MCP tool 调用事件结构

### 5. Agentic Harness Engineering

来源：

- https://arxiv.org/abs/2604.25850

对方怎么做：

- 用 observability 驱动 harness 自动进化
- component observability：把 prompt、tools、memory、middleware 等组件显式化、可编辑、可回滚
- experience observability：把长 trajectory 压缩成可钻取证据
- decision observability：每次 harness 编辑都带有预测，后续用任务结果验证
- 用 evolutionary tree 管理候选变体和选择过程

关键创新点：

- 自动进化不是“随便改 prompt”，而是对 harness 组件做可观测、可验证、可回滚的实验
- 进化需要预测和结果验证
- 收益主要来自 tools、middleware、long-term memory，而不只是 system prompt

steerBox 融合方式：

- `EvolutionProposal`: 目标组件、触发证据、预期收益、风险、验证计划、回滚计划
- `ComponentRegistry`: prompt/tool/skill/MCP/memory/policy/loop/model routing 都作为组件注册
- `ExperienceDigest`: 从 trace 中生成可钻取经验摘要
- `DecisionRecord`: 每次进化记录预测和实际结果
- `EvolutionExperiment`: 小范围验证候选修改，成功后 promotion

第一阶段可做：

- EvolutionProposal schema
- ComponentRegistry 文档和静态 manifest
- trace -> lesson candidate 的最小流程

### 6. Self-Harness

来源：

- https://arxiv.org/abs/2606.09498

对方怎么做：

- 从执行 trace 中挖掘模型弱点
- 生成 harness 修改提案
- 通过 regression testing 验证提案
- 为不同模型生成不同的 harness policy

关键创新点：

- harness 不应是模型无关的固定模板
- 模型弱点应被显式建模
- 候选 harness 修改必须经 regression 验证

steerBox 融合方式：

- `ModelProfile`: 记录模型能力、弱点、成本、延迟、工具调用质量、代码质量、安全分析质量
- `WeaknessMiner`: 从失败 trace 中识别模型常见错误模式
- `ModelSpecificPolicy`: 不同模型使用不同 context、tool、evaluator、verification 策略
- `RegressionGate`: harness 改动通过回归样例后才能 promotion

第一阶段可做：

- ModelProfile schema
- model routing 中加入 historical_success_rate、failure_patterns
- 失败样例进入 evaluation dataset

### 7. Continual Harness

来源：

- https://arxiv.org/abs/2605.09998

对方怎么做：

- agent 在单次长运行中持续改进 prompt、subagents、skills、memory
- 从历史 trajectory 中识别可以改进的 harness 部分
- 不等待下一个大版本或离线批处理才学习

关键创新点：

- 进化可以发生在运行中
- harness 状态本身可以随任务演进
- prompt、skills、subagents、memory 都可作为在线改进对象

steerBox 融合方式：

- `OnlineEvolutionQueue`: 运行中产生 evolution candidate
- `RealTimeLessonCandidate`: 工具错误、验证失败、人工纠偏立即结构化
- `PromotionGate`: 低风险本地 recipe 可快速生效，高风险进入审批和回归
- `ConsolidationJob`: 日/周级合并重复经验，清理冲突和过期规则

第一阶段可做：

- 工具错误现场记录
- lesson candidate 文件或表
- local recipe 更新必须带验证命令和回滚点

### 8. SkillHone

来源：

- https://arxiv.org/abs/2606.08671

对方怎么做：

- 把 skill 进化过程中的诊断、修订、测试、结果和被拒绝方案都保存为 persistent decision history
- 用不同角色的 subagents 对 skill 候选进行测试、批评和修订
- 不只保留最终 skill，而是保留为什么这么改

关键创新点：

- skill 不是静态 prompt，而是有版本、有历史、有证据的可进化资产
- 被拒绝方案也有价值，因为它解释了边界
- 进化过程本身是知识

steerBox 融合方式：

- `SkillRegistry`: skill name、version、trigger、required tools、verification、risk、owner
- `SkillDecisionHistory`: diagnosis、proposal、test result、accepted/rejected reason
- `SkillEvaluator`: 对 skill 候选做回归和场景测试
- `SkillBoundary`: when_to_apply / when_not_to_apply

第一阶段可做：

- skill manifest 草案
- skill 修订必须记录 old/new、原因、证据、验证结果

### 9. Recursive Agent Harnesses

来源：

- https://arxiv.org/abs/2606.13643

对方怎么做：

- 把递归单元定义为完整 agent harness，而不是普通模型调用
- parent agent 可以生成脚本、调度多个子 harness 并行工作
- 用子 harness 分解细粒度任务，提高复杂任务处理能力

关键创新点：

- subagent 应该有自己的工具、状态、trace、权限和恢复边界
- 多 agent 编排不等于聊天群，而是 harness 递归

steerBox 融合方式：

- `AgentProcess`: agent/subagent identity、parent、capability、state、trace、checkpoint
- `SubHarness`: 子 agent 运行在同一套 policy、event、checkpoint 体系下
- `TraceTree`: parent/child 任务树可审计
- `SubagentBudget`: 子 agent 消耗的 token、时间、工具调用、资源进入总预算

第一阶段可做：

- AgentProcess schema
- parent/child trace id
- subagent 默认继承最小权限

### 10. Meta-Engineering Harnesses

来源：

- https://arxiv.org/abs/2605.25665

对方怎么做：

- 以 contract-driven verification architecture 管理 AI-native software production
- 将 operational requirements 转成 explicit contracts
- 使用 role-specialized agents 和 independent/adversarial verification
- 结构化 failure classification，进入 outer-loop calibration

关键创新点：

- contract 是生产级智能体的中心对象
- verification 应独立甚至对抗式
- 失败分类是后续改进的输入

steerBox 融合方式：

- `GoalContract` 扩展为 contract-driven verification 的核心
- `AdversarialEvaluator`: 高风险任务引入对抗检查
- `FailureTaxonomy`: 工具错误、模型错误、上下文错误、权限错误、验证错误、恢复错误分类
- `OuterLoopCalibration`: 基于失败分类调整 policy、tool、model routing、memory、loop

第一阶段可做：

- FailureCategory 枚举
- VerificationResult 增加 failure_category
- 高风险安全动作引入 adversarial review 占位

### 11. Reflexion

来源：

- https://arxiv.org/abs/2303.11366

对方怎么做：

- 任务失败后生成 verbal reflection
- 把反思作为 episodic memory 存储
- 后续 trial 读取反思，改进决策
- 不更新模型权重

关键创新点：

- 用语言记忆替代权重更新，实现轻量自我改进
- 失败经验可以跨尝试复用

steerBox 融合方式：

- `ReflectionNote`: 绑定原始事件、失败原因、适用条件、不适用条件
- `EvidenceBoundMemory`: 反思必须可追溯到 trace/event
- `ReflectionGate`: 反思默认是 candidate，不直接变成 policy

第一阶段可做：

- failure -> reflection candidate
- reflection 必须包含 evidence 和 confidence

### 12. Voyager

来源：

- https://arxiv.org/abs/2305.16291

对方怎么做：

- 在开放环境中持续探索
- 通过 automatic curriculum 选择任务
- 通过 iterative prompting 和 execution feedback 生成/修复程序
- 把成功程序沉淀为 skill library

关键创新点：

- skill 是可执行、可组合、可检索的程序资产
- 执行反馈和自验证驱动 skill 增长
- curriculum 控制探索方向

steerBox 融合方式：

- `ExecutableSkill`: skill 不只是说明文字，可以是脚本、工具链、流程模板
- `SkillLibrary`: 可检索、可版本化、可验证
- `CurriculumPolicy`: 控制学习任务，从低风险、可验证任务开始
- `ExecutionFeedback`: 工具执行结果直接影响 skill 修订

第一阶段可做：

- skill registry 支持 command/script/workflow 类型
- skill 使用前后记录效果指标

### 13. Self-Evolution Survey / Long-Term Memory

来源：

- https://arxiv.org/abs/2404.14387
- https://arxiv.org/abs/2410.15665

对方怎么做：

- 将自进化拆成 experience acquisition、experience refinement、updating、evaluation
- 强调长期记忆是 inference-time self-evolution 的基础
- 关注 interaction data 的保存、管理、提炼和再利用

关键创新点：

- 自进化不是一个动作，而是数据获取、经验提炼、更新和评估的闭环
- 长期记忆是可持续改进的底座

steerBox 融合方式：

- `ExperienceEvent`: 原始经验采集
- `DreamConsolidator`: 离线整理、去重、冲突检测、摘要、索引
- `MemoryPromotionGate`: 从 raw -> summary -> pattern -> policy candidate -> accepted policy
- `MemoryEvaluator`: 检查记忆是否提升效率或引入错误

第一阶段可做：

- memory 分层 schema
- raw event 不被 summary 覆盖
- accepted policy 必须可追溯

### 14. ModelScope-Agent

来源：

- https://arxiv.org/abs/2309.00986

对方怎么做：

- 建立可定制 agent 框架
- 支持 tool-use data collection、tool retrieval、tool registration、memory control、customized model training、evaluation
- 连接模型社区和工具生态

关键创新点：

- tool retrieval 和 tool registration 是 agent 能力核心
- evaluation 与模型/工具生态连接

steerBox 融合方式：

- `ToolRegistry`: 工具注册、权限、schema、side effect、版本
- `ToolRetriever`: 按 GoalContract 检索工具
- `ToolUsageDataset`: 工具调用成功/失败样本用于后续评估
- `ModelRouter`: 多模型接入与任务分配

第一阶段可做：

- tool manifest schema
- tool 调用事件与失败率指标

### 15. AgentScope

来源：

- https://arxiv.org/abs/2402.14034
- https://arxiv.org/abs/2407.17789

对方怎么做：

- developer-centric multi-agent platform
- actor-based distributed framework
- service functions、utility monitor、fault tolerance、tools、external knowledge
- 大规模 multi-agent simulation 支持 web monitor 和并行执行

关键创新点：

- 多 agent 系统需要运行时、监控、容错和资源管理
- 大规模 agent 不是多开几个进程，而是调度与观测问题

steerBox 融合方式：

- `Supervisor`: 极简外框，管理 agent/subagent 生命周期
- `AgentProcess`: 身份、父子关系、状态、权限、trace、checkpoint
- `ResourceBudget`: CPU、内存、IO、sidecar、并发数限制
- `AgentMonitor`: 存活、资源、失败、重启、效率指标

第一阶段可做：

- supervisor 设计文档
- agent process state schema
- 空闲资源占用指标

## steerBox 应开放融合的功能包

### 1. Observability Pack

吸收来源：OpenAI、LangSmith、AHE、AgentScope

功能：

- TraceStore
- EventLog
- MetricsStore
- TraceTree
- RunViewer / ControlPlane adapter

最小对象：

```text
TraceEvent
RunId
LoopId
GoalId
ParentTraceId
ToolCallTrace
ModelCallTrace
PolicyDecisionTrace
EvaluatorTrace
CheckpointTrace
```

### 2. Verification Pack

吸收来源：Anthropic、LangSmith、Meta-Engineering Harnesses

功能：

- EvaluatorRegistry
- VerificationResult
- OfflineEvaluation
- OnlineEvaluation
- RegressionDataset
- AdversarialEvaluator

最小对象：

```text
VerificationPlan
EvaluationDataset
EvaluationCase
EvaluationResult
FailureCategory
RegressionGate
```

### 3. Real-time Evolution Pack

吸收来源：Continual Harness、Self-Harness、SkillHone、Reflexion、Voyager，以及用户提出的现场自进化设计

功能：

- ToolUseErrorEvent
- LessonCandidate
- ToolUsageRecipe
- OnlineEvolutionQueue
- PromotionGate
- EvolutionProposal

最小闭环：

```text
error -> classify -> verify correct usage -> write lesson candidate -> update recipe -> evaluate later -> promote or revert
```

### 4. Memory and Dream Learning Pack

吸收来源：Self-Evolution Survey、Long-Term Memory、Reflexion、Voyager、AHE

功能：

- MemoryStore
- ExperienceEvent
- EpisodeSummary
- PatternMemory
- FailureMemory
- ToolMemory
- DreamConsolidator
- MemoryPromotionGate

最小闭环：

```text
raw experience -> summary -> pattern -> candidate policy -> verified policy
```

### 5. Plugin and Capability Pack

吸收来源：MCP、ModelScope-Agent、OpenAI harness、LangGraph

功能：

- PluginManifest
- Capability
- ToolRegistry
- SkillRegistry
- McpRegistry
- ModelRegistry
- ModelRouter
- PermissionScope

关键要求：

- 插件不是信任边界
- 新插件默认 disabled
- 权限提升需要审批
- 所有插件调用进入 trace

### 6. Multi-Agent Harness Pack

吸收来源：Recursive Agent Harnesses、AgentScope、AgentVerse

功能：

- AgentProcess
- SubHarness
- Supervisor
- TraceTree
- SubagentBudget
- AgentMonitor

关键要求：

- subagent 必须运行在 harness 中
- parent/child 关系可审计
- 子 agent 权限默认最小化
- 子 agent 消耗进入总预算

### 7. Code Intelligence Pack

吸收来源：OpenAI agent-legibility 思想、用户提出的代码逻辑组织工程、Voyager skill library

功能：

- CodeLogicIndex
- SymbolGraph
- CallGraph
- MacroExpansionMap
- GeneratedCodeMap
- ConfigToCodeMap
- TestToCodeMap
- ErrorLogToCodeMap

关键要求：

- 不只依赖 LSP
- 索引结果带来源和置信度
- 服务 retrieval、evaluator、memory、impact analysis

## 开放融合原则

### 1. 先抽象对象，再选实现

不要一开始就决定用哪个框架。

先定义：

- GoalContract
- LoopEvent
- TraceEvent
- PluginManifest
- ToolUsageRecipe
- EvolutionProposal
- EvaluationCase

再决定是否参考 LangGraph、MCP、SQLite、JSONL、vector DB、graph store、local model 等实现。

### 2. 每个创新点都要有证据链

吸收外部创新时，必须回答：

- 来源是什么
- 它解决什么问题
- 它怎么做
- 它的风险是什么
- `steerBox` 如何改造
- 如何验证有效
- 如何回滚

### 3. 不把论文能力写成当前能力

文档可以写方向和候选对象，但 README 和架构总纲不能暗示已经实现。

### 4. 不把单一框架绑定成核心

LangChain、LangGraph、MCP、AgentScope、ModelScope-Agent 都可以借鉴，但 `steerBox` 核心应保持：

```text
minimal core + plugin adapters + policy gates + event/trace + checkpoint/recovery
```

### 5. 实时进化和周期进化并存

- real-time micro-evolution: 现场错误、工具误用、help 未查、参数错误、低风险 recipe 修正
- daily consolidation: 合并重复经验、去重、冲突检测、更新 memory
- weekly architecture evolution: 调整 harness、policy、model routing、plugin、skill、evaluator

## 第一阶段建议落地顺序

### Step 1: Schema First

先定义最小 schema：

- GoalContract
- LoopEvent
- TraceEvent
- ToolUseErrorEvent
- LessonCandidate
- ToolUsageRecipe
- PluginManifest
- ModelProfile
- VerificationResult
- EvolutionProposal

### Step 2: Local Files First

第一阶段可以先用本地文件：

- SQLite for state/events
- JSONL for traces/sessions
- Markdown for human-readable docs
- YAML/TOML/JSON for manifests

不急于引入外部服务。

### Step 3: Policy Gate Before Automation

在自动改 tool/skill/memory/policy 前，先有：

- risk level
- promotion gate
- rollback plan
- verification status

### Step 4: Evaluation Before Promotion

所有可持续影响 agent 行为的变更，都应至少经过一种验证：

- minimal command verification
- regression dataset
- evaluator review
- human approval

### Step 5: Plugin Adapters Later

先把抽象稳定，再接入：

- MCP
- LangGraph-like workflow
- LangSmith-like observability
- local model runtime
- vector / graph memory

## 当前结论

之前的阅读清单已经列出了资料和核心价值，但不足以指导实现。

本文件补齐的是：

- 对方怎么做
- 创新点是什么
- `steerBox` 如何融合
- 应抽象成哪些对象
- 第一阶段如何落地

下一步如果继续细化，最应该拆出的实现讨论稿是：

- `docs/REAL_TIME_SELF_EVOLUTION_DISCUSSION.md`
- `docs/OBSERVABILITY_AND_EVALUATION_DISCUSSION.md`
- `docs/MEMORY_AND_DREAM_LEARNING_DISCUSSION.md`
- `docs/CODE_LOGIC_INDEX_DISCUSSION.md`
