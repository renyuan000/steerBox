# Reference Frameworks and Observability Notes

## 文档目的

本文件用于沉淀 `steerBox` 对外部智能体框架的可借鉴工程能力，当前重点关注 LangChain / LangGraph / LangSmith 中对我们有价值的部分。

本文件不是技术选型结论，不表示 `steerBox` 会基于 LangChain 实现，也不把 LangChain 作为项目方向来源。

它的作用是明确：哪些成熟框架能力值得吸收，哪些地方需要保持 `steerBox` 自己的边界。

## 当前结论

LangChain 生态对 `steerBox` 最有价值的不是“链式调用”本身，而是以下工程能力：

- tracing / observability
- evaluation / experiment / dataset
- production monitoring
- persistence / checkpoint / store
- interrupts / human steering
- graph state 与线程级恢复
- trace 到失败诊断与质量改进的闭环

对 `steerBox` 来说，最应该单独吸收的是：

> 可追踪、可验证、可恢复、可对比、可持续改进。

## 官方资料摘取

### 1. LangSmith Observability

`官方文档`

LangSmith Observability 提供从单个 trace 到生产级性能指标的可见性。它支持连接多种框架和模型供应商，并提供 tracing、trace 查看、性能监控、告警、自动化、反馈收集等能力。

来源：

- https://docs.langchain.com/langsmith/observability

`对 steerBox 的启发`

`steerBox` 应该从第一阶段就把 observability 设计成核心能力，而不是等 agent 能跑起来后再补日志。

至少应保留：

- trace id
- run id
- goal id
- loop id
- parent / child relation
- selected model
- tool call
- input / output summary
- cost metrics
- latency metrics
- policy decision
- evaluator result
- checkpoint event
- human decision
- side effect

### 2. LangSmith Evaluation

`官方文档`

LangSmith Evaluation 区分 offline evaluation 和 online evaluation：

- offline evaluation 用 curated datasets 在开发阶段比较版本、benchmark、发现 regression
- online evaluation 在生产交互中实时监控质量、发现问题和衡量效果

文档中还提到 dataset、evaluator、experiment、human review、code rules、LLM-as-judge、pairwise comparison、regression tests、backtesting、sampling 和 feedback loop。

来源：

- https://docs.langchain.com/langsmith/evaluation

`对 steerBox 的启发`

`steerBox` 的验证不应只停留在一次任务完成后的测试命令。

应区分：

- `offline evaluation`: 用历史任务、样例、回归集、攻击样本、安全日志样本验证 harness 版本
- `online evaluation`: 对长期运行中的真实 agent 事件进行持续抽样验证
- `human feedback`: 把人工审查和纠偏纳入数据集和后续评估
- `experiment comparison`: 比较不同模型、不同 loop 策略、不同 tool/skill/MCP 组合

这直接服务于 AI Agent Efficiency Engineering。

### 3. LangGraph Persistence

`官方文档`

LangGraph persistence 通过两类机制支持状态延续：

- checkpointers：保存 thread 的 graph state checkpoint，用于短期、线程级记忆、对话连续性、human-in-the-loop、time travel 和 fault tolerance
- stores：保存 graph state 之外的应用级数据，用于长期、跨 thread 的记忆，例如用户偏好、事实和共享知识

来源：

- https://docs.langchain.com/oss/python/langgraph/persistence

`对 steerBox 的启发`

`steerBox` 之前已经确定 `state.sqlite`、`events.sqlite`、`sessions/traces JSONL` 分层，这个方向与 LangGraph 的 checkpointer / store 分离思路一致。

可吸收点：

- thread-scoped short-term state 与 cross-thread long-term memory 分离
- checkpoint 用于恢复当前运行位置
- store 用于跨任务长期知识
- persistence 不只是 memory，还服务 fault tolerance、human steering 和 time travel

但 `steerBox` 需要更强调：

- event log source of truth
- side-effect ledger
- hard / soft checkpoint
- operator-marked checkpoint
- 安全场景下的审计与补偿边界

### 4. LangGraph Interrupts

`官方文档`

LangGraph interrupts 支持在 graph execution 中暂停，保存 graph state，等待外部输入后恢复。文档强调：

- interrupt 可用于 approval workflow、review/edit、tool call 前审批、human input validation
- checkpointer 保存状态用于恢复
- thread_id 是恢复同一运行的持久游标
- side effects called before interrupt must be idempotent

来源：

- https://docs.langchain.com/oss/python/langgraph/interrupts

`对 steerBox 的启发`

这与我们刚刚修正的人机介入模型一致：不是所有任务都强制人在环，而是支持无人值守、被动观察、主动审批、人工接管。

可吸收点：

- interrupt 是一等运行事件，不是简单 UI 弹窗
- interrupt 前后必须有 checkpoint / state persistence
- resume 必须绑定稳定 run/thread/session id
- interrupt payload 应结构化，方便前台控制面展示
- interrupt 前的 side effect 必须幂等或可补偿

## steerBox 应吸收的设计原则

### 1. Trace First

每一次关键动作都应进入 trace。

