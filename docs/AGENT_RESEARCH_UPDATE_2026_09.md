# Agent Research Update (2026-09-22)

## 文档定位

本文件记录截至 2026-09-22 对近期智能体论文、官方工程文章和协议文档的定向核验结果。它用于更新设计判断，不代表 `steerBox` 已经实现这些能力。

来源分为三类：

- `本次已核验`：本轮从公开来源取得页面或摘要，并记录了可追溯 URL；
- `项目既有引用`：仓库已有引用，但本轮没有重新取得可核验正文；
- `待复核`：来源可定位，但当前访问失败、信息不足或需要进一步实验。

## 结论摘要

近期有价值的进展不是简单增加一个更强的模型，而是把 agent 视为由模型、harness、状态、记忆、评估和治理共同构成的系统：

1. 长运行 agent 需要 durable state、结构化交接和可恢复阶段；context window 不是事实源。
2. 记忆正在从“对话向量库”转向事件、结构化状态、时间关系图、语义索引和 procedural memory 的混合系统。
3. generator 与 evaluator 分离、任务级成功标准和回归验证，是长任务质量的关键控制点。
4. harness 自进化开始采用弱点挖掘、候选修改、预测、回归验证和可回滚 promotion 的闭环；不能把“模型自行改 prompt”当作安全的自进化。
5. 工具协议标准化并不等于信任建立。工具、资源和 prompt 仍必须经过 capability、PolicyGate、数据范围、用户同意和审计。
6. 对 `steerBox` 最重要的基础对象仍是 `GoalContract`、`LoopController`、`EventLog`、`StateStore`、`CheckpointMetadata`、`VerificationResult` 和受治理的 `MemoryCandidate`。

## 来源与可吸收结论

