# Goal Alignment Checkpoints Discussion Draft

## 文档目的

本文件用于定义 `steerBox` 在长程自主 loop 中用于检查和纠正任务方向偏离的机制。

本文件是讨论稿，不是最终实现规格。

## 为什么要和 RecoveryCheckpoint 区分

当前文档中的 `checkpoint` 主要指：

- restore checkpoint
- 状态恢复点
- 文件/工作区快照
- sandbox/process checkpoint
- side-effect ledger cursor

它回答的是：

> 出错或中断后，如何恢复到某个状态。

但本文件讨论的是另一类机制：

> 长程自主执行过程中，agent 如何定期或事件触发地检查自己是否仍然朝最初任务目标前进，并在偏离时纠正方向。

为了避免和 restore checkpoint 混淆，建议明确使用不同术语。

本项目建议采用：

> `Goal Alignment Checkpoint`

中文可称：

> 目标对齐检查点 / 阶段方向校准点

说明：这里保留 `Checkpoint` 一词，是因为它确实是长程任务中的检查点；但它不表示可恢复快照。恢复快照统一叫 `RecoveryCheckpoint`。

## 与 restore checkpoint 的区别

| 概念 | 目的 | 触发时机 | 输出 | 是否用于恢复 |
|---|---|---|---|---|
| Restore Checkpoint | 保存状态，支持恢复 | 写文件前、风险动作前、固定周期、人工标记 | checkpoint id、状态快照、restore policy | 是 |
| Goal Alignment Checkpoint | 检查是否偏离初始目标、当前阶段目标，并结合阶段链判断是否合理 | 每 N 轮、阶段切换、scope 扩大、长时间无验证、模型切换后 | alignment decision、偏离原因、阶段链解释、纠偏动作 | 不直接用于恢复 |

两者可以联动：

- `Goal Alignment Checkpoint` 发现严重偏离时，可以触发 restore checkpoint 回退
- restore checkpoint 恢复后，应立即执行 `Goal Alignment Checkpoint`，确认恢复后的下一步仍符合目标

## 核心目标

`Goal Alignment Checkpoint` 负责回答：

- 当前行动是否仍服务最初目标
- 当前计划是否超出 scope
- 当前 agent 是否在解决未要求的问题
- 当前改动是否为了验证目标，还是为了模型自己的偏好
- 当前 loop 是否长时间没有收敛
- 当前是否需要回到 GoalContract、请求人类确认或停止
- 当前行动是否服务当前阶段目标
- 当前阶段目标是否仍能追溯到初始任务目标
- 当前行动如果看似偏离初始任务，是否能通过已执行阶段链解释为必要中间步骤

## 建议放在 loop 中的位置

基础 loop 应调整为：

```text
observe
  -> goal alignment checkpoint
  -> plan
  -> pre-action alignment checkpoint
  -> act
  -> record
  -> verify
  -> post-action alignment checkpoint
  -> reflect
  -> update memory/context/goal
  -> recovery checkpoint decision
  -> next action / stop
```

说明：

- `pre-action alignment checkpoint`: 行动前防止越界
- `post-action alignment checkpoint`: 行动后检查是否真的推进目标
- `recovery checkpoint decision`: 只负责是否保存/恢复状态

## GoalAlignmentCheckpoint 输入

```text
goal_contract
root_goal
root_objective
current_stage
stage_goal
stage_success_criteria
stage_chain
completed_stages
current_plan
current_action
loop_history
recent_tool_calls
recent_file_changes
verification_result
alignment_history
human_instruction
memory_retrieval_result
model_routing_decision
```

## GoalAlignmentCheckpoint 输出

```text
alignment_status: aligned | weakly_aligned | drifting | off_goal | unclear
root_alignment_status: aligned | weakly_aligned | drifting | off_goal | unclear
stage_alignment_status: aligned | weakly_aligned | drifting | off_stage | unclear
stage_chain_status: justified | weakly_justified | unjustified | missing
alignment_score: 0.0-1.0
root_alignment_score: 0.0-1.0
stage_alignment_score: 0.0-1.0
drift_type:
  - none
  - root_goal_drift
  - stage_goal_drift
  - stage_chain_break
  - scope_expansion
  - solution_overreach
  - over_refactor
  - tool_chasing
  - verification_avoidance
  - architecture_drift
  - user_intent_loss
  - security_boundary_drift
reason
stage_chain_justification
required_action:
  - continue
  - revise_stage_plan
  - revise_global_plan
  - shrink_scope
  - run_verification
  - ask_human
  - switch_model
  - run_evaluator
  - restore_checkpoint
  - stop
```

## 三层对齐判断

`Goal Alignment Checkpoint` 不应只把当前动作和最初目标做简单相似度比较。长程任务经常需要中间阶段，这些阶段可能看起来离初始目标较远，但实际上是必要铺垫。

因此建议分三层判断。

### 1. RootGoalAlignment

判断当前行动、当前阶段和最初任务目标之间是否仍有合理关系。

它回答：

- 当前工作最终是否仍服务 root goal
- 是否已经偏离用户最初要解决的问题
- 当前阶段是否仍在原始 scope 或经批准扩展后的 scope 内

### 2. StageGoalAlignment

判断当前行动是否服务当前阶段目标。

它回答：

- 当前动作是否服务当前 stage goal
- 当前动作是否是阶段内必要步骤
- 当前动作是否跳到了另一个未批准阶段

### 3. StageChainJustification

判断当前阶段虽然看似离 root goal 较远，但是否能通过已执行阶段链解释为必要中间步骤。

它回答：

- 当前阶段是否由前一阶段自然推出
- 当前阶段是否有已记录的阶段决策依据
- 当前阶段完成后如何回到 root goal
- 是否存在阶段链断裂或无限铺垫

