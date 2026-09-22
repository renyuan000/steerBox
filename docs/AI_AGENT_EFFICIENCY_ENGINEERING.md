# AI Agent Efficiency Engineering

## 文档目的

本文件用于定义 `steerBox` 中的 AI 智能体效率工程。

本文件是讨论稿，不是最终实现规格，也不代表当前仓库已经具备对应能力。

## 一句话定义

AI 智能体效率工程的目标是：在最短时间内，以最快速度、最高质量、最佳代码实践、最小 token 花费、最小系统资源占用，达成可验证的运行目标。

它不是单纯的 token 优化，也不是单纯的性能优化，而是面向智能体任务闭环的整体效率工程。

## 核心目标

效率工程同时优化以下目标：

- 最短完成时间
- 最快推进速度
- 最高交付质量
- 最佳代码实践
- 最小 token 花费
- 最小系统资源占用
- 最少无效动作
- 最低跑偏概率
- 最低人工纠偏成本
- 最高可恢复性
- 最高可审计性
- 最高目标达成率

这些目标可能冲突，因此效率工程不是盲目压缩成本，而是在目标、质量、风险和成本之间做可控权衡。

## 不是什么

AI 智能体效率工程不是：

- 只追求少用 token
- 只追求快
- 只追求少调用工具
- 只追求模型单轮回答漂亮
- 只依赖 prompt 技巧
- 牺牲验证换速度
- 牺牲安全边界换自动化
- 牺牲代码质量换一次性完成

如果一个 agent token 用得很少，但经常误改、漏验证、无法恢复、需要大量人工返工，它不是高效率。

## 效率公式草案

可以先用一个概念公式描述：

```text
Agent Efficiency = Verified Goal Achievement / Total Cost
```

其中 `Verified Goal Achievement` 包括：

- 目标完成度
- 验证通过情况
- 质量水平
- 风险控制情况
- 可维护性
- 可恢复性

`Total Cost` 包括：

- wall time
- token cost
- tool call cost
- model selection cost
- compute / memory / IO cost
- human intervention cost
- failed attempt cost
- rollback / repair cost
- audit cost

`推断，高置信`：真正的效率不是“便宜”，而是“以更低总成本稳定达成更高质量的可验证结果”。

## 关键维度

### 1. 时间效率

关注 agent 从接收目标到完成验证所需的真实时间。

指标包括：

- `wall_time_to_verified_result`
- `loop_iterations_to_success`
- `time_to_first_useful_action`
- `time_to_first_verification`
- `blocked_time`
- `retry_time`

设计方向：

- 减少无效探索
- 减少重复读取
- 减少不必要上下文加载
- 尽早建立验证闭环
- 能并行的只在安全边界内并行

### 2. 推进速度

关注 agent 是否持续向目标推进，而不是忙于无意义动作。

指标包括：

- `productive_action_ratio`
- `dead_end_action_count`
- `repeated_action_count`
- `unnecessary_file_read_count`
- `unnecessary_tool_call_count`

设计方向：

- goal contract 明确目标和边界
- loop controller 每轮判断是否推进目标
- evaluator 检查是否偏离
- memory 记录已探索路径，避免重复踩坑

### 3. 质量效率

关注 agent 交付结果是否正确、可维护、可验证。

软件开发方向指标包括：

- `test_pass_rate`
- `lint_pass_rate`
- `review_findings_count`
- `regression_count`
- `bug_reopen_count`
- `unintended_file_change_count`
- `code_practice_violation_count`

安全领域方向指标包括：

- `false_positive_rate`
- `false_negative_rate`
- `incident_evidence_completeness`
- `response_correctness_rate`
- `high_risk_action_approval_rate`
- `rollback_success_rate`

设计方向：

- done 必须事先定义
- 结果必须证据化
- generator 和 evaluator 职责分离
- 高风险动作必须有审批和回滚点
- 代码实践应沉淀为可检查规则，而不是只写在提示词里