| 来源 | 状态 | 本次核验到的事实 | 对 `steerBox` 的设计影响 | 阶段 |
|---|---|---|---|---|
| Anthropic, [Effective harnesses for long-running agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents), 2025-11-26 | 本次已核验 | 长任务跨多个 context window 时，使用 initializer、增量 coding session、进度工件和 git history 维持连续进展；文章明确把 structured progress 作为后续 session 的输入。 | `HandoffArtifact`、checkpoint、进度状态和可重放事件是 runtime primitive。 | Phase 1：最小状态与 checkpoint；后续：多 session handoff |
| Anthropic, [Harness design for long-running application development](https://www.anthropic.com/engineering/harness-design-long-running-apps), 2026-03-24 | 本次已核验 | 文章采用 planner、generator、evaluator 分离，并强调把工作拆成可处理的块、用结构化 artifact 交接、将主观质量标准转成可评分标准。 | `GoalContract.success_criteria`、`VerificationResult` 和独立 evaluator 不能被模型自评替代。 | Phase 1：契约与验证接口；后续：多角色协作 |
| LangChain, [LangGraph Persistence](https://docs.langchain.com/oss/python/langgraph/persistence) | 本次已核验 | 官方文档区分 checkpointer（线程/执行级短期状态）与 store（跨线程长期数据），并将 persistence 用于恢复、记忆和 time travel。 | checkpoint、current state、long-term memory 必须是不同边界；不能共用一个无语义的 memory 表。 | Phase 1：本地 checkpoint/state；后续：project/global store |
| Letta, [Stateful agents](https://docs.letta.com/v1-sdk/concepts/stateful-agents/) | 本次已核验 | 官方文档把 stateful agent 定义为跨对话维护 memory/context 的 agent，并强调状态需要持久化而不是只依赖当前 context。 | context window 只做热工作集；被 compact/evict 的信息要通过受控存储重新取得。 | Phase 1：working memory + durable state |
| MCP, [Specification 2025-06-18](https://modelcontextprotocol.io/specification/2025-06-18) | 本次已核验 | 规范定义 tools/resources/prompts 和有状态连接，并明确用户同意、数据隐私、工具安全和进度跟踪等安全关注点。 | MCP 是能力接入层，不是信任边界；每次调用仍需要 capability、PolicyGate、审计和 side-effect 记录。 | Phase 1：manifest/事件边界；后续：MCP runtime |
| Zep, [A Temporal Knowledge Graph Architecture for Agent Memory](https://arxiv.org/abs/2501.13956), 2025-01-20 | 本次已核验 | 摘要提出 Graphiti 时间知识图谱，动态整合对话和结构化业务数据，并维护历史关系；报告了跨 session 与时间推理收益。 | 为 cross-project relation memory 预留时间关系、版本和来源；不把图索引当事实源或权限源。 | 后续阶段：关系记忆 |
| Mem0, [Building Production-Ready AI Agents with Scalable Long-Term Memory](https://arxiv.org/abs/2504.19413), 2025-04-28 | 本次已核验 | 摘要提出从对话动态提取、整合和检索显著信息，并报告相对全上下文方法的成本/延迟收益。 | memory consolidation 要有 candidate、dedup、provenance、confidence、retention 和 promotion，而不是直接保存完整 transcript。 | Phase 1：candidate；后续：consolidation |
| Agentic Harness Engineering, [arXiv:2604.25850](https://arxiv.org/abs/2604.25850), 2026-04-28 | 本次已核验 | 摘要提出 component/experience/decision observability，把 harness 修改变成可回滚、带预测、可用后续任务结果验证的闭环；其摘要报告 tools、middleware 和 long-term memory 的收益。 | `EvolutionProposal` 必须带触发证据、预期、回归结果、版本和回滚点；进化对象不能只有 prompt。 | Phase 1：lesson candidate；后续：受治理 promotion |
| Self-Harness, [arXiv:2606.09498](https://arxiv.org/abs/2606.09498), 2026-06-08 | 本次已核验 | 摘要把自改进拆成 Weakness Mining、Harness Proposal、Proposal Validation，并要求候选修改经过 regression testing。 | 将 runtime failure 转成候选，不直接改变 global policy；模型特定弱点进入 `ModelProfile`/`EvolutionProposal`。 | Phase 1：记录候选；后续：验证实验 |
| Continual Harness, [arXiv:2605.09998](https://arxiv.org/abs/2605.09998), 2026-05-11 | 本次已核验 | 摘要研究在单次长运行中在线调整 prompt、sub-agents、skills 和 memory，并强调 reset-free adaptation。 | 支持长年运行不等于无限自动变更；在线适应必须有 checkpoint、预算、停止条件、PolicyGate 和回滚。 | 后续阶段：在线适应 |
| SkillHone, [arXiv:2606.08671](https://arxiv.org/abs/2606.08671), 2026-06-07 | 本次已核验 | 摘要强调 persistent decision history，保存诊断、修订、证据、结果和被拒绝方案，并用角色分离的 subagents 进行 practice probes。 | skill 记录不能只保存最终版本；需要 decision history、negative evidence 和 rejected proposal。 | 后续阶段：procedural memory/skill evolution |
| OpenAI, [Harness Engineering](https://openai.com/index/harness-engineering/) | 待复核 | 本轮公开页面返回 HTTP 403；仓库已有该来源引用，但本轮未取得正文证据。 | 先沿用项目已有“repo knowledge、机械化约束、可观测 harness”方向，待访问恢复后再更新精确引文。 | 设计参考，未作为本轮唯一依据 |

## 对当前开发的直接决定

### Phase 1 必须进入

- `GoalContract`：目标、范围、成功标准、风险、工具/模型边界、验证计划、停止条件；
- `LoopController`：observe/plan/act/record/verify/reflect 的有限状态闭环；
- `EventLog`、`StateStore`、`TraceStore`、`CheckpointMetadata`；
- 独立的 `VerificationResult` 和 evidence 引用；
- `WorkingMemory` 与项目级 `MemoryCandidate`；
- 按 project/goal/task 元数据的确定性检索；
- `EvolutionProposal` 的记录边界，但不自动 promotion；
- 多 provider/model 的 `ModelProfile`、`PromptProfile` 和可审计 `RouteDecision`。

### Phase 1 明确不做

- 向量数据库或图数据库作为事实源；
- 全局记忆自动提升；
- 跨项目隐式可见或隐式读写；
- 自动修改全局 policy、prompt、skill 或 model routing；
- 无限 loop、无预算长运行或无人工升级路径的自我修改；
- 完整多项目分布式调度和多角色协作 runtime。

### 研究限制

arXiv 条目是公开预印本摘要，不等同于已完成同行评审；摘要中的 benchmark 数字在本文件只作为来源报告的结果，不作为 `steerBox` 的性能承诺。所有吸收结论仍需通过本项目自己的 fixture、回归测试和安全评审验证。
