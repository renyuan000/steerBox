# Model, Prompt, Routing, and Checkpoint Policy Discussion Draft

## 文档目的

本文件用于沉淀 `steerBox` 关于多模型接入、模型专用系统提示、模型路由与协作、全自主 loop 检查点策略、防跑偏和防破坏性改写的设计方向。

本文件是讨论稿，不是最终实现规格，也不代表当前仓库已经具备对应能力。文中使用“应支持”“建议”的地方是设计目标；除非第一阶段文档和实际代码、日志、测试另有证据，不得表述为已经实现。

## 当前判断

`steerBox` 应同时支持：

1. Provider、Model、Endpoint 分层接入
2. 多模型路由与成本 / 延迟 / 质量预算
3. 模型专用 prompt profile 和角色专用系统提示
4. 任务专用 prompt pack
5. 模型切换、能力兼容的 fallback 和多模型协作
6. 固定检查点
7. 非固定 / 事件触发检查点
8. 风险触发检查点
9. operator-marked checkpoint
10. side-effect ledger
11. rollback / replay / fork 策略

原因：不同模型的能力、服从度、工具调用稳定性、长上下文可靠性、成本、响应速度和安全边界都不同。如果所有模型共用同一套系统提示和 loop 策略，通常会浪费强模型、压垮弱模型，也更容易跑偏。

## 一、Provider、Model、Endpoint 分层

### 1. 为什么不能只用一个 provider 字段

多模型系统至少要区分三层：

~~~
ProviderProfile
  -> EndpointProfile
      -> ModelProfile
~~~

同一个模型可能通过官方 API、中转服务、本地部署或不同区域 endpoint 提供。它们的上下文窗口、价格、限流、工具能力、隐私边界和可用性可能不同，因此不能把这些差异压缩到一个 provider 字符串中。

### 2. ProviderProfile

ProviderProfile 描述供应方的协议、合规和可用性边界：

~~~
provider_id
provider_name
protocol_type
region
privacy_policy
supported_features
rate_limit
availability_policy
status
auth_ref
~~~

auth_ref 只能引用外部 secret manager、环境注入或操作系统凭据存储中的位置。API key、token、完整 endpoint secret 和认证响应绝不能写入模型注册表、普通配置、trace、event log、checkpoint 或 prompt 文档。

### 3. EndpointProfile

EndpointProfile 描述一次可调度的部署入口：

~~~
endpoint_id
provider_id
model_id
deployment_name
region
context_window
cost_profile
latency_profile
availability_policy
capabilities_override
privacy_class
status
~~~

Endpoint 的覆盖字段必须进入路由和调用记录；不能假定同一 model_id 的所有 endpoint 行为相同。

### 4. ModelProfile 的两类字段

模型画像要区分不会随单次调用改变的静态能力，与基于真实运行证据统计出的动态表现。

静态能力包括：

~~~
context_window
supports_tools
supports_structured_output
supports_streaming
supports_vision
supports_reasoning
supports_background_run
supports_mcp
supports_batch
~~~

动态表现包括：

~~~
historical_success_rate
tool_call_success_rate
verification_pass_rate
coding_quality_score
review_quality_score
security_analysis_quality
average_latency
p95_latency
average_cost
fallback_success_rate
sample_count
measurement_window
source_trace_ids
confidence
last_verified_at
~~~

动态字段必须带样本数、统计窗口、来源 trace、置信度和最后验证时间；一次运行结果只能作为反馈，不能直接升级成永久能力声明。

## 二、多模型提示符多态化

### 1. 是否应该做

应该做。

但它不应只是“给每个模型写一份 system prompt”。更准确的设计应是：

```text
ModelProfile + PromptProfile + TaskPromptPack + PolicyGate + EvaluationFeedback
```

其中：

- `ModelProfile` 描述模型能力和弱点
- `PromptProfile` 描述该模型适合怎样被约束和引导
- `TaskPromptPack` 描述某类任务的目标、规则、检查清单和输出格式
- `PolicyGate` 决定该模型是否允许执行某类动作
- `EvaluationFeedback` 根据实际效果修正 prompt profile 和 model routing