### 4. Token 效率

关注 token 是否花在真正有价值的上下文、推理和验证上。

指标包括：

- `tokens_per_solved_task`
- `tokens_per_verified_change`
- `context_reuse_rate`
- `irrelevant_context_ratio`
- `summary_compression_ratio`
- `memory_hit_rate`
- `retrieval_precision`
- `retrieval_recall`
- `model_route_cost_saving`
- `model_route_quality_delta`
- `fallback_model_success_rate`

设计方向：

- AGENTS.md 保持短而稳定
- 深层知识进入结构化 docs
- 上下文按需加载
- 长期记忆分层管理
- trace 与 transcript 不直接塞入上下文
- dream learning 离线压缩经验

### 5. 系统资源效率

关注 agent 长期运行时对 CPU、内存、磁盘 IO、进程数、sidecar 的占用。

指标包括：

- `idle_cpu_usage`
- `idle_memory_usage`
- `active_memory_peak`
- `sqlite_write_latency`
- `event_log_flush_latency`
- `sidecar_process_count`
- `mcp_server_idle_cost`
- `checkpoint_size`
- `restore_time`

设计方向：

- minimal supervisor 保持轻量
- 不默认常驻大量 sidecar
- 高频 trace 与核心 state 分离
- SQLite WAL 用于关键状态和事件
- transcript / trace 可使用 JSONL append
- 工具和 MCP server 按需启动、可回收

### 6. 人工介入效率

关注人类如何用最少成本完成掌舵、审批、纠偏和接管。

指标包括：

- `human_intervention_count`
- `approval_latency`
- `clarification_count`
- `operator_takeover_count`
- `manual_repair_time`
- `decision_trace_readability`

设计方向：

- 关键节点才请求人类
- 请求人类时必须给出证据、风险、选项和建议
- 人类决策必须进入 audit log
- 人类随时可暂停、恢复、接管
- 前台控制面只显示可行动信息，不堆噪声

### 7. 恢复效率

关注 agent 出错、重启、OOM、上下文丢失后能否快速恢复。

指标包括：

- `checkpoint_restore_success_rate`
- `restore_time`
- `lost_work_amount`
- `state_rebuild_time`
- `side_effect_reconciliation_success_rate`

设计方向：

- agent 不是状态拥有者
- 状态属于 harness
- 关键事件 event-driven durable write
- 软 checkpoint 和硬 checkpoint 分级
- side-effect ledger 记录外部影响
- 恢复后必须知道哪些动作已执行、哪些未执行、哪些需要补偿

## 效率工程的核心控制面

建议后续把效率工程落到以下对象上。

### 1. GoalContract

决定做什么、不做什么、怎么验证、成本预算是多少。

关键字段：

- `objective`
- `scope`
- `success_criteria`
- `quality_bar`
- `constraints`
- `risk_level`
- `verification_plan`
- `efficiency_budget`
- `stop_conditions`

### 2. LoopController

决定每轮是否继续、怎么继续、是否偏离、是否需要验证或人类介入。

关键职责：

- observe
- plan
- act
- record
- evaluate
- update state / memory
- checkpoint
- continue / stop

### 3. ContextBudget

决定当前轮次应该加载哪些上下文，不加载哪些上下文。

关键职责：

- 控制上下文大小
- 排除低相关内容
- 保留任务关键事实
- 管理摘要与原始证据关系

### 3.1 Context Packing Efficiency

关注后端 LLM 实际接收到的上下文是否足够精确，而不是只看总 token 数是否低。

建议指标：

- `context_reuse_rate`
- `irrelevant_context_ratio`
- `summary_compression_ratio`
- `retrieval_precision`
- `retrieval_recall`
- `context_budget_utilization`
- `duplicate_context_ratio`
- `low_confidence_escalation_rate`
- `user_confirmation_rate`
- `cleanup_suggestion_acceptance_rate`
- `pruned_context_precision`
- `archive_vs_drop_ratio`
- `cleanup_candidate_review_latency`
- `turn_level_restore_success_rate`
- `restore_to_correct_step_rate`

