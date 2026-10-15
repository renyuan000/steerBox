# steerBox RSI 采用清单与性能设计

**日期**：2026-10-15  
**状态**：设计已获方向性批准；本文件是实现前的采用决策与性能门禁，不代表 RSI runtime 已实现。  
**范围**：Recursive Self-Improvement、Real-time Operational Self-Evolution、harness/skill/memory/recipe 的受治理演进。

## 1. 结论先行

steerBox 不采用“模型在主循环中随时修改自己”的 RSI。采用三条分离的路径：

```text
热路径：Agent 正常工作
  事实记录、预算检查、PolicyGate、验证、用户控制
  额外 RSI LLM 调用 = 0

暖路径：当前 run / project 的轻量适应
  规则分类、计数、候选引用、已验证 recipe 查询
  不修改安全边界，不阻塞主循环

慢路径：Evolution Plane
  经验提炼、弱点挖掘、候选生成、隔离实验、回归评估、晋升或回滚
  独立队列、独立预算、独立工作区
```

核心决策：

1. 进化对象优先是 tool recipe、middleware、memory/context 组织和 model routing；不把 system prompt 文案当作唯一或首要优化对象。
2. 每个候选改动必须有触发证据、预期收益、验证结果、适用范围、负面证据和回滚引用。
3. 失败经验可以实时成为 candidate，但不能实时成为全局 policy。
4. 主 Agent 的正常任务预算不能被 RSI 实验消耗；RSI 失败不能阻塞、重试或改变正在运行的任务。
5. 安全边界、凭据、DLP、审批、EventLog 语义和 evaluator 本身不允许无人审批自动修改。

## 2. 研究证据分层

### 2.1 可以直接吸收的思想

| 来源 | 一手结论 | steerBox 采用方式 | 性能/可靠性限制 |
|---|---|---|---|
| Reflexion, arXiv:2303.11366 | 用语言反馈和 episodic memory 改进后续 trial，不更新模型权重 | 失败事件生成带证据的 `ReflectionNote`/`LessonCandidate` | 反思不是事实；必须绑定外部验证，不得直接变成 policy |
| Voyager, arXiv:2305.16291 | 执行反馈、错误、自验证驱动可组合 skill library | 版本化 `ToolUsageRecipe`/`ExecutableSkill`，记录成功和被拒方案 | skill 执行前仍需 capability、scope 和 PolicyGate |
| Self-Refine, arXiv:2303.17651 | 生成→反馈→修订的迭代可以提升输出 | 仅在高价值、可验证、低副作用的任务或离线候选上选择性启用 | 每轮反馈都增加模型调用和延迟；不作为每个工具调用的默认行为 |
| CRITIC, arXiv:2305.11738 | 外部工具反馈比模型内在自评更适合纠错 | 优先使用编译器、测试、link checker、schema validator 等确定性 verifier | “模型自己说自己正确”不算验证；工具输出也要脱敏和限长 |
| Self-Evolution Survey, arXiv:2404.14387 | 将自进化拆成 experience acquisition、refinement、updating、evaluation | 作为 Evolution Plane 四阶段生命周期 | 四阶段不能压缩成一次 prompt 调用；每阶段要有证据边界 |
| Long Term Memory, arXiv:2410.15665 | 长期记忆是 inference-time self-evolution 的基础 | 原始事件、摘要、pattern、candidate、accepted recipe 分层保存 | 记忆不是事实源或权限源；检索结果不能隐式扩大能力 |
| Agentic Harness Engineering, arXiv:2604.25850 | component/experience/decision observability 使 harness 改动可回滚、可验证 | `ComponentVersion`、`ExperienceDigest`、`EvolutionProposal`、`DecisionRecord` | 论文摘要的 benchmark 数字不作为 steerBox 性能承诺；必须本地回归 |
| Self-Harness, arXiv:2606.09498 | Weakness Mining→Harness Proposal→Proposal Validation | 从模型/任务特定失败生成最小 proposal，再做 regression gate | 不能跳过回归；模型弱点只能影响候选，不得直接改全局策略 |
| SkillHone, arXiv:2606.08671 | persistent decision history 比只保存最终 skill 更可复盘 | 保存诊断、修订、评估、拒绝方案和结果 | 历史需要压缩和索引；不能把全部 trajectory 放进热 context |
| Meta-Engineering Harnesses, arXiv:2605.25665 | contract、角色分离、独立/对抗验证和外层校准适合持续软件生产 | 保持 generator/evaluator/promotion 分离，失败分类进入外层校准 | evaluator 不能与 generator 完全同源；高风险需人工升级 |
| Darwin Gödel Machine, arXiv:2505.22954 | 通过 sandbox、候选 archive 和 benchmark 验证探索自改代码 | 仅吸收隔离 archive、分支探索、可验证晋升 | 不把自改生产代码、开放式搜索和主 Agent 运行态混在一起 |
| GEPA, arXiv:2507.19457 | 反思驱动的 prompt 演化可用较少 rollout 探索候选 | 作为离线 prompt/context optimizer 的候选算法 | 只允许在固定评测集和候选分支中运行，不能同步影响生产任务 |
| ACE, arXiv:2510.04618 | 将 context 作为可累积、可组织的 playbook，避免摘要反复重写导致信息坍缩 | 增量式 context/recipe 更新，保留 provenance 和历史版本 | 不做无界增长；按任务和预算检索最小必要片段 |