### 2. 为什么不同模型需要不同提示

不同模型差异至少包括：

- 指令遵循能力不同
- 工具调用格式稳定性不同
- 长上下文注意力不同
- 代码修改风格不同
- 对边界条件敏感度不同
- 是否容易过度自信
- 是否容易自作主张扩大修改范围
- 是否擅长规划、执行、审查、总结、检索、修复
- token 成本不同
- latency 不同
- 可访问性和限流不同

因此，弱模型可能需要更强约束、更短上下文、更明确格式、更小任务粒度；强模型可以承担架构推理、复杂定位、风险判断和 evaluator 工作。

### 3. PromptProfile 字段草案

建议定义：

```text
prompt_profile_id
version
status
compatible_provider_types
compatible_model_families
required_capabilities
intended_roles
strengths
weaknesses
instruction_style
context_style
tool_calling_style
output_contract
reasoning_budget
allowed_task_types
forbidden_task_types
risk_limit
checkpoint_policy_override
evaluator_policy_override
known_failure_patterns
required_preflight_checks
fallback_model
last_verified_at
```

PromptProfile 与 ModelProfile 是兼容关系而不是永久硬绑定：一个模型可以有规划、实现、审查等多个 profile，一个 profile 也可以适用于同一模型家族。可以维护 preferred_prompt_profile、compatible_prompt_profiles 和 forbidden_prompt_profiles，但最终仍须由解析器结合任务和 provider 能力决定。

### 4. TaskPromptPack 字段草案

建议定义：

```text
task_prompt_pack_id
task_type
system_prompt_fragments
developer_prompt_fragments
user_prompt_template
required_context_blocks
forbidden_context_blocks
preflight_checklist
execution_checklist
verification_checklist
stop_conditions
handoff_template
failure_response_template
```

### 5. Prompt 组合方式

每次任务不应直接拼一个大 prompt，而应由 harness 组合：

```text
BaseHarnessPrompt
  + ProviderAdapterPrompt
  + ModelPromptProfile
  + AgentRolePrompt
  + TaskPromptPack
  + GoalContract
  + PolicySummary
  + ToolCapabilitySummary
  + CheckpointPolicySummary
  + OutputContract
```

组合过程必须可追踪：

- 使用了哪些 prompt fragment
- 来自哪个版本
- 适用哪个模型
- 适用哪个任务类型
- 为什么被选中
- 本次执行效果如何

### 6. AgentRolePrompt

“同一模型配一套 system prompt”仍然不够。系统应按执行角色提供独立的角色级提示层：

~~~text
planner
implementer
reviewer
tester
evaluator
summarizer
researcher
security_analyzer
coordinator
human_interaction
~~~

强模型也不能因为能力高就跳过角色约束；规划、修改、审查和评估应使用不同的输出契约、工具权限、检查清单和停机条件。

### 7. Prompt Resolution

模型、provider、角色和任务确定后，harness 应按以下顺序解析提示包：

~~~text
1. Provider compatibility
2. Model capability
3. Agent role
4. Task type
5. GoalContract
6. Risk level
7. Tool policy
8. Context budget
9. Human steering mode
10. PromptProfile version
~~~

解析结果必须形成不可变的 ResolvedPromptPack，并包含：

~~~text
prompt_pack_id
base_prompt_version
provider_prompt_version
model_prompt_version
role_prompt_version
task_prompt_version
policy_summary_hash
tool_summary_hash
context_package_hash
resolution_reason
created_at
~~~

每次 ModelCall 都要记录 resolved_prompt_pack_id、各提示版本、model_id、provider_id 和 route_decision_id。否则失败后无法区分模型能力、提示不适配、上下文污染、工具选择或 policy 配置问题。

### 8. 不同模型的典型分工

建议模式：

- 低成本模型：分类、摘要、格式整理、低风险检索、重复检查
- 中等模型：常规代码定位、文档整理、日志初筛、简单修复建议
- 强模型：架构设计、疑难 bug、安全分析、复杂 refactor 策略、关键决策
- 本地模型：隐私敏感数据、离线环境、低成本批处理、初筛
- evaluator 模型：独立审查、反例寻找、风险判断、完成度判断

