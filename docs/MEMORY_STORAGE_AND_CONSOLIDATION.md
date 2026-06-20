# Memory, Storage, and Consolidation Discussion Draft

## 文档目的

本文件用于沉淀 `steerBox` 中多层存储、长期记忆、后台整理、记忆检索、向量/图索引和梦境学习式 consolidation 的设计方向。

本文件是讨论稿，不是最终实现规格，也不代表当前仓库已经具备对应能力。

它回答的问题是：

- 长时间运行的 agent 应该保存哪些东西
- 哪些数据必须同步可靠落盘，哪些可以异步整理
- 记忆能不能用向量存储
- 向量存储、结构化存储、事件日志、图索引、代码索引之间如何分工
- 后台 consolidation worker 应该做什么
- 第一阶段到未来高级阶段应如何递进实现

## 和 Storage / Recovery 文档的边界

`STORAGE_AND_RECOVERY_DISCUSSION.md` 主要回答：

- supervisor 如何守护 agent
- event / state / checkpoint 如何支撑恢复
- `RecoveryCheckpoint` 如何定义
- restore guarantee 如何分级
- 外框重启时如何避免丢状态

本文件主要回答：

- 运行过程中产生的事实、经验、错误、工具用法、代码知识、长期记忆如何分层保存
- 后台如何压缩、去重、归档、索引和验证记忆
- 记忆如何服务下一轮 context、retrieval、tool use、skill evolution 和 evaluator

简单说：

```text
Storage and Recovery -> 活下来、能恢复
Memory and Consolidation -> 活得久、记得准、越用越稳
```

## 当前主流趋势

### 1. checkpointer 和 store 分离

LangGraph 官方文档把 persistence 分成两类：

- `checkpointer`: 保存单个 thread / graph 的短期状态，用于连续对话、人在环、time travel、fault tolerance
- `store`: 保存 graph state 之外的跨 thread 长期数据，例如偏好、事实和共享知识

来源：https://docs.langchain.com/oss/python/langgraph/persistence

对 `steerBox` 的启发：

- 不要把 checkpoint、event log、long-term memory 混成一个表
- checkpoint 负责恢复执行位置
- store 负责跨任务复用知识
- event log 负责审计和重放事实链

### 2. agent state 必须持久化，不应只留在 context window

Letta 文档强调 stateful agent 会积累行为、事实和过去交互；消息、记忆、推理、工具调用都持久化到数据库，即使被 context window 驱逐也不会丢。

来源：https://docs.letta.com/guides/core-concepts/stateful-agents

对 `steerBox` 的启发：

- context window 只是工作区，不是存储层
- 被 compact / evict 的内容仍应可检索
- agent 应能通过工具读取和更新自己的部分记忆，但必须受 policy gate 管控

### 3. 记忆层级类似操作系统内存层级

MemGPT 把 LLM 记忆管理类比操作系统虚拟内存，通过不同 memory tier 之间移动信息来突破 context window 限制。

来源：https://arxiv.org/abs/2310.08560

对 `steerBox` 的启发：

- context 是热内存
- working memory 是当前任务工作集
- event log / transcript 是原始历史
- long-term memory 是整理后的可复用知识
- archive 是低频但可追溯原始数据

### 4. 生产级长期记忆通常结合提取、压缩、检索和治理

Mem0 论文与文档强调长期记忆不是简单保存聊天记录，而是动态提取、整合、检索显著信息，并通过平台能力提供向量存储、reranker、审计和治理。

来源：https://arxiv.org/abs/2504.19413
来源：https://docs.mem0.ai/platform/overview

对 `steerBox` 的启发：

- 长期记忆需要写入策略
- 记忆需要搜索策略
- 记忆需要审计、删除、更新和治理
- 向量库只是其中一层，不是完整记忆系统

### 5. 图记忆适合复杂关系和时间推理

Zep / Graphiti 方向强调 temporal knowledge graph，用于跨 session 信息综合、历史关系、结构化业务数据和非结构化对话的动态整合。

来源：https://arxiv.org/abs/2501.13956

对 `steerBox` 的启发：

- agent 记忆中有大量关系：谁影响谁、哪个工具适合哪个错误、哪个文件和哪个测试相关
- 向量相似度不擅长表达确定关系和时间演化
- 图索引适合 code relation、tool relation、task stage、incident timeline、cause-effect chain