### 2.2 必须吸收的反例

`Large Language Models Cannot Self-Correct Reasoning Yet`（arXiv:2310.01798）指出，缺少外部反馈时，模型内在自纠错可能无效甚至退化。对 steerBox 的直接要求是：

- reflection 只能产生候选或诊断；
- verifier 必须尽量独立于 generator；
- 文件、代码、配置和安全任务优先使用确定性检查；
- 高风险候选必须有对抗性或第二模型审查；
- 不能把“又调用一次模型”误认为质量提升。

所有论文都是公开预印本或摘要级证据。论文中的提升数字只说明研究方向有价值，不等于 steerBox 已达到相同效果。

## 3. steerBox 的 Evolution Plane

### 3.1 生命周期

```text
ExperienceEvent
  → FailureCategory / OutcomeMetric
  → LessonCandidate / ReflectionNote
  → EvolutionProposal
  → isolated ExperimentRun
  → EvaluationResult
  → PromotionDecision
  → accepted version 或 rejected/rollback
```

候选晋升只能沿作用域逐级进行：

```text
run → project → portfolio → global
```

默认只允许 `run` 和 `project` 范围。跨项目和 global 必须满足更高验证强度与明确审批。

### 3.2 进化对象

优先级从高到低：

1. 低风险工具用法 recipe；
2. verifier、middleware 和 context 组织；
3. project-local memory retrieval；
4. skill 的步骤、前置检查和后置验证；
5. model routing 的非安全参数；
6. prompt/task profile 的候选版本；
7. MCP manifest 的描述、健康和成本元数据。

默认禁止自动修改：

- `PolicyGate`、ApprovalStore、credential resolver、DLP/redaction；
- allowed path、网络权限、沙箱边界和 side-effect 分类；
- EventLog、StateStore、checkpoint 和恢复语义；
- evaluator、回归集和 promotion gate 本身；
- 全局安全策略和生产主分支代码。

### 3.3 必备对象

后续实现至少需要这些可版本化对象：

```text
ComponentVersion
ExperienceDigest
LessonCandidate
EvolutionProposal
ExperimentRun
EvaluationResult
PromotionDecision
RollbackReference
```

`EvolutionProposal` 最少应包含：