示例：

```text
Root goal: 修复安全日志分析 agent 的误报问题
Stage 1: 读取现有日志 schema
Stage 2: 补充 evaluation dataset
Stage 3: 调整 evaluator 规则
Stage 4: 回归验证误报率
```

其中 Stage 2 看起来不是直接“修 bug”，但如果阶段链证明它是修复误报的必要评估基础，就不应被误判为 off_goal。

## 触发策略

### 固定触发

```yaml
fixed_alignment_checkpoints:
  every_loop_iterations: 2
  every_minutes: 5
  every_tool_calls: 5
```

适用：长程自主任务最低方向保障。

### 阶段触发

```yaml
phase_alignment_checkpoints:
  before_plan: true
  before_file_write: true
  after_file_write: true
  before_scope_change: true
  after_verification_failure: true
  before_subagent_spawn: true
  after_model_switch: true
```

适用：关键阶段方向确认。

### 风险触发

```yaml
risk_alignment_checkpoints:
  when_scope_expands: true
  when_diff_size_large: true
  when_no_verification_for_n_loops: 3
  when_repeated_tool_failure: 2
  when_agent_proposes_rearchitecture: true
  when_security_boundary_touched: true
  when_user_intent_uncertain: true
```

适用：偏离风险上升时强制校准。

## 阶段链数据结构草案

```text
stage_id
stage_goal
parent_stage_id
root_goal_id
entry_reason
expected_output
success_criteria
verification_plan
status
evidence
next_stage_hint
return_to_root_explanation
```

关键要求：

- 每个 stage 必须说明为什么存在
- 每个 stage 必须能追溯到 root goal
- 每个 stage 必须有退出条件
- 长时间无法回到 root goal 时，应触发 human alignment intervention

## Alignment Decision 分级

### aligned

当前动作明确服务目标。

处理：继续。

### weakly_aligned

可能服务目标，但理由不够强或证据不足。

处理：缩小动作、补充验证、减少改动范围。

### drifting

开始偏离目标，例如顺手优化、扩大范围、长时间无验证。

处理：暂停执行当前计划，重新读取 GoalContract，生成收缩后的 plan。

### off_goal

已经明显偏离目标。

处理：停止当前动作，请求人类或恢复到最近安全点。

### unclear

无法判断是否对齐。

处理：请求人类确认，或切换强模型/evaluator 判断。

## 与 DriftGuard 的关系

`Goal Alignment Checkpoint` 和 `DriftGuard` 不一样。

- `Goal Alignment Checkpoint`: 判断是否仍在朝任务目标前进
- `DriftGuard`: 判断改动是否破坏代码、架构、安全边界或稳定 artifacts

例子：

- agent 没有改文件，但一直在读无关资料：Goal Alignment Checkpoint 会发现偏离；DriftGuard 可能不会触发
- agent 改了核心架构文件，即使理由和目标相关：DriftGuard 会触发；Goal Alignment Checkpoint 可能认为目标相关但风险过高

推荐联动：

```text
GoalAlignmentCheckpoint detects drifting
  -> shrink scope or revise plan

DriftGuard detects high-risk diff
  -> require evaluator / human approval / restore checkpoint
```

## 与 Restore Checkpoint 的关系

`Goal Alignment Checkpoint` 本身不是恢复点。

但它可以触发恢复动作：

```text
alignment_status = off_goal
  -> find nearest safe restore checkpoint
  -> inspect side-effect ledger
  -> restore or fork
  -> rerun Goal Alignment Check
```

因此建议命名区分：

- `RecoveryCheckpoint`: 可恢复状态点
- `GoalAlignmentCheckpoint`: 方向校准点
- `DriftGuard`: 破坏性改动防护

## 与人类介入的关系

长程自主任务不应该每轮都问人，但以下情况应请求人类：

- `off_goal`
- `unclear` 且高风险
- 多次 `drifting`
- 需要扩大 scope
- 需要改 architecture invariant
- 需要执行不可逆 side effect
- GoalContract 本身需要修改

## 与模型路由的关系

不同模型对齐能力不同。

如果低成本模型连续出现：

- scope expansion
- tool chasing
- over-refactor
- verification avoidance

则 `ModelRouter` 应考虑切换强模型或 evaluator。

记录指标：

- `alignment_failure_rate_by_model`
- `alignment_repair_success_rate`
- `model_switch_after_drift_count`

## 指标

建议记录：

- `alignment_check_count`
- `alignment_drift_count`
- `off_goal_count`
- `alignment_repair_success_rate`
- `loops_between_alignment_checks`
- `unverified_loop_count`
- `scope_expansion_attempt_count`
- `human_alignment_intervention_count`
- `restore_triggered_by_alignment_count`

## 第一阶段建议

第一阶段可先实现文档/schema 级设计：

- `GoalAlignmentCheckpoint`
- `AlignmentDecision`
- `AlignmentEvent`
- `AlignmentPolicy`

最小检查清单：

- 当前动作是否直接服务 objective
- 当前动作是否在 scope 内
- 当前动作是否触碰 out_of_scope
- 当前动作是否扩大任务范围
- 最近 N 轮是否有验证
- 是否出现重复工具失败仍继续推进
- 是否需要人类确认

## 当前结论

你说的“检查点”应该单独命名，不应该和 restore checkpoint 混用。

建议在 `steerBox` 中明确区分：

```text
RecoveryCheckpoint   -> 用于恢复状态
GoalAlignmentCheckpoint   -> 用于检查和纠正 root goal / stage goal / stage chain 方向
DriftGuard           -> 用于防止破坏性改动
SideEffectLedger     -> 用于处理外部副作用
```

这样后续设计 loop 时不会把“能恢复”和“没跑偏”混为一谈。