### 9. PromptProfile 的进化

PromptProfile 不应手工固定不变。

它应基于：

- 成功率
- 跑偏率
- 工具调用失败率
- 格式错误率
- 人工纠偏次数
- token 成本
- loop 收敛轮数
- evaluator 失败原因

生成候选更新。

但 prompt profile 更新也必须经过：

- 小范围验证
- 对比实验
- 回滚点
- 风险审批

## 三、路由、Fallback 与多模型协作

### 1. RoutePolicy 与 RouteDecision

路由不是简单的“选一个最便宜模型”，而是对任务、风险、预算、隐私和能力约束求解。RoutePolicy 至少应考虑：

~~~text
task_type
agent_role
risk_level
quality_requirement
cost_budget
latency_target
privacy_requirement
context_requirement
tool_requirement
availability_requirement
human_steering_mode
~~~

每次路由必须生成可审计的 RouteDecision：

~~~text
route_decision_id
task_id
task_type
agent_role
candidate_models
selected_provider
selected_model
selected_endpoint
selection_reason
rejected_candidates
cost_estimate
latency_estimate
quality_requirement
risk_level
privacy_requirement
context_requirement
tool_requirement
fallback_chain
approval_required
prompt_profile_id
prompt_profile_version
created_at
~~~

selection_reason 必须说明为什么选择当前模型、为什么排除候选模型；路由结果和预算消耗进入 trace。高风险任务不得仅按价格排序，也不得在没有能力兼容性检查时降级。

### 2. Model Switch 与 Fallback

模型切换与 fallback 必须显式产生新的 TaskRun / Attempt，保留原始 GoalContract，重新解析 prompt，并重新执行必要验证：

~~~text
primary failure
  -> classify failure
  -> check capability-compatible fallback
  -> create new TaskRun / Attempt
  -> re-resolve PromptProfile
  -> preserve GoalContract
  -> record new RouteDecision
  -> record provider/model change
  -> run required verification
~~~

禁止静默切换。fallback 规则按任务类型分别治理：

~~~text
fallback_for_read_only
fallback_for_planning
fallback_for_code_edit
fallback_for_review
fallback_for_security_action
~~~

高风险写操作、权限动作和不可逆外部副作用默认不允许因为超时自动切到弱模型；必要时只能暂停、升级到强模型或请求人工审批。

### 3. Model Collaboration

模型协作不是 fallback 的别名。除了主模型失败后的切换，还要支持角色分工和并行互审：

~~~text
cheap_fast_model
  -> 初筛、分类、摘要、简单检索

strong_slow_model
  -> 复杂规划、关键定位、架构判断

specialized_model
  -> 代码、数学、安全、长上下文、视觉等专用任务

evaluator_model
  -> 独立验证、反例寻找、风险审查

local_model
  -> 隐私敏感、离线、低成本批处理
~~~

典型编排为：

~~~text
planner -> implementer -> reviewer -> evaluator
~~~

或：

~~~text
primary implementation
  + independent reviewer
  + test/evaluator
  -> fan-in decision
~~~

每个分支必须拥有独立的 TaskRun、输入/输出摘要、权限、预算、trace 和 join 规则；不得用自由群聊替代有界的 supervisor/orchestrator 编排。

### 4. ModelCallRecord 与评估

每次模型调用至少记录：

~~~text
model_call_id
task_run_id
provider_id
endpoint_id
model_id
route_decision_id
resolved_prompt_pack_id
input_context_hash
output_artifact_ref
latency_ms
input_tokens
output_tokens
estimated_cost
finish_reason
tool_call_count
verification_status
error_class
created_at
~~~

原始 prompt、模型输出和工具参数应按数据敏感性策略分级存储；审计记录默认保存哈希、摘要和 artifact 引用，不把 secret 或未经授权的敏感正文写进公共 trace。