### 6. 未来方向是 neuro-symbolic / hybrid memory

2026 年多篇资料都指向混合记忆：向量检索适合语义召回，结构化/符号/规则层适合确定性约束、逻辑查询和可解释推理。

参考：

- NS-Mem: https://arxiv.org/abs/2603.15280
- Episodic-Semantic Memory Architecture: https://arxiv.org/abs/2605.17625
- ProcMEM: https://arxiv.org/abs/2602.01869

对 `steerBox` 的启发：

- 未来不应押注单一向量库
- 应同时预留 semantic memory、episodic memory、procedural memory、symbolic rules、tool recipes、code graph
- 高风险决策不能只依赖向量相似度

## 记忆能不能用向量存储

可以，但不能只用向量存储。

向量存储适合：

- 语义相近内容召回
- 模糊问题查找
- 相似错误经验检索
- 相似工具用法检索
- 相似代码片段 / 文档片段检索
- 跨表达方式找相关内容

向量存储不适合单独承担：

- 事实源
- 审计日志
- 精确时间线
- 权限判断
- checkpoint 恢复
- side effect 记录
- 强一致状态
- 安全策略
- 可回滚版本链
- “必须找到全部相关项”的完整性查询

原因：

- 向量检索是相似度召回，不保证完整性
- embedding 模型升级会改变检索行为
- 相似不等于正确
- 召回结果需要 metadata filter、reranker、证据来源和置信度
- 高风险任务需要 deterministic query 或人工审批

因此，`steerBox` 应把向量库定位为：

```text
semantic retrieval index, not source of truth
```

也就是：向量索引用来帮助找到相关记忆，真正的事实、版本、来源、权限和审计仍在结构化存储与事件日志里。

## 推荐多层存储模型

### L0: Context Window

当前模型可见内容。

保存内容：

- 当前 GoalContract 摘要
- 当前 stage 状态
- 最近关键事件
- 必要工具说明
- 必要代码片段
- 必要记忆片段

特点：

- 最快
- 最贵
- 容量最小
- 必须严格预算

### L1: Working Memory

当前任务工作集。

保存内容：

- 当前计划
- 当前阶段目标
- 已读文件摘要
- 当前假设
- 待验证事项
- 临时 scratchpad

建议存储：

- SQLite state table
- 小型 JSON 文件
- 可重建的 task state

特点：

- 面向当前任务
- 可频繁更新
- 不等同长期记忆

### L2: Durable Event Log

事实源。

保存内容：

- loop iteration
- model call metadata
- tool call input/output 摘要
- policy decision
- human decision
- error
- verification result
- checkpoint created
- memory written
- side effect

建议存储：

- SQLite WAL
- append-only JSONL

特点：

- append-only
- 可审计
- 可重放
- 不应被向量库替代

### L3: Current State Store

当前有效状态。

保存内容：

- 当前 GoalContract
- 当前 agent/subagent 状态
- 当前 stage
- 当前权限
- 当前 checkpoint 指针
- 当前 memory policy

建议存储：

- SQLite

特点：

- 快速读取
- 可从 event log 重建
- 需要 schema 版本

### L4: Recovery Checkpoint Store

可恢复点。

保存内容：

- checkpoint metadata
- workspace snapshot reference
- state snapshot
- restore preconditions
- restore guarantees
- side-effect ledger cursor

建议存储：

- SQLite metadata
- filesystem artifacts
- optional compressed archive

特点：

- 支持恢复
- 不负责语义记忆
- 和 GoalAlignmentCheckpoint 分离

### L5: Episodic Memory

事件型长期记忆。

保存内容：

- 发生过什么
- 何时发生
- 当时目标是什么
- 使用了什么工具
- 结果如何
- 哪些验证通过 / 失败

适用：

- “上次类似任务怎么做的”
- “这个错误以前出现过吗”
- “这个工具之前怎么失败的”

建议存储：

- SQLite structured rows
- vector index for semantic retrieval
- metadata: goal_id, task_type, tool, outcome, confidence, timestamp

### L6: Semantic Memory

事实型长期记忆。

保存内容：

