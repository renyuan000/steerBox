# Memory Scope and Lifecycle Contracts

## 文档定位

本文件把 `MEMORY_STORAGE_AND_CONSOLIDATION.md` 的讨论方向收敛为可实现的边界。它是 memory schema、可见性、生命周期和 promotion 的 Phase 1 约束补充；不代表当前仓库已经实现持久记忆、向量索引、图记忆或跨项目共享。

核心原则：

```text
事件是事实源，状态是当前投影，checkpoint 用于恢复，memory 用于复用经验。
```

## 一、四层可见性边界

### 1. Run / Working Memory

当前 `LoopRun`、`TaskRun` 或 `Attempt` 的临时工作集，包括当前计划、已读摘要、待验证事项、假设、工具结果摘要和当前 Goal 上下文。

- 默认只对当前 run 可见；
- 任务结束后可以压缩、归档或丢弃；
- 不得因为出现在 context 中就自动成为 project/global memory；
- 可被 checkpoint 引用，但不能替代 checkpoint。

### 2. Project Memory

项目内部已确认的仓库结构、约定、验证命令、架构决策、失败模式和项目级 tool recipe。

- 默认只对一个 `project_id` 可见；
- 任何记录必须带 `project_id`、来源和适用范围；
- 项目 workspace 的读写权限不能由 memory retrieval 自动授予；
- 项目删除、迁移或脱敏时，关联 memory 必须按 retention policy 处理。

### 3. Global Harness Memory

跨项目可复用的通用工具 recipe、模型能力画像、共享 harness 经验、通用安全规则和经过验证的 procedural knowledge。

- 不能由单个项目的一次成功自动产生；
- 必须经过去重、证据审查、回归验证、版本化、审批和回滚登记；
- global memory 只能提供知识，不自动提供任何项目 workspace 或 secret 访问权；
- policy、skill、routing 等高风险对象必须另行经过对应的 promotion gate。

### 4. Cross-Project Relation Memory

只记录经授权的项目间关系，例如共享组件、依赖、接口契约、共同 incident、兼容性约束、迁移窗口和 evolution proposal。

- 关系记录必须明确 `source_project_id`、`target_project_id`、关系类型和授权范围；
- 默认只暴露关系摘要，不暴露另一个项目的原始文件、日志或凭据；
- 跨项目关联不等于跨项目读写授权；
- 每个项目保留独立 `PolicyGate`、workspace ownership 和 approval boundary。

## 二、核心对象

### MemoryRecord

```text
MemoryRecord {
  memory_id
  schema_version
  object_version
  memory_type              # episodic | semantic | procedural | relation | code_fact
  scope                    # run | project | global | cross_project
  portfolio_id?
  project_id?
  source_project_id?
  target_project_id?
  goal_id?
  task_id?
  task_run_id?
  title
  content_ref               # 结构化内容或受控 artifact 引用，不默认保存敏感原文
  source_event_ids[]
  source_artifact_refs[]
  provenance
  confidence
  applicability
  visibility
  lifecycle_state
  supersedes_memory_id?
  expires_at?
  created_at
  updated_at
}
```

`content_ref` 不允许成为绕过 EventLog、PolicyGate 或 secret redaction 的自由文本通道。高敏感正文只能保存为经授权的 artifact 引用。

### MemoryCandidate

运行时从事件、验证结果、失败或用户确认中提取的候选，不等同于已接受 memory。

```text
MemoryCandidate {
  candidate_id
  source_event_ids[]
  source_memory_ids[]
  proposed_scope
  proposed_memory_type
  claim_summary
  evidence_refs[]
  negative_evidence_refs[]
  confidence
  dedup_key
  conflict_set[]
  proposed_retention
  validation_plan
  candidate_state       # new | reviewing | rejected | accepted | superseded
  proposer
  created_at
}
```

### MemoryPromotion

把 candidate 提升为某个 scope 的 `MemoryRecord` 的审计记录。

```text
MemoryPromotion {
  promotion_id
  candidate_id
  from_scope
  to_scope
  decision               # accept | reject | defer | supersede
  decision_basis
  validation_refs[]
  approver_ref?
  policy_decision_ref
  rollback_ref?
  target_memory_id?
  created_at
}
```

### CrossProjectLink

```text
CrossProjectLink {
  link_id
  source_project_id
  target_project_id
  relation_type           # dependency | shared_component | shared_incident | compatibility | evolution_proposal
  summary_ref
  evidence_refs[]
  visibility
  policy_ref
  lifecycle_state
  created_at
  updated_at
}
```

## 三、生命周期

```text
observed
  -> candidate
  -> reviewing
  -> accepted
  -> active
  -> stale | superseded | expired | revoked | archived
```

- `observed`：事件或验证中出现的事实，仍在 EventLog；
- `candidate`：抽取出的可复用主张，未获得记忆语义；
- `reviewing`：正在去重、冲突分析或验证；
- `accepted`：通过 scope 对应的 promotion gate；
- `active`：允许检索，但仍受 policy 和 applicability 过滤；
- `stale`：证据过期或环境变化，禁止高风险自动使用；
- `superseded`：被新版本替代，保留链路；
- `expired`：超过 retention，按删除/归档规则处理；
- `revoked`：因错误、泄露或策略变化主动撤销；
- `archived`：移入冷归档，仍可通过审计路径恢复，不进入默认 context。

