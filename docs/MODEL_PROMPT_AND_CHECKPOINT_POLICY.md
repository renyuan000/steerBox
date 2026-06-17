# Model Prompt and Checkpoint Policy Discussion Draft

## 文档目的

本文件用于沉淀 `steerBox` 关于多模型提示符多态化、模型专用系统提示、全自主 loop 检查点策略、防跑偏和防破坏性改写的设计方向。

本文件是讨论稿，不是最终实现规格，也不代表当前仓库已经具备对应能力。

## 当前判断

`steerBox` 应同时支持：

1. 多模型路由
2. 模型专用 prompt profile
3. 任务专用 prompt pack
4. 固定检查点
5. 非固定 / 事件触发检查点
6. 风险触发检查点
7. operator-marked checkpoint
8. side-effect ledger
9. rollback / replay / fork 策略

原因：不同模型的能力、服从度、工具调用稳定性、长上下文可靠性、成本、响应速度和安全边界都不同。如果所有模型共用同一套系统提示和 loop 策略，通常会浪费强模型、压垮弱模型，也更容易跑偏。

## 一、多模型提示符多态化

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
model_id
model_family
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
  + ModelPromptProfile
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

### 6. 不同模型的典型分工

建议模式：

- 低成本模型：分类、摘要、格式整理、低风险检索、重复检查
- 中等模型：常规代码定位、文档整理、日志初筛、简单修复建议
- 强模型：架构设计、疑难 bug、安全分析、复杂 refactor 策略、关键决策
- 本地模型：隐私敏感数据、离线环境、低成本批处理、初筛
- evaluator 模型：独立审查、反例寻找、风险判断、完成度判断

### 7. PromptProfile 的进化

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

## 二、全自主 loop 的检查点策略

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

## 三、防跑偏与防破坏性改写

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

## 四、与外部资料的对应

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

## 五、建议新增核心对象

### 1. ModelProfile

```text
model_id
provider
context_window
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
fallback_models
```

### 2. PromptProfile

```text
prompt_profile_id
model_id
instruction_style
context_style
tool_calling_style
output_contract
risk_limit
required_preflight_checks
known_failure_patterns
last_verified_at
```

### 3. TaskPromptPack

```text
task_type
system_fragments
developer_fragments
required_context
preflight_checklist
execution_checklist
verification_checklist
stop_conditions
```

### 4. CheckpointPolicy

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

### 5. DriftGuard

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

## 六、第一阶段建议

第一阶段不需要实现完整 runtime，但设计必须占位。

建议先做：

- `ModelProfile` 文档 schema
- `PromptProfile` 文档 schema
- `TaskPromptPack` 文档 schema
- `CheckpointPolicy` 文档 schema
- `DriftGuard` 文档 schema
- 在 `LoopController` 中预留 `CheckpointGate` 和 `DriftGuard`
- 在 `ModelRouter` 中预留 `preferred_prompt_profile`

## 当前结论

多模型系统提示多态化是必要的，而且应做成可追踪、可评估、可进化的 `PromptProfile` / `TaskPromptPack`，而不是手工散落在不同 prompt 文件里。

全自主 loop 必须同时支持固定检查点和非固定检查点。更进一步，还需要风险触发检查点、用户标记检查点、side-effect ledger 和 drift guard，才能防止 agent 把已经成型的项目按自己的“合理理由”改得面目全非。