- 项目事实
- 架构原则
- 模块职责
- API 约定
- 代码约束
- 安全策略
- 工具能力描述

适用：

- “这个项目的约束是什么”
- “这个模块应该怎么改”
- “这个工具支持什么参数”

建议存储：

- SQLite / document store
- vector index
- optional graph relation

### L7: Procedural Memory

程序型 / 技能型记忆。

保存内容：

- 工具使用 recipe
- 常见错误处理步骤
- 验证流程
- task playbook
- skill activation condition
- skill execution steps
- termination condition

适用：

- “遇到这个错误下次怎么做”
- “这种任务应该按什么流程执行”
- “这个工具应该先查 help 再运行什么命令”

建议存储：

- versioned recipe files
- SQLite metadata
- test/eval result
- optional vector retrieval

注意：

- procedural memory 不能自动无审查升级成全局规则
- 高风险 recipe 需要验证和审批

### L8: Code Intelligence Index

代码逻辑索引。

保存内容：

- symbol graph
- call graph
- macro expansion mapping
- generated code mapping
- config-to-code mapping
- test-to-code mapping
- error-log-to-code mapping
- cross-language FFI mapping

适用：

- IDE/LSP 难以跳转的复杂关系
- C / Rust / Go / Python 混合项目
- 宏、生成代码、协议字段、配置驱动逻辑

建议存储：

- graph store or SQLite edge table
- vector index for semantic code search
- source span references

### L9: Graph / Relation Memory

关系型长期记忆。

保存内容：

- entity
- relation
- time range
- confidence
- source evidence
- supersedes relation
- conflicts_with relation

适用：

- 多跳推理
- 时间线推理
- 影响分析
- 漏洞链分析
- 工具/错误/修复之间的因果关系

建议存储：

- SQLite edge table in early stage
- graph database adapter later

### L10: Cold Archive

低频、原始、可追溯数据。

保存内容：

- 完整 transcript
- 原始 trace
- 历史 artifact
- 旧 checkpoint artifact
- 旧 memory version
- 被 supersede 的经验

建议存储：

- compressed JSONL
- compressed files
- content-addressed blobs

特点：

- 不进入常规 context
- 用于审计、恢复、复盘、重新索引

## 后台整理任务

后台整理不应影响热路径的可靠落盘。

热路径只做：

```text
append event -> update current state -> optionally create checkpoint -> return control
```

后台任务做：

- transcript compaction
- trace summary
- memory candidate extraction
- duplicate merge
- conflict detection
- stale memory detection
- confidence update
- vector embedding refresh
- graph edge extraction
- code index refresh
- tool recipe candidate generation
- skill candidate generation
- evaluation dataset generation
- cold archive compaction

## 写入路径建议

### 同步写入

必须同步写入：

- event log
- current state
- policy decision
- human decision
- tool result summary
- side effect ledger
- checkpoint metadata

原因：这些数据丢失会影响恢复、审计和安全。

### 异步写入

可以异步写入：

- vector embedding
- summary
- relation extraction
- long-term memory candidate
- code index refresh
- cold archive compression
- metrics aggregation

原因：这些数据可以延迟生成，失败后可以从 event log 重建。

### 缓冲队列

需要一个 `MemoryConsolidationQueue`。

字段建议：

```text
job_id
type
priority
source_event_range
source_artifact_ref
status
attempt_count
last_error
created_at
updated_at
output_refs
```

## 读取路径建议

读取不应只做 vector top-k。
## Context Packing and Retrieval Budget

`steerBox` 需要把“如何喂给后端 LLM”当成一等工程问题，而不是把所有可检索内容直接塞回 context。

### 目标

- 只把当前任务真正需要的内容送进 context window
- 尽量减少无关上下文、重复上下文和过期上下文
- 让跨会话记忆可检索，但不把每次检索都变成大段灌入
- 保持 token、latency 和 IO 成本稳定

### 核心原则

1. Context 不是日志回放。
2. Retrieval 不是全量倾倒。
3. Memory 不是“能搜到就全塞进去”。
4. 每一段进入 LLM 的内容都应有明确用途。
5. 先压缩、再过滤、再重排、最后装箱。

### 上下文打包流水线