- `proposal_id`、`component_id`、`base_version`、`candidate_version`；
- `scope`、`trigger_event_ids`、`evidence_ids`；
- 预期收益和可测指标；
- 风险、兼容条件、适用/不适用条件；
- evaluator 和 regression dataset 版本；
- token/time/tool-call budget；
- canary 范围、promotion 条件和 rollback 引用；
- rejected alternatives、negative evidence 和人工审批级别。

## 4. 性能设计：快路径优先

### 4.1 热路径硬门禁

这些规则属于性能和可靠性合同：

- 每个正常 loop step 的 RSI 额外 LLM 调用默认必须为 `0`；
- 不同步执行 embedding、vector search、graph traversal、reflection 或 evaluator；
- EventLog 只追加 bounded metadata，不写完整 transcript、原始 tool output 或 credential；
- 高频成功事件允许采样或聚合，但失败、拒绝、验证失败、人工纠偏和安全事件默认全量保留；
- RSI 队列满时丢弃可重算的 digest，不阻塞主 Agent；不可丢的事实仍写入 EventLog；
- 主任务和 Evolution Plane 使用独立 token、CPU、磁盘和并发预算；
- 任何 evolution worker 崩溃、超时或解析失败都只产生 `evolution_worker_failed`，不改变当前 run 状态。

### 4.2 暖路径规则

暖路径只做本地、确定性、可界定的工作：

- 失败类别映射；
- recipe ID 查询；
- 本地计数器、去重 key、cooldown；
- 当前 project 范围候选引用；
- 是否触发异步 proposal 的阈值判断。

暖路径不能调用远程 LLM，也不能等待后台 worker。

### 4.3 慢路径调度

慢路径按优先级和收益触发，而不是每次循环触发：

| 触发 | 默认动作 | 是否可用模型 |
|---|---|---|
| 单次低风险成功 | 只记计数 | 否 |
| 同一工具同一错误重复出现 | 生成 candidate | 可选 cheap/local |
| 验证失败或人工纠偏 | 生成 evidence digest | 可选 cheap/local |
| 项目级失败模式达到阈值 | EvolutionProposal | cheap/local，必要时 strong |
| 提案进入实验 | regression/evaluator | 与主任务隔离 |
| 影响 global/security/approval | 停在审批 | 不允许自动晋升 |

推荐的第一版阈值是配置项而非硬编码。例如同一 `tool_id + failure_signature + scope` 在冷却窗口内重复两次才生成 proposal；实际阈值必须通过本地 benchmark 调整，不在此文档中虚构为已验证性能。

### 4.4 性能指标和门槛

以下是设计目标，不是当前实测结果：

- 热路径 RSI 额外模型调用：`0`；
- 热路径额外同步 I/O：一次 bounded event append，或进入已有 append queue；
- 暖路径不等待网络；
- evolution worker 不得占用主 Agent 的并发槽；
- `wall_time_to_verified_result`、`tokens_per_verified_change`、`tool_calls_per_solved_task`、`human_intervention_count`、`verification_pass_rate` 必须可按版本比较；
- 任何候选只有在质量不下降且延迟/成本回归通过时才允许晋升；
- 评估报告必须同时给出质量、延迟、token、工具调用、失败率和人工介入变化。

## 5. 采用、暂缓与禁止清单

### 立即采用

- `ToolUseErrorEvent → LessonCandidate → verified local recipe`；
- failure category、negative evidence 和 source event 引用；
- component/experience/decision observability；
- generator/evaluator/promotion 分离；
- deterministic verifier 优先；
- project-local、版本化、可回滚的 recipe/memory candidate；
- 结构化决策历史；
- 低风险成功采样、失败/安全事件全量审计；
- 快路径与慢路径分离。

### Phase 1.5 采用

- 本地异步 consolidation worker；
- `EvolutionProposal`、`ExperimentRun`、`EvaluationResult`；
- sandbox/worktree 候选实验；
- replay/regression dataset；
- cheap/local evaluator 与 strong/evaluator fallback；
- project 范围 promotion 和显式 rollback。