任何状态变更都产生事件；不能直接覆盖历史记录。删除敏感内容时，保留最小化的删除/撤销审计事件，不保留被删除 secret 原文。

## 四、检索与可见性

检索顺序必须先执行权限和范围过滤，再进行相似度召回：

```text
query intent
  -> PolicyGate / visibility filter
  -> project / portfolio / goal / task scope filter
  -> lifecycle and time filter
  -> deterministic metadata query
  -> optional keyword/vector/graph retrieval
  -> provenance and confidence check
  -> conflict resolution
  -> context budget selection
```

- 向量索引、关键词索引和图索引都是派生索引，不是事实源；
- 高风险任务必须先满足确定性 metadata 查询和 evidence 要求，不能只凭向量相似度；
- 召回结果必须携带 `memory_id`、来源、scope、confidence、版本和状态；
- `scope=global` 不意味着对所有项目自动可见，仍需 visibility 与 policy filter；
- 冲突记录必须并列返回或阻断自动使用，不能静默选择一个。

## 五、RAG 与 Wiki 派生视图

RAG 负责检索证据，Wiki 负责把多个 `EvidencePack` 编译成可导航、可版本化的知识视图。Wiki 不是事实源，也不能绕过 `EventLog`、`PolicyGate`、source artifact 或 secret redaction。

- Wiki 页面必须带 `source_refs`、`evidence_pack_ids`、confidence、适用范围、版本和 freshness/expiry；
- Wiki 命中后仍要检查来源状态，过期或低置信度页面只能作为导航；
- 新证据生成新 `WikiRevision` 或 `WikiPageCandidate`，不能静默覆盖旧页面；
- project Wiki 默认 project-scoped；global/cross-project Wiki 必须经过对应 promotion gate；
- 低风险项目页可以自动发布，高风险页面、policy/skill/routing 依据和跨项目关系只生成 candidate 或要求审批；
- Wiki 是 `MemoryRecord` 的可读投影，不等于 memory、event 或 source corpus。

## 六、Promotion Gate

### Run -> Project

需要：项目范围证据、可复现验证、适用条件、来源 event/artifact、无 secret、冲突处理和项目 policy 许可。

### Project -> Global

需要：至少一个明确复现证据，必要时跨任务/跨 fixture 回归，负面证据记录，兼容性检查，global policy 审批和可回滚版本。

### Project -> Cross-Project Relation

需要：两个项目的显式授权、关系证据、最小化摘要和双方 policy gate；不得通过 relation link 传播原始内容访问权。

### Global -> Runtime Policy / Skill / Routing

需要：独立 proposal、验证数据集、风险评估、人工审批、版本化、灰度范围和 rollback；candidate 或 accepted memory 不能直接改写这些对象。

## 七、Phase 1 规定

Phase 1 只实现：

- durable EventLog、StateStore、TraceStore 和 checkpoint metadata；
- `WorkingMemory` 的本地持久化与 run 级检索；
- 项目范围的 `MemoryCandidate`；
- `MemoryRecord` 的最小 metadata 和 provenance；
- 按 `project_id`、`goal_id`、`task_id` 的确定性检索；
- `ToolUseErrorEvent -> LessonCandidate -> verified local recipe` 的候选链；
- memory 写入、检索和 promotion 的 PolicyGate 事件。
- 低风险项目范围的 `WikiPageCandidate`、`EvidencePack` 和 source citation。

Phase 1 不实现：

- 向量数据库、图数据库或自动 embedding pipeline；
- global memory 自动 promotion；
- 跨项目隐式共享或跨项目 workspace 访问；
- 自动修改全局 policy、skill、prompt 或 model routing；
- 完整后台 dream learning、在线自改 harness 或无审批长期自进化。
- Wiki 替代 EventLog/source corpus、无证据自动入 Wiki、global/cross-project 自动发布。

## 八、必需事件

```text
memory_candidate_created
memory_candidate_reviewed
memory_retrieval_requested
memory_retrieval_filtered
memory_retrieval_used
memory_promoted
memory_rejected
memory_superseded
memory_expired
memory_revoked
cross_project_link_created
cross_project_link_revoked
wiki_page_candidate_created
wiki_revision_generated
wiki_revision_published
wiki_revision_rejected
wiki_page_revalidated
wiki_page_staled
wiki_page_revoked
```

事件 payload 必须使用摘要、引用和 hash；不得写入 API key、token、cookie、credential response 或未经授权的敏感正文。

## 九、冲突、删除与审计

- 同一 `dedup_key` 的候选必须进入冲突集合或合并流程；
- 新证据不能静默覆盖旧证据，必须使用 `supersedes_memory_id` 和事件链；
- 项目隔离、用户删除、数据分类变化或 provider policy 变化都可以触发 revoke/expire；
- memory retrieval 失败不能破坏 EventLog，也不能伪造“没有相关记忆”；
- memory 质量指标至少包括 provenance completeness、retrieval precision、staleness、conflict rate、promotion rejection rate 和 rollback count。

## 十、实现顺序

1. 先实现事件、状态和 checkpoint，再实现候选和确定性检索；
2. 再实现项目级 consolidation 和人工确认；
3. 再增加语义索引、时间关系和代码图，并保持派生索引可重建；
4. 最后实现 procedural memory、skill evolution 和 global/cross-project promotion。

任何阶段都不能把“设计存在”写成“runtime 已具备”。