```text
query intent
  -> scope filter
  -> time filter
  -> project / goal filter
  -> vector search
  -> keyword search
  -> graph expansion
  -> rerank
  -> deduplicate
  -> compress
  -> pack into context budget
```

### 打包时优先保留的内容

- 当前 GoalContract
- 当前 stage state
- 最近的关键事件
- 当前 diff 的最小必要片段
- 与当前动作直接相关的 tool result
- 失败模式的最小证据
- 必要的约束 / policy / checkpoint 摘要

### 默认应丢弃或延迟加载的内容

- 与当前 goal 无关的旧历史
- 重复的总结
- 没有来源的长段记忆
- 低置信度但高体积的召回结果
- 只对后续阶段才可能有用的内容
- 过多的原始 transcript

### 相关性控制

建议每次 context packing 都记录：

```text
packing_id
query_intent
selected_items
rejected_items
budget_tokens
estimated_value
irrelevant_context_ratio
compression_ratio
retrieval_precision
retrieval_recall
```

### User Confirmation for Critical Memory

对高价值但低置信度的内容，`steerBox` 应允许 agent 主动向用户确认，而不是擅自当作事实写入长期记忆。

适用场景：

- 关键信息是否应保留不明确
- 召回结果看起来相关，但证据不足
- 历史内容可能影响当前任务，但真实性未确认
- 压缩时无法判断哪些细节必须保留
- 任务中途混入了大量无关对话，可能污染当前上下文

推荐做法：

```text
ask_user_to_confirm
  -> critical_or_not?
  -> keep / drop / archive / review_later
```

可以让用户确认的问题包括：

- 这段信息是否属于当前任务的关键信息
- 是否应在 compaction 时保留
- 是否应写入长期记忆
- 是否应仅归档不参与后续检索
- 是否需要手动补充置信度

### Lightweight Local Model for Compaction

`steerBox` 可以使用本地专业小模型做低成本的 compaction / triage / labeling 工作，避免把所有整理任务都交给主模型。

适合本地小模型的工作：

- 摘要
- 去重
- 初步分类
- 低风险记忆候选提取
- 关键信息候选标注
- 污染检测
- context packing 预筛

不建议本地小模型直接做：

- 高风险决策
- 权限判断
- 安全策略变更
- 复杂恢复决策

### Context Hygiene Review

在长对话或多轮任务里，agent 应主动帮助用户识别“低置信度、非相关、污染上下文”的内容，并在用户认可后进行精准清理。

这不是简单清空聊天记录，而是面向当前任务的上下文卫生管理。

优先提示用户处理的内容包括：

- 与当前 goal 无关的片段
- 低置信度但体积很大的摘要
- 已被更高置信度内容替代的旧结论
- 会影响任务方向的插话
- 会占用大量 token 但短期内不会再用到的历史
- 可能让后端 LLM 误判当前约束的过期内容

推荐流程：

```text
mark candidate
  -> explain why it is low-confidence or irrelevant
  -> ask user confirm keep / prune / archive
  -> if confirmed, prune precisely
  -> if uncertain, keep in archive or pending review
```

### CleanupCandidate 显示字段

清理候选不应只显示编号，还应让用户一眼看到来源和影响。

建议字段：

```text
candidate_id
created_at
relative_time
source_turn_id
source_step_id
source_message_range
source_tool_calls
source_files
side_effect_type
source_summary
reason
confidence
estimated_tokens
impact_if_removed
recommended_action
```

示例展示：

```text
candidate-3
时间：2026-06-20 14:32:10，约 18 分钟前
来源：turn-42 / step-97
内容：中途讨论 Claude /context 输出
原因：与当前目标弱相关，占用 1.8k tokens
置信度：0.82
副作用：read_only
建议：archive，不直接删除
影响：移出当前 context 后不影响当前任务，可从 archive 调回
```

### 用户可选动作

用户应能选择：

```text
keep
archive
prune_from_context
delete_record
drop_without_storing
mark_as_critical
review_later
```

含义：

- `keep`: 保留在当前上下文
- `archive`: 移出当前上下文，但保存到归档，后续可调回
- `prune_from_context`: 从当前上下文清理，但保留事件记录
- `delete_record`: 删除候选记录，适合明确无价值内容
- `drop_without_storing`: 直接从上下文清掉，不进入长期记忆或归档
- `mark_as_critical`: 标记为关键内容，后续 compaction 必须保留
- `review_later`: 暂缓处理，进入待确认队列

