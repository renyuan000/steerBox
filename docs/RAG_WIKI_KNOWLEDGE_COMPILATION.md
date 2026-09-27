# RAG + Wiki Knowledge Compilation

## 文档定位

本文件定义 `steerBox` 的 RAG 与持久 Wiki 组合方式。这里的 Wiki 是由检索证据编译出的、可版本化和可失效的知识视图，类似用户描述的“RAG 结果长期沉淀为 Wiki”，但不要求人工逐条整理、交叉编排和维护。

它不是：

- 把模型回答直接保存成事实；
- 把 RAG 的 top-k 片段直接拼成页面；
- 用 Wiki 替代 EventLog、原始文档、代码、验证结果或权限系统；
- 让自动生成页面直接变成全局 policy、skill 或安全规则。

核心原则：

```text
source -> retrieve -> evidence pack -> claim extraction -> wiki candidate
       -> conflict/freshness checks -> publishable wiki revision
       -> wiki-first retrieval -> source revalidation
```

## 一、为什么要同时有 RAG 和 Wiki

RAG 擅长回答“当前问题附近有什么相关来源”，但每次都从原始资料开始会重复检索、重复总结，难以形成稳定的项目知识结构。Wiki 擅长提供稳定的主题入口、目录、术语、决策和跨任务导航，但单独使用会产生过期、断章取义和无来源总结。

两者的分工是：

| 层 | 负责 | 不能负责 |
|---|---|---|
| Source corpus | 原始代码、文档、EventLog、验证产物、授权外部资料 | 不负责自动回答 |
| RAG retrieval | 按问题、范围、时间和权限找证据 | 不负责证明事实已经正确 |
| EvidencePack | 保存本次检索的来源、片段、时间、hash、可信度和过滤结果 | 不替代原始来源 |
| Wiki | 将多个 EvidencePack 编译为结构化、可导航、可复用的知识页面 | 不成为唯一事实源 |
| Memory / EventLog | 保存事实链、状态、候选、版本和审计 | 不承担页面展示体验 |

## 二、读路径：Wiki-first，RAG 回查

```text
user query
  -> scope / PolicyGate
  -> wiki topic lookup
  -> freshness and applicability check
  -> source RAG backfill for missing or stale claims
  -> conflict and provenance check
  -> context budget selection
  -> answer with page and source citations
```

规则：

- Wiki 页面命中后仍要检查 `updated_at`、`expires_at`、适用版本和来源状态；
- 过期或低置信度页面只能作为导航，不能直接支撑高风险动作；
- 回查发现新证据时，先生成新 revision/candidate，不静默覆盖当前页面；
- 回答必须能同时引用 Wiki 页面和底层 EvidencePack/source ref；
- `scope=project` 的 Wiki 不得因为 RAG 相似度而泄露给其他项目。

## 三、写路径：自动编译，分级发布

### 触发条件

以下事件可以进入 `WikiCompilationQueue`：

- 用户明确要求“记住、沉淀、整理成知识”；
- 同一主题在多个 TaskRun 中重复检索或重复解释；
- 一个 Goal 完成并有通过的 VerificationResult；
- 失败模式、工具 recipe 或架构决策经过复现；
- source corpus 发生变更，导致既有页面可能过期；
- 项目初始化或新模块扫描完成。

### 编译流程

```text
RAG retrieval
  -> EvidencePack
  -> claim extraction
  -> dedup / merge candidate
  -> contradiction and freshness check
  -> structure generation
  -> link and citation lint
  -> risk-based publish decision
  -> WikiRevision + index update
```

自动编译必须保留：

- 来源文档、代码、事件和 artifact 引用；
- 每个重要 claim 的 evidence ref；
- 生成模型、prompt/profile 版本和时间；
- confidence、适用范围、有效期和 source freshness；
- 被合并、被拒绝和被 supersede 的候选；
- 页面与 MemoryRecord、Goal、Project、Task 的关联。

## 四、Wiki 对象

### WikiPage

```text
WikiPage {
  wiki_page_id
  schema_version
  object_version
  scope                    # project | global | cross_project
  project_id?
  topic_key
  title
  current_revision_id
  lifecycle_state          # draft | published | stale | superseded | revoked | archived
  source_count
  citation_coverage
  confidence
  applicability
  updated_at
  expires_at?
}
```

### WikiRevision

```text
WikiRevision {
  revision_id
  wiki_page_id
  parent_revision_id?
  content_ref
  structured_claims[]
  evidence_pack_ids[]
  source_refs[]
  rejected_claims[]
  conflict_refs[]
  generated_by
  generator_prompt_version
  verification_status
  publish_decision
  created_at
}
```

### WikiPageCandidate