### Phase 2 或更后采用

- 多候选 Pareto/archive 搜索；
- GEPA/ACE 风格 prompt/context optimizer；
- 模型特定 harness 进化；
- canary promotion；
- 跨项目 portfolio 进化；
- 并行子 harness 实验。

### 当前明确暂缓

- 权重更新、持续训练、LoRA/adapter 自动发布；
- 生产代码自修改后直接运行；
- 自动修改 PolicyGate、DLP、credential、approval 或 evaluator；
- 无界自动探索和无预算递归 agent；
- 每步 Reflection/CRITIC/第二模型审查；
- 把完整轨迹塞入每次 prompt；
- 因一次成功就全局晋升 recipe/skill/prompt。

## 6. 与当前 Phase 1 的关系

当前 Phase 1 只实现 RSI 的地基，不实现完整 Evolution Plane：

1. 保持 EventLog、Trace、Checkpoint、ModelCallStore 和 ToolRegistry 的事实边界；
2. 完成 bounded foreground provider runner，产生可审计的 model-call、decision 和 failure metadata；Markdown 写入、action 执行和 verification 闭环仍待后续接线；
3. 将工具失败和验证失败映射为候选，而不是自动改行为；
4. 补最小 `LessonCandidate`/recipe candidate 的来源回链和 project scope；
5. 记录性能指标，不在主循环中运行 consolidation 或 evaluator。

这意味着 Phase 1 仍不能称为完整 RSI，也不能称为完整模型连接型自主 Agent。当前已有 OpenAI-compatible library runner、Anthropic library-only injected transport seam、CredentialResolver 注入边界和 ModelCallStore 审计接线；standalone CLI 的生产 credential resolver、真实账号/live provider 验收、受控开发闭环、后台 worker 和恢复仍按现有 Phase 1 文档执行。

## 7. 参考来源

一手论文/文章：

- [Reflexion](https://arxiv.org/abs/2303.11366)
- [Voyager](https://arxiv.org/abs/2305.16291)
- [Self-Refine](https://arxiv.org/abs/2303.17651)
- [CRITIC](https://arxiv.org/abs/2305.11738)
- [A Survey on Self-Evolution of Large Language Models](https://arxiv.org/abs/2404.14387)
- [Long Term Memory: The Foundation of AI Self-Evolution](https://arxiv.org/abs/2410.15665)
- [Darwin Gödel Machine](https://arxiv.org/abs/2505.22954)
- [GEPA](https://arxiv.org/abs/2507.19457)
- [Meta-Engineering Harnesses](https://arxiv.org/abs/2605.25665)
- [Agentic Harness Engineering](https://arxiv.org/abs/2604.25850)
- [Self-Harness](https://arxiv.org/abs/2606.09498)
- [SkillHone](https://arxiv.org/abs/2606.08671)
- [ACE](https://arxiv.org/abs/2510.04618)
- [Large Language Models Cannot Self-Correct Reasoning Yet](https://arxiv.org/abs/2310.01798)

业界/协议实践：

- [OpenAI Harness Engineering](https://openai.com/index/harness-engineering/)
- [OpenAI Unlocking the Codex Harness](https://openai.com/index/unlocking-the-codex-harness/)
- [Anthropic Effective Harnesses for Long-Running Agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents)
- [Anthropic Harness Design for Long-Running Application Development](https://www.anthropic.com/engineering/harness-design-long-running-apps)
- [LangGraph Persistence](https://docs.langchain.com/oss/python/langgraph/persistence)
- [MCP Specification](https://modelcontextprotocol.io/specification/2025-06-18)

来源状态：上述论文的题名和摘要已从 arXiv 一手页面核验；业界页面和协议资料部分沿用项目已有研究记录，若本轮未重新取得正文，则不把其细节当作独立实验证据。论文 benchmark 数字不是 steerBox 承诺。