设计要求：

- 默认不直接替用户删除
- 清理动作必须可审计
- 清理前应尽量保留可回溯引用
- 高价值但低置信度的内容优先放入待确认区
- 清理后应更新 context budget 和 retrieval state
- `drop_without_storing` 应只用于用户明确确认的非相关、低价值内容

### Mid-Conversation Pollution Handling

如果对话中途插入了大量无关话题，系统应优先提示用户处理，而不是默认帮用户“硬保留”。

清理建议应按优先级输出：

1. 非相关且低置信度的内容，优先建议清掉或隔离
2. 可能影响上下文方向的内容，优先建议移出当前 context
3. 可能影响上下文容量的长段内容，优先建议压缩或存档
4. 可能影响上下文正确性的争议内容，优先建议标记为待确认
5. 可能影响 LLM 高效做正确处理的噪声内容，优先建议剔除

系统可以提示用户是否需要：

- 清理当前上下文
- 把当前主线存档
- 将杂项对话隔离到独立线程
- 将有价值的杂项内容转入长期记忆候选
- 丢弃明显无关内容

但默认不应替用户直接执行不可逆清空；应先给出建议，再由用户确认。

这样可以避免：

- 当前任务上下文被杂项污染
- 长期记忆混入无关信息
- 关键事实被噪声淹没
- 后续检索召回偏移

### 实践建议

- 先摘要，再召回，不要直接全量喂原文
- 先结构化字段过滤，再做语义检索
- 对同类记忆做去重和合并
- 对长链上下文做阶段性压缩
- 对跨会话知识只加载当前任务需要的子集
- 对高风险动作加 policy filter 和 provenance check

### 对模型的现实影响

如果上下文装箱做得不好，后端 LLM 会出现：

- token 浪费高
- 注意力被无关信息稀释
- 相关证据淹没在长历史里
- 误把旧经验当新约束
- 频繁重复解释已知事实

所以，高效记忆系统的关键不只是“存得多”，而是“装得准、装得少、装得对”。


推荐流程：

```text
query intent classification
  -> structured filter
  -> vector search
  -> keyword / BM25 search
  -> graph expansion
  -> rerank
  -> provenance check
  -> policy filter
  -> context budget selection
```

其中：

- structured filter 控制 scope、project、task_type、risk、time
- vector search 找语义相关内容
- keyword search 找精确命令、错误码、文件名、函数名
- graph expansion 找关系邻居
- rerank 控制相关性和去重
- provenance check 确认来源
- policy filter 防止敏感或不可信记忆进入 context

## 向量存储方案

### 能做什么

向量存储可以把语义相近的记忆放到相近空间，检索时通过 embedding similarity 找到相关内容。

适合：

- 自然语言描述相似但措辞不同的经验
- 工具错误和修复方法
- 代码意图和文档说明
- 安全事件模式
- 日志模式
- 历史任务摘要

### 必须配合什么

向量记录必须带 metadata：

```text
memory_id
memory_type
source_event_id
source_trace_id
goal_id
project_id
task_type
created_at
updated_at
confidence
status
version
supersedes
conflicts_with
risk_level
visibility
embedding_model
chunk_hash
```

否则会出现：

- 查到旧记忆
- 查到错误记忆
- 查到不属于当前项目的记忆
- 查到被废弃的工具用法
- 查到无来源的总结

### 不建议做什么

不建议：

- 用向量库保存唯一原文
- 用向量相似度决定权限
- 用向量相似度决定是否执行高风险动作
- 不带来源地把 retrieved memory 放进 context
- embedding 模型升级后不重建索引

## 分层递进实现方案

### Phase 0: 文件级讨论稿

目标：先把设计边界想清楚。

做法：

- 保留本文档
- 不实现 runtime
- 明确 memory / storage / checkpoint / event / trace / index 的边界

### Phase 1: Reliable Local Store

目标：先可靠，不追求智能。

实现：

- SQLite WAL: events / state / memory metadata
- JSONL: trace / transcript
- filesystem: checkpoint artifacts
- append-only event log
- current state table
- minimal memory table

