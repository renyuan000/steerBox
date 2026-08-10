# Phase 1 Core Contracts

## 文档定位

本文件是 `steerBox` Phase 1 的规范契约层。

它把以下文档中的对象和边界收敛成实现时必须遵守的最小规则：

- `PHASE_1_IMPLEMENTATION_SCOPE.md`
- `PHASE_1_DESIGN.md`
- `PHASE_1_TODO.md`
- `PHASE_1_VALIDATION_PLAN.md`
- `CONTROL_PLANE_DISCUSSION.md`
- `MODEL_PROMPT_AND_CHECKPOINT_POLICY.md`

本文件只约束 Phase 1 的本地、单项目、顺序执行 fixture。它不承诺真实多项目并行调度、分布式 worker、完整 TUI/Web control plane 或完整 self-evolution runtime。

如果长期讨论稿与本文件冲突，Phase 1 实现以本文件为准，并在后续文档同步提交中修正引用关系。新增字段可以向后兼容地扩展；改变既有语义必须新增 schema 版本或 ADR，不能静默改变。

## 一、Phase 1 实现边界

Phase 1 的最小闭环是：

~~~text
one Portfolio
  -> one Project
      -> one Goal
          -> one Task
              -> one TaskRun
                  -> one or more Attempt
                      -> EventLog + StateStore + TraceStore
                          -> BoardProjection / run summary
~~~

必须支持：

- 一个真实软件开发任务的 Goal / Loop / Tool / Verification 闭环
- 一个真实 model adapter、多个静态 model fixture 和规则式路由
- 本地 EventLog、StateStore、TraceStore 和 checkpoint metadata
- Task / TaskRun / Attempt 的最小身份与状态
- 从事件重建只读 BoardProjection
- `ToolUseErrorEvent -> LessonCandidate -> verified local recipe` 的候选链，但不自动提升全局 policy

明确不实现：

- 真实多项目并行调度
- Scheduler 的公平性、分布式 queue 和跨机器 worker
- worktree lease 的并行写入协调
- 多模型 planner/implementer/reviewer 协作 runtime
- 自动 prompt、skill、policy 或模型路由 promotion
- 完整安全智能体处置闭环

## 二、公共标识与版本

所有持久化对象和事件必须使用以下公共规则：

| 字段 | 规则 |
|---|---|
| `*_id` | 由 runtime 生成的不透明稳定字符串；业务不得解析其内部结构 |
| `schema_version` | 对象或事件 payload 的整数版本；从 `1` 开始 |
| `object_version` | 可变状态的乐观并发版本；每次成功状态更新递增 |
| `event_id` | 事件唯一 ID；重复提交不得产生第二份事件 |
| `sequence_no` | EventLog 的单调递增顺序；投影按它消费，不按 wall clock 排序 |
| `occurred_at` | 业务动作发生时间，UTC RFC 3339 |
| `recorded_at` | durable 写入时间，UTC RFC 3339 |
| `correlation_id` | 一次用户命令或一次 TaskRun 的关联 ID |
| `causation_id` | 触发当前事件的上一个事件或命令 ID |

时间戳只用于展示和诊断，不能替代 `sequence_no` 判断事件顺序。

Phase 1 允许 fixture 使用可预测 ID，但生产实现不得依赖固定 ID。

## 三、EventEnvelope 契约

所有 EventLog 事件使用统一 envelope：

~~~text
EventEnvelope {
  event_id
  event_type
  schema_version
  sequence_no
  aggregate_type
  aggregate_id
  portfolio_id?
  project_id?
  goal_id?
  task_id?
  task_run_id?
  attempt_id?
  loop_run_id?
  step_id?
  correlation_id
  causation_id?
  producer
  occurred_at
  recorded_at
  payload
  redaction_class
}
~~~

要求：

1. `payload` 只放该事件的业务数据，不能把完整模型认证请求、API key、token、cookie、credential response 或未授权敏感正文写入事件。
2. 工具输入输出、模型输入输出按敏感性只保存摘要、hash 或 artifact ref；原始正文如果需要保存，必须经过明确的数据分类和 PolicyGate。
3. 事件 append-only；修正事实必须新增补偿或更正事件，不能覆盖历史事件。
4. EventLog append 成功后，StateStore 可以稍后重建；StateStore 是可重建的当前状态投影，不是第二套事实源。
5. 相同 `event_id` 或相同 mutating command 的重复提交必须返回既有结果，不重复执行副作用。

Phase 1 最小事件集合：

~~~text
goal_created
portfolio_created
project_registered
task_created
task_ready
task_queued
task_run_started
attempt_started
loop_started
step_started
context_loaded
model_route_decided
prompt_pack_resolved
model_call_started
model_call_completed
tool_call_requested
tool_call_completed
tool_use_error_recorded
plan_created
file_change_recorded
verification_started
verification_completed
policy_decision
side_effect_recorded
checkpoint_created
lesson_candidate_created
human_intervention
task_run_waiting
task_run_succeeded
task_run_failed
task_blocked
task_canceled
attempt_finished
loop_completed
loop_failed
~~~