关键 trace 单元：

- goal created / updated
- loop iteration started / finished
- context retrieved
- model selected
- tool called
- MCP called
- policy decision made
- evaluator executed
- memory written
- checkpoint created
- human decision received
- side effect emitted
- restore executed

### 2. Evaluation as Product Loop

验证不是一次性测试，而是产品化循环。

建议形成：

```text
trace -> failure/sample selection -> dataset -> offline evaluation -> fix harness -> online evaluation -> feedback -> dataset
```

这对两个主线都重要：

- 软件开发智能体：历史 bug、失败任务、review 问题、回归样例可以进入 dataset
- 安全智能体：历史告警、误报、漏报、攻击模拟、事件响应样例可以进入 dataset

### 3. Experiment Comparison

每次 harness、tool、skill、model routing、memory policy、loop policy 变化，都应能够对比实验结果。

建议至少记录：

- experiment id
- dataset id
- harness version
- model routing policy
- tool registry version
- skill registry version
- memory policy version
- evaluator version
- metrics
- regression result

### 4. Online Evaluation With Sampling

长期运行系统不能每个事件都高成本评估。

需要支持：

- 按风险采样
- 按异常采样
- 按新策略灰度采样
- 按用户反馈采样
- 按低置信度采样

高风险事件可以强制 evaluator，低风险事件可以抽样。

### 5. State and Memory Separation

`steerBox` 应明确区分：

- `run state`: 当前运行状态
- `thread state`: 当前任务/会话的短期状态
- `event log`: 可审计事件来源
- `trace`: 可观察执行链路
- `long-term memory`: 跨任务复用知识
- `dataset`: 用于评估和回归的样例集合

不要把所有东西都叫 memory。

### 6. Interrupt / Resume as Core Runtime Capability

人工介入不应只是审批按钮，而应成为 runtime 能力。

需要设计：

- interrupt id
- interrupt reason
- interrupt payload
- current state snapshot
- required human action
- timeout policy
- resume payload
- resume target
- audit record

### 7. Side Effects Must Be Accounted For

LangGraph interrupts 文档提醒 side effects before interrupt must be idempotent。

`steerBox` 应进一步要求：

- 高风险 side effect 前尽量 checkpoint
- side effect 必须进入 ledger
- 恢复后必须知道 side effect 是否已经发生
- 无法回滚的动作必须主动审批或人工接管

## 不应直接照搬的地方

### 1. 不把 graph 结构当成唯一执行模型

LangGraph 的 graph 模型很适合显式 workflow，但 `steerBox` 还需要支持：

- 自主 loop
- subagent tree
- supervisor restart
- long-running security operations
- background consolidation
- codebase indexing

因此 graph 可以借鉴，但不应成为唯一抽象。

### 2. 不把 observability 外包给 SaaS

LangSmith 是成熟产品，但 `steerBox` 应保留本地、可自托管、可审计的基础 trace/event 结构。

可以吸收理念，不应依赖外部 SaaS 才能运行。

### 3. 不把 evaluation 简化成 LLM-as-judge

LLM-as-judge 有价值，但高可靠系统还需要：

- deterministic tests
- code rules
- security rules
- replay / simulation
- human review
- regression datasets
- provenance checks

### 4. 不把 human-in-the-loop 当成唯一模式

`steerBox` 要支持：

- unattended
- passive oversight
- active approval
- manual takeover

不同模式由 GoalContract、PolicyGate、risk level 和运行环境决定。

## 对现有文档的映射

### 已覆盖

- `ARCHITECTURE_DIRECTION.md`: 总方向、四个专业研发方向、人类可掌舵运行模式
- `STORAGE_AND_RECOVERY_DISCUSSION.md`: 持久化、checkpoint、restore、supervisor
- `GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md`: GoalContract、LoopController、控制门
- `AI_AGENT_EFFICIENCY_ENGINEERING.md`: 效率指标、成本、质量、恢复、多模型路由
- `PLUGIN_AND_EXTENSION_ARCHITECTURE_DISCUSSION.md`: 模型、工具、skill、MCP、memory、evaluator、policy、control plane 插件

### 仍需补齐

- Evaluation dataset schema
- Trace schema
- Experiment schema
- Online evaluation sampling policy
- Human feedback data model
- Side-effect ledger schema

## 建议后续新增文档

后续可以继续拆出：

- `docs/OBSERVABILITY_AND_EVALUATION_DISCUSSION.md`
- `docs/TRACE_SCHEMA_DISCUSSION.md`
- `docs/EVALUATION_DATASET_DISCUSSION.md`

当前本文件先作为参考框架吸收记录，避免过早拆太多文件。

## 当前结论

应该单独准备这个文档。

原因是 LangChain / LangGraph / LangSmith 的优势点正好对应 `steerBox` 的长期核心能力：

- 可追踪
- 可验证
- 可恢复
- 可对比
- 可持续改进

但这些能力应被吸收到 `steerBox` 的 harness、goal、loop、efficiency、plugin 和 storage/recovery 体系中，而不是把项目方向改成“基于 LangChain 的应用框架”。