暂不做：

- 图数据库
- 分布式存储
- 自动进化
- 复杂向量库

### Phase 2: Async Consolidation Worker

目标：后台整理不阻塞主 loop。

实现：

- consolidation queue
- transcript summary
- tool error extraction
- memory candidate extraction
- duplicate detection
- stale memory marking
- cold archive compression

关键规则：

- worker 失败不影响 agent 主流程
- worker 输出必须带 source_event_range
- memory candidate 不等于 accepted memory

### Phase 3: Vector Retrieval Layer

目标：让 agent 能精准找回相关经验。

实现：

- embedding adapter
- vector index adapter
- metadata filter
- hybrid retrieval: vector + keyword
- reranker adapter
- provenance display

第一阶段可选技术：

- SQLite + sqlite-vec / sqlite-vss
- Qdrant
- Chroma
- LanceDB

选择原则：

- 本地优先
- 可重建索引
- metadata filter 强
- 资源占用低
- 不把向量库作为事实源

### Phase 4: Graph and Code Intelligence Layer

目标：支持复杂代码关系、工具关系和安全事件关系。

实现：

- entity table
- relation table
- temporal relation
- code symbol graph
- tool-error-fix relation
- incident timeline
- test-to-code relation

第一阶段可以先用 SQLite edge table，不必直接引入独立图数据库。

### Phase 5: Procedural Memory and Skill Evolution

目标：把经验转成可复用流程。

实现：

- tool recipe candidate
- skill candidate
- activation condition
- execution steps
- termination condition
- validation result
- rollout status

关键规则：

- 从错误中实时生成候选经验
- 低风险经验可局部生效
- 高风险经验必须审批和验证
- 经验可以回滚

### Phase 6: Dream Learning / Offline Consolidation

目标：低峰期或后台持续整理长期记忆。

实现：

- daily consolidation
- weekly architecture memory review
- conflict resolution
- supersedes chain cleanup
- memory quality evaluation
- forgotten / archived policy
- eval dataset generation

注意：

- dream learning 不应私自改全局 policy
- 输出应是 proposal，经过验证后再进入正式 memory / skill / policy

### Phase 7: Distributed / Enterprise Memory

目标：多用户、多 agent、多项目的可治理长期记忆。

实现：

- workspace isolation
- RBAC
- encrypted memory
- retention policy
- audit export
- multi-tenant vector / graph index
- remote sync
- privacy and deletion workflow

## 第一阶段推荐落地结构

建议第一阶段文件/库结构保持简单：

```text
.steerbox/
  state.sqlite
  events.sqlite
  traces/
    *.jsonl
  checkpoints/
    <checkpoint_id>/
  memory/
    candidates.jsonl
    accepted.jsonl
    rejected.jsonl
  indexes/
    vector/
    graph/
  archives/
```

说明：

- 这只是运行态数据目录建议，不是当前代码目录结构要求
- 后续实现时可以让路径可配置
- 不建议把运行态数据提交到 git

## Evolution Persistence: Cache, Candidate, Recipe, Registry

进化沉淀不应直接放在普通 cache 里。

但基础、稳定、高频、跨项目通用的工具用法不需要等失败后再进入进化链路。它们应作为 `baseline tool recipes` 预置，然后通过运行证据持续修正。

推荐分层：

```text
cache -> event log -> memory candidate -> accepted recipe -> skill / policy / tool registry
```

### Cache

用于短期加速。

适合保存：

- 工具输出缓存
- 网页抓取缓存
- embedding cache
- 中间摘要缓存

特点：

- 可丢
- 可重算
- 不作为事实源
- 不直接跨项目复用

### Event Log

用于记录事实来源。

应记录：

- tool call succeeded / failed
- user corrected tool usage
- recipe candidate created
- recipe accepted / rejected
- policy update proposed

特点：

- append-only
- 可审计
- 可回放
- 是经验沉淀的证据来源

### Memory Candidate

用于保存还未被接受的经验。

示例：

- 这个工具下次要先查 `--help`
- 这个命令不能裸跑
- 这个错误应先检查路径权限

特点：

- 不是正式规则
- 需要验证或用户确认
- 可以被后台 consolidation 处理