模型评估至少区分：模型静态能力、单次调用结果、跨样本动态指标和路由策略效果。动态指标必须能回溯到 source_trace_ids，并通过回归门后才允许更新 registry 或 PromptProfile。

## 四、全自主 loop 的检查点策略

### 1. 是否主流

当前趋势是：长时运行和自主 agent 必须支持 checkpoint / restore / persistence / interrupt / rollback。

但更准确地说，成熟方向不是“每一步都固定 checkpoint”，而是：

```text
fixed checkpoint + event-triggered checkpoint + risk-triggered checkpoint + operator-marked checkpoint + side-effect ledger
```

原因：

- 只靠固定 checkpoint，成本高，且可能错过高风险动作前的关键恢复点
- 只靠事件触发 checkpoint，可能在长时间低风险任务中恢复粒度过粗
- 只保存聊天历史，无法恢复文件系统、进程状态和外部 side effect
- 盲目 restore 可能造成重复外部动作或语义回滚风险

### 2. 固定检查点

固定检查点用于提供最低恢复保障。

典型触发：

- 每 N 个 loop iteration
- 每 N 分钟
- 每 N 个文件变更
- 每 N 次工具调用
- 每个任务阶段结束

适用：

- 长时任务
- 资源不稳定环境
- 容易 OOM 或进程重启的 agent
- 低风险但长链路操作

缺点：

- 可能成本高
- 可能保存大量无意义状态
- 不一定覆盖关键风险动作前的状态

### 3. 非固定 / 事件触发检查点

事件触发检查点用于关键状态变化。

典型触发：

- 即将写文件
- 即将批量修改
- 即将运行 destructive command
- 即将执行外部 side effect
- evaluator 通过后
- GoalContract 更新后
- model routing 策略变化前
- skill/tool/policy 更新前
- 用户手动标记

适用：

- 软件开发智能体的代码修改
- 安全智能体的响应动作
- 工具/skill/MCP/policy 自动进化
- 任何不可轻易回滚的动作

### 4. 风险触发检查点

风险触发检查点由 `PolicyGate` 或 `RiskGate` 决定。

风险因素：

- 修改范围扩大
- 涉及核心文件
- 涉及安全策略
- 涉及权限提升
- 涉及外部系统
- 涉及不可逆 side effect
- loop 多轮没有收敛
- evaluator 连续失败
- agent 试图改动原本不在 scope 的内容

触发动作：

- 创建 hard checkpoint
- 暂停并请求审批
- 降级为只读模式
- 切换强模型/evaluator
- restore 到安全点
- fork 新分支探索

### 5. Operator-marked checkpoint

用户或控制面应能随时标记 checkpoint。

用途：

- “当前状态我认可，后续可以回到这里”
- “这个版本已经可用，后续探索不能破坏它”
- “进入高风险自动化前先保存”

字段建议：

```text
checkpoint_id
label
operator_id
reason
scope
durability_level
created_at
related_goal_id
related_trace_id
restore_preconditions
```

## 五、防跑偏与防破坏性改写

### 1. 问题定义

全自主推进最危险的不是 agent 报错，而是 agent 给出“看似合理”的理由，把已经成型的代码、架构或文档改得面目全非。

这类风险包括：

- scope creep
- over-refactor
- architecture drift
- style drift
- deleting useful edge cases
- weakening security checks
- changing behavior to satisfy tests only
- replacing project-specific practice with generic pattern
- modifying stable code because agent prefers another design

### 2. 需要的控制机制

建议至少有以下 gate：

- `ScopeGate`: 改动是否在 GoalContract scope 内
- `DiffSizeGate`: 改动规模是否异常
- `StableArtifactGate`: 是否触碰已标记稳定文件/模块
- `ArchitectureInvariantGate`: 是否破坏架构不变量
- `BehaviorChangeGate`: 是否改变外部行为
- `SecurityInvariantGate`: 是否降低安全边界
- `CheckpointGate`: 高风险改动前是否已有恢复点
- `HumanSteeringGate`: 是否需要人类审批

### 3. 固定检查项

每轮或每阶段固定检查：