## 四、核心对象关系

### 1. Goal / Loop / TaskRun

~~~text
Portfolio
  -> Project
      -> Goal
          -> Task
              -> TaskRun
                  -> Attempt
                      -> LoopStep
                          -> ToolCall / ModelCall / Verification
~~~

- `Goal` 是可验证目标契约，定义 objective、scope、out_of_scope、success criteria、verification plan 和风险边界。
- `Task` 是可调度工作项，不等同于一次执行。
- `TaskRun` 是 Task 的一次执行 lineage，保留 worker、workspace、budget 和最终状态。
- `Attempt` 是 TaskRun 内一次实际尝试；retry、fallback、resume 和 fork 都创建新 Attempt。
- `LoopStep` 是 Attempt 内的可审计执行单元；每个 step 必须有 start 和 terminal result。

### 2. Phase 1 核心对象字段

以下字段是 Phase 1 最小稳定边界；公共 ID、`schema_version`、`object_version` 和 UTC 时间戳规则按本文件第二节执行。未列出的字段不得被实现默认推断为跨模块契约。

~~~text
Portfolio:
  portfolio_id
  name
  policy_scope
  project_ids
  resource_budget
  model_budget
  status

Project:
  project_id
  portfolio_id
  name
  workspace_ref
  policy_ref
  default_model_policy
  status

Goal:
  goal_id
  project_id
  objective
  scope
  out_of_scope
  success_criteria
  quality_bar
  constraints
  risk_level
  allowed_tools
  forbidden_tools
  allowed_models
  model_routing_policy
  verification_plan
  stop_conditions
  human_steering_mode
  checkpoint_policy
  side_effect_boundary
  rollback_or_compensation_plan

Task:
  task_id
  project_id
  goal_id
  title
  task_type
  priority
  risk_level
  domain_state
  dependency_ids
  required_capabilities
  required_artifacts
  success_criteria
  verification_plan
  owner

TaskDependency:
  dependency_id
  project_id
  upstream_task_id
  downstream_task_id
  dependency_type       # finish_to_start | data | verification | approval | resource
  required_artifact_refs?
  created_at

TaskRun:
  task_run_id
  task_id
  worker_id?
  workspace_ref?
  budget
  status
  verification_status
  artifact_refs
  evidence_refs
  blocking_issue_refs
  started_at
  heartbeat_at?
  finished_at?
~~~

`Task.domain_state` 使用本文件第五节的状态机。`TaskRun.status` 只描述该次执行 lineage 的摘要，不能替代 Task 的领域状态。Phase 1 可以让 `worker_id`、`workspace_ref`、`required_artifact_refs` 和 `blocking_issue_refs` 为空，但必须保留字段位置；不得落盘 secret、API key、token 或认证响应。

### 3. TaskRun 与 Attempt 规则

每个 Attempt 必须记录：

~~~text
attempt_id
task_run_id
attempt_no
reason            # initial | retry | fallback | resume | fork
worker_id?
workspace_ref?
route_decision_id?
resolved_prompt_pack_id?
parent_attempt_id?
started_at
finished_at?
status
verification_status
artifact_refs
evidence_refs
~~~

Attempt 结束后不能重新变成运行中。需要继续执行时创建新 Attempt，并通过 `parent_attempt_id` 保留关系。

## 五、Task 状态机

### 1. 状态定义

~~~text
backlog
ready
queued
running
waiting_dependency
waiting_human
verifying
succeeded
blocked
failed
canceled
superseded
archived
~~~

### 2. Phase 1 允许迁移

~~~text
backlog -> ready | canceled
ready -> queued | blocked | canceled
queued -> running | blocked | canceled
running -> waiting_dependency | waiting_human | verifying | failed | canceled
waiting_dependency -> ready | blocked | canceled
waiting_human -> running | blocked | canceled
verifying -> succeeded | failed | waiting_human | canceled
failed -> queued | archived | canceled
blocked -> ready | canceled | archived
succeeded -> archived | superseded
superseded -> archived
canceled -> archived
archived -> terminal
~~~

`terminal` 不是持久化的 Task 状态，只表示不再允许业务迁移。

非法迁移必须：

- 拒绝执行
- 记录 policy/state decision
- 保留当前 `object_version`
- 不产生伪造的成功事件

### 3. succeeded 的强制条件

Task 只能在以下条件全部满足时进入 `succeeded`：

- success criteria 已逐项得到验证
- 至少一个 VerificationResult 为通过
- required evidence 已存在并可反查
- PolicyGate 没有未解决的阻断
- 当前 Attempt 已结束
- 若存在 join/conflict 检查，则已通过

TODO / DOING / DONE 是 BoardProjection 的显示列，不是领域状态。

## 六、BoardProjection 契约

BoardProjection 由以下输入确定性派生：

~~~text
EventLog + rebuildable StateStore
  -> BoardProjection
      -> CLI / JSON / TUI / Web adapter
~~~