### Accepted Recipe

用于保存已验证、可复用的工具或流程经验。

建议字段：

```text
tool_id
task_type
correct_usage
preflight_checks
postflight_checks
known_errors
verification
scope
confidence
version
source_event_ids
last_verified_at
```

特点：

- 可跨会话复用
- 可被 agent 检索
- 可导出到别处使用
- 可回滚

### Skill / Policy / Tool Registry

用于更正式、更稳定的能力。

适合保存：

- skill activation condition
- tool profile
- allowed / forbidden actions
- approval requirement
- high-risk policy

规则：

- 低风险高频通用工具 recipe，可晋升到 global registry
- 高风险 rule / policy 不能自动全局生效
- 全局记录必须带 scope、version、source_event_ids、last_verified_at

## Global Tool Recipes

高频、通用、低风险、已验证的工具用法，可以晋升为全局记录。

适合全局记录：

- `rg` 搜索含 shell 特殊字符时使用 `-F` 或安全引用
- `git status --short --branch` 检查工作区
- JSON 格式检查
- 只读查询类命令
- 常见 shell / git / build 工具的安全用法

不适合直接全局记录：

- 当前项目专用路径
- 客户环境路径
- 高风险写操作
- 安全响应动作
- 未验证 workaround
- 需要权限提升的命令

## Global Project Registry and Cross-Project Memory

长周期大项目通常需要跨多个项目联调、联测、分析和调试。

因此，全局层应支持 `GlobalProjectRegistry`，但它不应保存每个项目的全部细节。

全局层保存：

```text
project_id
project_name
repo_path
language_stack
role
upstream_projects
downstream_projects
shared_protocols
shared_interfaces
test_entrypoints
debug_entrypoints
memory_index_ref
last_verified_at
```

原则：

- global memory 保存“去哪里找”和“项目之间如何关联”
- project memory 保存“该项目内部什么为真”
- cross-project memory 保存联调联测链路、接口契约、依赖关系和共享协议
- 当前项目不应默认加载其他项目全部上下文
- 只有当任务需要跨项目分析时，才按 registry 关系检索相关项目记忆

## 核心对象草案

### MemoryRecord

```text
memory_id
memory_type
content
summary
source_refs
project_id
goal_id
task_type
confidence
status
version
created_at
updated_at
expires_at
supersedes
conflicts_with
embedding_refs
graph_refs
policy_tags
```

### MemoryCandidate

```text
candidate_id
source_event_range
candidate_type
proposed_content
reason
evidence
confidence
risk_level
validation_plan
status
```

### ConsolidationJob

```text
job_id
job_type
input_refs
output_refs
status
attempt_count
last_error
created_at
updated_at
```

### RetrievalResult

```text
result_id
memory_id
score
retrieval_method
source_refs
confidence
staleness
policy_decision
context_budget_cost
```

## 关键设计原则

1. Event log 是事实源，向量库不是事实源。
2. Checkpoint 用于恢复，memory 用于复用经验，二者不要混淆。
3. 写入必须带来源，检索必须带 provenance。
4. 热路径要快，后台整理要可重试。
5. 记忆候选不等于正式记忆。
6. 正式记忆必须可更新、可废弃、可回滚。
7. 向量检索必须配合 metadata filter、keyword search、rerank 和 policy filter。
8. 高风险动作不能只基于相似记忆执行。
9. 工具错误应实时沉淀为候选经验。
10. dream learning 应输出 proposal，而不是静默改变规则。

## 当前结论

`steerBox` 应采用分层递进的记忆与存储架构。

短期最优方案不是直接上复杂知识图谱或分布式向量库，而是：

```text
SQLite WAL event/state
+ JSONL trace/transcript
+ checkpoint artifacts
+ memory candidate pipeline
+ async consolidation worker
+ optional vector index adapter
```

中长期应演进为：

```text
event-sourced durable store
+ semantic / episodic / procedural memory
+ vector retrieval
+ graph relation index
+ code intelligence index
+ dream learning consolidation
+ audited memory governance
```

向量存储可以用于“把语义相关的记忆放近、检索时找回相关信息”，但必须作为检索索引使用，不能替代事实源、权限系统、checkpoint、审计日志或确定性关系查询。