- 当前动作是否服务目标
- 是否越过 scope
- 是否修改了未授权文件
- 是否扩大了改动范围
- 是否有验证证据
- 是否需要 checkpoint
- 是否需要 evaluator
- 是否需要停止并请求人类

### 4. 非固定检查项

事件触发检查：

- 大量 diff 出现
- 删除代码
- 修改公共接口
- 修改配置/权限/安全规则
- 测试通过但 diff 不合理
- agent 反复解释“为了更好架构”而扩大修改
- 工具失败后 agent 试图绕过工具
- 长时间没有验证还继续修改

## 六、与外部资料的对应

### LangGraph / LangSmith

可吸收：

- persistence / checkpointer / store
- interrupt / resume
- evaluation / experiment / trace

`steerBox` 应扩展：

- hard / soft checkpoint
- side-effect ledger
- operator-marked checkpoint
- risk-triggered checkpoint

### Cursor checkpoint

可吸收：

- 用户可以从 UI 恢复到先前检查点
- checkpoint 对代码修改体验很重要

`steerBox` 应扩展：

- 终端类 agent 也应有可靠 checkpoint
- checkpoint 应与 GoalContract、trace、side-effect ledger 绑定

### Crab / agent sandbox checkpoint research

可吸收：

- 不是每个 turn 都需要昂贵 checkpoint
- OS-visible effects 可以帮助判断哪些 turn 需要 checkpoint
- agent-level 状态和 OS-level side effect 之间存在语义鸿沟

`steerBox` 应扩展：

- 工具调用事件、文件系统变更、side-effect ledger 应联合判断 checkpoint 粒度
- checkpoint 成本进入 efficiency metrics

### ACRFence / semantic rollback risk

可吸收：

- agent restore 后可能重新合成不完全相同的外部请求
- 外部系统可能把重放请求当成新动作，导致重复支付、重复授权、凭证复活等风险

`steerBox` 应扩展：

- 恢复后对 irreversible tool call 采用 replay-or-fork 语义
- side-effect ledger 记录不可逆动作
- 高风险外部动作恢复后默认不自动重放

## 七、建议新增核心对象

### 1. ProviderProfile

~~~text
provider_id
provider_name
protocol_type
region
privacy_policy
supported_features
rate_limit
availability_policy
status
auth_ref
~~~

### 2. EndpointProfile

~~~text
endpoint_id
provider_id
model_id
deployment_name
region
context_window
cost_profile
latency_profile
availability_policy
capabilities_override
privacy_class
status
~~~

### 3. ModelProfile

```text
model_id
model_family
version
status
provider_ids
endpoint_ids
context_window
capabilities
cost_profile
latency_profile
strengths
weaknesses
tool_calling_quality
coding_quality
security_analysis_quality
instruction_following_score
known_failure_patterns
preferred_prompt_profile
compatible_prompt_profiles
forbidden_prompt_profiles
fallback_models
static_capabilities_verified_at
dynamic_performance
```

### 4. PromptProfile

```text
prompt_profile_id
version
status
compatible_provider_types
compatible_model_families
required_capabilities
intended_roles
instruction_style
context_style
tool_calling_style
output_contract
reasoning_budget
allowed_task_types
forbidden_task_types
risk_limit
required_preflight_checks
known_failure_patterns
last_verified_at
```

### 5. AgentRolePrompt

~~~text
role_prompt_id
role
version
system_prompt_fragments
tool_policy
output_contract
checkpoint_policy
evaluator_policy
compatible_model_families
status
~~~

### 6. TaskPromptPack

```text
task_prompt_pack_id
task_type
version
system_prompt_fragments
developer_prompt_fragments
required_context
forbidden_context
preflight_checklist
execution_checklist
verification_checklist
stop_conditions
handoff_template
failure_response_template
```

### 7. ResolvedPromptPack

~~~text
prompt_pack_id
base_prompt_version
provider_prompt_version
model_prompt_version
role_prompt_version
task_prompt_version
policy_summary_hash
tool_summary_hash
context_package_hash
resolution_reason
created_at
~~~