Phase 1 只要求单 Portfolio、单 Project、本地顺序 fixture，但 projection 必须保留未来扩展字段：

~~~text
BoardTaskCard {
  portfolio_id
  project_id
  goal_id
  task_id
  task_run_id?
  current_attempt_id?
  domain_state
  board_column
  owner?
  worker?
  model?
  priority
  risk_level
  dependency_summary
  verification_status
  evidence_completeness
  blocking_issue_refs
  last_heartbeat?
  next_action?
  updated_at
}
~~~

显示映射固定为：

| Board column | Domain states |
|---|---|
| TODO | `backlog`, `ready`, `queued` |
| DOING | `running`, `waiting_dependency`, `waiting_human`, `verifying` |
| DONE | `succeeded`, `canceled`, `superseded`, `archived` |
| ATTENTION | `blocked`, `failed` |

Projection rebuild 验证必须比较：

- task domain state
- board column
- current attempt
- dependency summary
- verification status
- evidence completeness
- blocking issues

BoardProjection 损坏不能破坏 EventLog；删除并重建 projection 后结果必须一致。

## 七、命令、幂等与并发

所有会改变状态的 ControlPlaneCommand 或 runtime command 必须包含：

~~~text
command_id
command_type
actor_ref
target_type
target_id
expected_object_version?
correlation_id
requested_at
payload
~~~

规则：

1. 相同 `command_id` 重试返回第一次结果，不重复产生副作用。
2. 提供 `expected_object_version` 时，版本不匹配必须拒绝并返回 conflict；不能覆盖较新的状态。
3. 查询命令不写 EventLog；暂停、恢复、取消、重试、审批和 reprioritize 必须经过 authorization、PolicyGate 和事件记录。
4. Phase 1 不实现多 worker 并发写，但 schema 必须保留版本和 command_id，避免未来无法加入并发控制。

## 八、Verification、Artifact 与 Evidence

`VerificationResult` 至少包含：

~~~text
verification_id
task_id
task_run_id
attempt_id
check_type
command_or_evaluator_ref
status             # passed | failed | skipped | blocked
exit_code?
output_summary?
artifact_refs
failure_category?
started_at
finished_at
~~~

`Artifact` 是产物引用；`Evidence` 是支持某个状态或结论的证据。二者不能只用自由文本互相替代。

Phase 1 的 run summary 必须能从 task、attempt、event、verification、checkpoint、side effect 和 evidence 反查到原始记录。

## 九、错误、失败和恢复

Phase 1 最小 failure category：

~~~text
schema_error
storage_error
policy_blocked
tool_error
model_error
context_error
verification_failed
checkpoint_error
side_effect_uncertain
human_required
cancelled
timeout
~~~

失败处理规则：

- tool/model/network 失败不直接修改既有 policy。
- retry、fallback、resume、fork 产生新 Attempt，并保留失败 Attempt 的证据。
- side effect 不确定时默认进入 `waiting_human` 或 `blocked`，不能自动重放。
- checkpoint 只承诺 metadata 中明确覆盖的范围。
- 失败信息必须可审计，但敏感正文按 `redaction_class` 处理。

## 十、Schema 与迁移

- 所有持久化表、JSONL、registry 和 artifact metadata 都带 schema version。
- Phase 1 只允许向后兼容的新增字段和新增事件；改变字段语义必须提升版本。
- 读取旧版本时必须显式执行 migration 或拒绝读取，不能静默按新语义解释。
- migration 必须可重复执行，失败时保留原始数据并产生 migration event。
- 任何破坏性迁移在 Phase 1 默认要求人工确认和备份/回滚依据。

## 十一、Secret 与敏感数据边界

- registry、prompt、trace、event、checkpoint、run summary 只保存 `auth_ref`，不保存 API key、token 或认证响应。
- provider adapter 只在调用边界解析 `auth_ref`；解析后的 secret 不进入普通日志和错误摘要。
- 测试 fixture 使用假值或不可用占位符，不能复用真实认证信息。
- 日志与 trace 默认保存摘要、hash 或 artifact ref；保存敏感原文必须经过 PolicyGate 和数据分类。

## 十二、Phase 1 验收映射

实现任务与验证必须能够对应到以下契约：

| 契约 | 最小验证 |
|---|---|
| EventEnvelope | 字段、版本、顺序、重复事件去重 |
| Task state machine | 合法迁移通过、非法迁移拒绝 |
| Attempt | retry/fallback/resume/fork 都产生新 Attempt |
| BoardProjection | EventLog 重建结果稳定 |
| Command | command_id 幂等、version conflict 被拒绝 |
| Verification | succeeded 缺 evidence 时不能通过 |
| Secret boundary | 序列化和 trace 中没有认证明文 |
| Failure category | 失败可分类并可反查原始证据 |

## 当前结论

完成本文件并同步 Phase 1 Scope、Design、TODO、Validation 后，才可以把 Phase 1 标记为：

```text
design contract locked
```

这仍不等于 runtime 已实现；实现完成必须由代码、测试、运行日志和可复现验证证明。