`WikiPageCandidate` 是尚未发布的页面提案。它可以由一次 RAG 检索、多个 EvidencePack 合并或既有页面 revalidation 产生；它不能被回答路径当作已确认知识直接使用。

```text
WikiPageCandidate {
  candidate_id
  proposed_scope
  project_id?
  topic_key
  proposed_title
  proposed_claims[]
  evidence_pack_ids[]
  source_refs[]
  conflict_refs[]
  freshness_status
  risk_class
  validation_plan
  candidate_state       # new | reviewing | published | rejected | superseded
  created_at
}
```

### EvidencePack

```text
EvidencePack {
  evidence_pack_id
  query_ref
  scope
  retrieval_policy_ref
  source_refs[]
  excerpt_hashes[]
  source_versions[]
  retrieved_at
  source_trust
  redaction_status
  completeness_status
}
```

Wiki 页面必须引用 EvidencePack；EvidencePack 必须能够回到 EventLog、source artifact 或受授权的外部来源。页面内容不能成为逃避 secret redaction 的自由文本出口。只有 `published` 的 `WikiRevision` 才能进入默认 Wiki 检索；candidate 只能进入审核、验证或显式预览路径。

## 五、减少人工干预的机制

人工不应参与每一页的格式化、去重和普通更新。系统自动完成：

- 主题聚类和 page routing；
- 重复页面合并；
- 标题、目录、术语和双向链接生成；
- 引用覆盖检查、链接检查和 markdown/schema lint；
- 过期检测、source change 触发的 revalidation；
- 低风险页面的自动 publish；
- 页面使用率、冲突率和错误反馈驱动的更新队列。

人工只在以下情况介入：

- global 或 cross-project promotion；
- 高风险安全、权限、合规、生产变更或不可逆动作依据；
- 证据冲突无法自动消解；
- 页面拟改变 policy、skill、model routing 或 tool capability；
- 用户明确要求确认或撤销。

因此，人工角色从“编辑每个页面”变成“处理高风险 promotion 和不可自动判定的冲突”。

## 六、Wiki 与四层 Memory 的关系

- Run Wiki：当前任务的临时导航，可在任务结束后压缩或删除；
- Project Wiki：项目内稳定知识入口，默认 project-scoped；
- Global Wiki：通过 Global Memory Promotion Gate 后形成的通用知识；
- Cross-Project Wiki：只保存经授权的关系摘要，不传播另一个项目的原始内容或访问权。

Wiki 是 MemoryRecord 的一种可读投影，不等于 MemoryRecord 本身。页面撤销、过期、合并和恢复都必须保留事件链。

## 七、风险和质量门槛

必须阻断或降级的情况：

- 没有 source ref 的 claim；
- 只有单个低可信来源且没有交叉验证的高风险结论；
- source 版本已失效或页面超过 retention；
- 页面存在未解释的相互矛盾 claim；
- 外部资料包含 prompt injection、工具调用指令或未经授权的秘密；
- 生成结果试图改变 policy、tool capability、routing 或 workspace 权限。

至少观测以下指标：

- citation coverage；
- source freshness / stale hit rate；
- page reuse rate；
- contradiction rate；
- retrieval-to-page dedup rate；
- page correction / revoke rate；
- 人工介入比例；
- Wiki 命中后 source revalidation 的成本和延迟。

## 八、Phase 1 进入方式

Phase 1 可以做轻量版本：

- 使用本地 SQLite metadata + Markdown/JSON 页面，不要求向量数据库；
- 实现 `EvidencePack`、`WikiPage`、`WikiRevision` 的最小 schema；
- 在项目范围内将通过验证的 RAG 结果生成 `WikiPageCandidate`；
- 页面保存 source refs、claim summary、confidence、适用 project/goal/task 和 expiry；
- 支持 wiki-first 查询、确定性 metadata filter 和 source citation；
- 支持低风险项目页自动发布，global/cross-project 页只生成 candidate；
- 把编译、发布、撤销和过期写入 EventLog。

Phase 1 不做：

- 让 Wiki 替代 EventLog 或原始 source corpus；
- 无证据的模型知识自动入 Wiki；
- 自动把 Wiki 内容提升为全局 policy、skill、routing 或安全规则；
- 复杂多租户图 Wiki、全自动语义聚类和无限后台重写。

## 九、后续演进

1. Phase 1：本地页面、证据回链、项目级自动编译和确定性检索；
2. Phase 2：异步 consolidation、增量 revalidation、版本 diff 和人工确认队列；
3. Phase 3：向量/关键词混合检索、时间关系和代码逻辑链接；
4. Phase 4：global/cross-project promotion、procedural Wiki 和受治理 skill evolution。