### 8. RoutePolicy

~~~text
route_policy_id
version
candidate_constraints
selection_weights
cost_budget
latency_target
quality_requirement
privacy_requirement
risk_rules
fallback_rules
collaboration_rules
approval_rules
status
~~~

### 9. RouteDecision

~~~text
route_decision_id
task_id
candidate_models
selected_provider
selected_model
selected_endpoint
selection_reason
rejected_candidates
cost_estimate
latency_estimate
fallback_chain
prompt_profile_id
prompt_profile_version
approval_required
created_at
~~~

### 10. ModelCallRecord

~~~text
model_call_id
task_run_id
provider_id
endpoint_id
model_id
route_decision_id
resolved_prompt_pack_id
input_context_hash
output_artifact_ref
latency_ms
input_tokens
output_tokens
estimated_cost
finish_reason
tool_call_count
verification_status
error_class
created_at
~~~

### 11. ModelEvaluation

~~~text
evaluation_id
model_id
prompt_profile_id
task_type
sample_count
measurement_window
source_trace_ids
metrics
confidence
regression_status
promotion_status
created_at
~~~

### 12. CheckpointPolicy

```text
policy_id
fixed_interval
loop_interval
tool_call_interval
event_triggers
risk_triggers
operator_marked_enabled
hard_checkpoint_conditions
soft_checkpoint_conditions
retention_policy
restore_policy
```

### 13. DriftGuard

```text
guard_id
scope_rules
stable_artifacts
architecture_invariants
max_diff_size
behavior_change_policy
security_invariants
human_approval_conditions
```

## 八、第一阶段与后续阶段边界

第一阶段不需要实现完整动态 runtime，但必须让多模型成为真实可扩展的接口，而不是一句未来愿景。

第一阶段至少实现或验证：

- 一个真实可调用的 model adapter
- 可配置多个 ProviderProfile、EndpointProfile 和 ModelProfile
- 静态 ModelRegistry 与 PromptRegistry
- AgentRolePrompt、TaskPromptPack、ResolvedPromptPack schema
- 基于能力、风险、成本和延迟约束的规则式 RoutePolicy
- RouteDecision、ModelCallRecord 的持久化 trace
- cheap/fast 只读任务到强模型复杂任务的最小路由 fixture
- fallback 的 schema、能力兼容检查和非静默事件

同时完成既有 checkpoint / drift 对象和控制门占位：

- `ModelProfile` 文档 schema
- `PromptProfile` 文档 schema
- `TaskPromptPack` 文档 schema
- `CheckpointPolicy` 文档 schema
- `DriftGuard` 文档 schema
- 在 `LoopController` 中预留 `CheckpointGate` 和 `DriftGuard`
- 在 `ModelRouter` 中预留 `preferred_prompt_profile`

第一阶段不要求：

- 多 provider 的全部协议差异都已接通
- 自动学习路由权重
- 自动生成或自动提升系统提示
- 无人审批的高风险 fallback 或模型协作
- 完整的模型市场、在线评测平台或训练流程

Phase 1.5 可加入多 provider live routing、成本 / 延迟预算、能力匹配、fallback chain、并行模型审查和 route evaluation；Phase 2 再加入历史表现驱动的自适应路由、跨项目模型画像和受治理的 prompt/model 自动进化。

## 当前结论

Provider、Model、Endpoint、Prompt、Route 和 ModelCall 必须分层并进入 trace；模型切换不能静默，模型协作必须有独立运行、预算和 join 规则；secret 只能通过外部引用进入 adapter，不能进入 registry、prompt、trace、event log 或 checkpoint。

多模型系统提示多态化是必要的，而且应做成可追踪、可评估、可进化的 `PromptProfile` / `TaskPromptPack`，而不是手工散落在不同 prompt 文件里。

全自主 loop 必须同时支持固定检查点和非固定检查点。更进一步，还需要风险触发检查点、用户标记检查点、side-effect ledger 和 drift guard，才能防止 agent 把已经成型的项目按自己的“合理理由”改得面目全非。