设计要求：

- 上下文必须先过滤、再压缩、再装箱
- 不相关内容不应因为“可检索”就直接进入 context
- 跨会话记忆应按任务意图和预算选择最小必要子集
- 高风险任务应提高 provenance 和 policy 过滤权重
- 对低置信度但高价值内容，应允许 user confirmation 而不是盲目写入


关注后端 LLM 实际接收到的上下文是否足够精确，而不是只看总 token 数是否低。

建议指标：

- `context_reuse_rate`
- `irrelevant_context_ratio`
- `summary_compression_ratio`
- `retrieval_precision`
- `retrieval_recall`
- `context_budget_utilization`
- `duplicate_context_ratio`

设计要求：

- 上下文必须先过滤、再压缩、再装箱
- 不相关内容不应因为“可检索”就直接进入 context
- 跨会话记忆应按任务意图和预算选择最小必要子集
- 高风险任务应提高 provenance 和 policy 过滤权重

### 4. ModelBudget

决定当前任务应使用哪个模型、是否需要多模型协作、何时降级或升级模型。

关键职责：

- model choice
- cost limit
- latency target
- quality requirement
- availability check
- fallback policy
- routing trace
- historical performance feedback

### 5. ToolBudget

决定哪些工具值得调用、何时调用、调用失败如何降级。

关键职责：

- tool choice
- call limit
- timeout
- retry policy
- fallback policy
- tool result summarization

### 6. ResourceBudget

决定系统资源上限。

关键职责：

- memory limit
- process limit
- sidecar limit
- IO rate limit
- checkpoint size limit
- background task limit

### 7. Evaluator

决定结果是否真的达到目标。

关键职责：

- 验证目标达成
- 检查质量
- 检查安全边界
- 检查误改和回归
- 检查证据完整性

### 8. EfficiencyMetrics

记录实际表现，让后续优化有依据。

关键职责：

- 收集指标
- 归因失败
- 发现低效模式
- 触发 evolution proposal
- 验证优化是否有效

## 与自动进化的关系

效率工程是自动进化的主要输入之一。

如果观测到：

- token 花费高
- 工具调用失败率高
- 重复读取多
- 人类纠偏多
- 误改文件多
- checkpoint 恢复慢
- MCP sidecar 空闲成本高
- 模型选择成本高但质量收益低
- 某类任务长期失败

系统可以生成 `EvolutionProposal`，但不应直接无约束生效。

每个 proposal 至少包含：

- 问题证据
- 目标组件
- 预期收益
- 风险
- 验证方法
- 回滚方案
- 是否需要人工审批

## 第一阶段建议指标

第一阶段不需要一次性实现全部指标，但应先记录最小闭环。

建议最小集合：

- `wall_time_to_verified_result`
- `tokens_per_solved_task`
- `model_route_cost_saving`
- `fallback_model_success_rate`
- `tool_calls_per_solved_task`
- `failed_tool_call_rate`
- `loop_iterations_to_success`
- `human_intervention_count`
- `verification_pass_rate`
- `checkpoint_restore_success_rate`
- `idle_memory_usage`
- `active_memory_peak`

## 当前结论

AI 智能体效率工程应该成为 `steerBox` 的横向核心能力。

它服务于四个专业方向：

- 软件开发智能体：更快、更稳、更高质量地完成真实代码任务
- 持续运维智能体：更低资源、更快发现和恢复服务/资源问题
- 安全评估与漏洞研究智能体：更高验证质量、更低副作用、更可审计地完成授权评估
- 安全运营智能体：更低误报漏报、更可审计地长期分析和响应安全事件

最终目标不是让 agent 看起来自动化，而是让 agent 在真实约束下以最小总成本稳定达成高质量、可验证、可恢复的目标。
