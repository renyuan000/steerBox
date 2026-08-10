# Portfolio, Scheduler, and Control Plane Discussion Draft

## 文档目的

本文件用于收敛 steerBox 的多项目、多任务、并行调度、协同执行、进度看板和控制面设计。

本文件是设计讨论稿，不代表当前仓库已经实现这些能力。第一阶段仍以 shared harness core + software development agent vertical slice 为边界；控制面和跨项目编排必须分阶段落地，不能把未来愿景写成当前功能。

## 核心判断

命令行形态不等于只能运行单个任务。steerBox 应同时具备：

- 可持久化的 Portfolio / Project / Goal / Task / TaskRun 模型
- 依赖、优先级、资源、模型、权限和验证约束下的调度
- 多 worker 并行、隔离、汇合、取消、重试和背压
- 从事实事件派生的 TODO / DOING / DONE 看板
- CLI / TUI / JSON 为主，未来可接 Web 的统一 BoardProjection
- 跨项目经验、模型画像和进化提案的受治理提升
- 每个状态、进度、问题、验证和完成结论可追溯到 evidence

看板不是事实源，控制面也不能绕过 PolicyGate。正确关系是：

~~~text
EventLog + StateStore
  -> BoardProjection
      -> CLI / TUI / JSON / Web adapter

ControlPlaneCommand
  -> authorization
  -> PolicyGate
  -> audited domain command
  -> EventLog
  -> updated BoardProjection
~~~

## 一、一等领域对象

### 1. 层级与运行关系

~~~text
Portfolio
  -> Project
      -> Goal
          -> Task
              -> TaskRun
                  -> Attempt
~~~

定义：

- Portfolio：多个项目的管理边界、资源预算和治理范围
- Project：一个仓库、工作区或长期业务目标的稳定边界
- Goal：可验证的目标契约，包含 scope、success criteria 和 verification plan
- Task：可调度工作项，不等同于一次实际执行
- TaskRun：Task 的一次运行实例，绑定 worker、模型、workspace 和预算
- Attempt：TaskRun 内因 retry、fallback、resume 或 fork 产生的独立尝试

Task 与 TaskRun 必须分开。一个 Task 可以没有运行、运行多次、失败后重试或由不同模型执行；看板不能用一条可变记录覆盖这些历史。

### 2. 依赖与跨项目关系

至少需要：

~~~text
TaskDependency
CrossProjectLink
Artifact
Evidence
ArtifactEvidenceLink
~~~

TaskDependency 支持：

- finish-to-start
- data dependency
- verification dependency
- approval dependency
- resource dependency

CrossProjectLink 只描述显式关系，例如：

- upstream / downstream
- shared component
- compatibility constraint
- shared incident
- shared evolution proposal

跨项目链接不能自动授权跨项目写入；每个项目仍有独立 scope、PolicyGate、workspace ownership 和 approval boundary。

### 3. 核心字段草案

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

Task:
  task_id
  project_id
  goal_id
  title
  task_type
  priority
  risk_level
  status
  dependency_ids
  required_capabilities
  required_artifacts
  success_criteria
  verification_plan
  owner

TaskRun:
  task_run_id
  task_id
  attempt_no
  worker_id
  workspace_lease_id
  route_decision_id
  resolved_prompt_pack_id
  started_at
  heartbeat_at
  finished_at
  status
  verification_status
  artifact_refs
  evidence_refs
  blocking_issue_refs
~~~

## 二、任务状态机与敏捷看板映射

### 1. 领域状态

任务状态必须比 TODO / DOING / DONE 更精确：

~~~text
backlog
  -> ready
  -> queued
  -> running
  -> waiting_dependency
  -> waiting_human
  -> verifying
  -> succeeded

terminal or exceptional:
blocked
failed
canceled
superseded
archived
~~~

禁止直接把 running 改成 done。进入 succeeded 至少要求：

- success criteria 满足
- verification result 通过
- required evidence 存在
- policy requirements 满足
- join / conflict checks 通过

### 2. BoardProjection

TODO / DOING / DONE 只是显示映射：

| Board column | Domain states |
|---|---|
| TODO | backlog, ready, queued |
| DOING | running, waiting_dependency, waiting_human, verifying |
| DONE | succeeded, canceled, superseded, archived |
| ATTENTION | blocked, failed |

DONE 列必须显示具体终态，不能把 canceled、superseded 和 succeeded 混成同一种完成。

每张 task card 至少显示：

- project / goal / task
- owner / worker / model
- 当前 domain state 和 board column
- priority / risk / cost / latency budget
- dependency progress
- 当前 attempt 和最后 heartbeat
- 当前问题与 blocking reason
- validation status
- artifacts / evidence
- next action

## 三、职责分离

### 1. Supervisor

负责 runtime 生命周期、worker 健康检查、异常退出和恢复，不负责拆任务或排序 ready queue。

### 2. Scheduler

负责：

- 依赖解析与 ready queue
- priority / fairness
- concurrency limit
- resource / model / tool capability matching
- workspace lease
- rate limit 和 backpressure
- retry / timeout / cancellation eligibility

Scheduler 不直接生成计划，也不执行工具。

### 3. Orchestrator

负责：

- 分解任务
- fan-out / fan-in
- 生成 HandoffContract
- 为子任务定义 join policy
- 汇总 artifacts 和 evidence

Orchestrator 受 GoalContract、PolicyGate、budget 和 trace 约束，不能成为不受控的总指挥。

### 4. Worker

负责执行一个明确 TaskRun，只持有最小必要上下文、工具权限、模型权限和 workspace lease。

### 5. Control Plane

负责查询、观察和发出受审计命令：

- list / show / watch
- pause / resume / cancel
- reprioritize
- approve / reject
- retry / fork
- mark operator checkpoint
- inspect evidence / trace / blocking issue

Control Plane 不直接修改 EventLog 或 StateStore。所有写命令必须转换为领域命令，经过授权、PolicyGate 和事件记录。

## 四、并行执行与协同安全

并行 worker 的最小安全边界：

- workspace / worktree isolation
- file or module ownership
- resource lease with expiry
- model / provider rate-limit lease
- duplicate-work detection
- conflict detection
- join policy
- cancellation propagation
- retry budget
- backpressure

同一 workspace 的并行写默认禁止。需要并行修改时，应使用独立 worktree / branch / patch artifact，并在 fan-in 阶段执行：

~~~text
collect artifacts
  -> verify each TaskRun
  -> compare overlapping writes
  -> conflict check
  -> policy check
  -> integration test
  -> join decision
  -> record merged or rejected artifacts
~~~

JoinPolicy 至少区分：

- all_required
- quorum
- first_valid
- reviewer_approved
- manual_selection

失败 worker 不应无限重试；重试、fallback 和 fork 都必须产生新 Attempt 并计入预算。

## 五、多模型路由与任务编排结合

TaskRun 创建前由 ModelRouter 生成 RouteDecision，考虑：

- task type
- agent role
- risk level
- required capabilities
- privacy requirement
- context requirement
- tool requirement
- cost budget
- latency target
- provider / endpoint availability
- historical evidence with confidence

一个 Task 可以使用多个模型，但必须区分：

- Model Switch：主模型失败或不可用，切换到兼容 fallback
- Model Collaboration：planner、implementer、reviewer、evaluator 等角色并行或串行协作

每个模型分支都有独立 TaskRun / Attempt、ResolvedPromptPack、ModelCallRecord、budget、evidence 和 verification。fan-in 只能消费有明确 provenance 的 artifact。

## 六、进度、问题、验证和完成度

进度不能只用百分比。建议组合：

~~~text
ProgressSnapshot:
  completed_dependencies
  total_dependencies
  completed_checkpoints
  total_checkpoints
  current_stage
  current_attempt
  verification_status
  evidence_completeness
  budget_used
  budget_remaining
  blocking_issue_count
  confidence
  updated_at
~~~

百分比仅在有明确分母时显示。探索型任务若没有稳定分母，应显示 stage、evidence completeness 和 blocking issues，不伪造 73% 之类的精确进度。

BlockingIssue 至少记录：

- issue_id
- type
- severity
- evidence
- affected_tasks
- owner
- next_action
- approval_required
- created_at
- resolved_at

## 七、CLI / TUI 展示

第一阶段优先 CLI 和 JSON；TUI 是同一 BoardProjection 的交互 adapter，未来 Web 仍复用相同查询模型。

建议的只读入口：

~~~text
steerbox portfolio list
steerbox project list
steerbox board show --portfolio <id>
steerbox board show --project <id>
steerbox task show <task_id>
steerbox run show <task_run_id>
steerbox run watch <task_run_id>
steerbox issues list
steerbox evidence show <evidence_id>
~~~

建议的受审计命令：

~~~text
steerbox task pause <task_id>
steerbox task resume <task_id>
steerbox task cancel <task_id>
steerbox task retry <task_id>
steerbox task reprioritize <task_id>
steerbox approval approve <approval_id>
steerbox checkpoint mark <task_run_id>
~~~

命令名是设计示例，最终 CLI 规格需另行收敛；这里锁定的是 query / command 分离和审计边界。

## 八、跨项目进化

跨项目进化必须使用显式 SystemEvolutionTask，而不是后台静默修改。

建议提升链：

~~~text
ToolUseErrorEvent
  -> LessonCandidate
  -> local verified recipe
  -> project proposal
  -> portfolio proposal
  -> global proposal
~~~

每次提升必须包含：

- scope
- positive evidence
- negative evidence
- source project / trace
- compatibility constraints
- affected models / prompts / tools / policies
- regression plan
- rollback plan
- approval level

跨项目共享的是经过治理的提案、评估结果和可移植 artifact，不是无边界复制某项目的原始记忆、secret 或私有上下文。

## 九、事件与投影

建议新增事件：

~~~text
portfolio_created
project_registered
task_created
task_dependency_added
task_ready
task_queued
task_run_started
task_run_heartbeat
task_run_waiting
task_run_verification_started
task_run_succeeded
task_run_failed
task_blocked
task_canceled
route_decision_recorded
prompt_pack_resolved
artifact_published
evidence_attached
join_decision_recorded
control_plane_command_requested
control_plane_command_decided
board_projection_updated
evolution_proposal_created
evolution_proposal_promoted
~~~

BoardProjection 可以重建：给定 EventLog 和 StateStore，应能重新计算同一任务状态、看板列、阻塞问题和验证摘要。BoardProjection 损坏不能破坏事实源。

## 十、分阶段边界

### Phase 1

- 保持 shared harness core + software development agent vertical slice
- 定义 Portfolio、Project、Task、TaskRun / Attempt、TaskDependency schema
- 定义 task state machine 和 BoardProjection schema
- 用单项目、本地顺序执行 fixture 证明 event -> projection
- 提供只读 CLI / JSON board fixture
- 不实现真实多项目并行调度、Web UI 或分布式 worker

### Phase 1.5

- 本地多项目 registry
- dependency-aware Scheduler
- 有上限的并行 worker
- workspace / worktree lease
- pause / resume / cancel / retry
- CLI / TUI board
- join / conflict / backpressure
- 多模型协作与 evaluator fan-in

### Phase 2

- 跨机器 worker 与持久任务队列
- Portfolio 级预算、优先级和容量规划
- Web control plane adapter
- 跨项目演化提案与回归平台
- OpenTelemetry / GenAI trace export
- A2A / MCP / external scheduler adapter

## 十一、验证标准

设计进入实现时至少验证：

1. Task 状态只能沿允许边迁移，非法迁移被拒绝并记录
2. DONE 中的 succeeded 具备 success criteria、verification 和 evidence
3. EventLog 能重建相同 BoardProjection
4. waiting_dependency 能在依赖满足后进入 ready
5. 并行写具有 workspace lease，冲突在 join 前被发现
6. retry、fallback、resume 和 fork 产生新 Attempt
7. cancel 能传播到未开始或可安全停止的子任务
8. ControlPlaneCommand 经过 authorization、PolicyGate 和 EventLog
9. 跨项目写入没有因 CrossProjectLink 自动获得授权
10. 进度百分比只在分母明确时出现
11. 多模型分支能反查 RouteDecision、ResolvedPromptPack、ModelCallRecord 和 evidence
12. evolution proposal 未经回归和审批不能提升 scope

## 十二、外部参考边界

可吸收的主流方向：

- OpenAI Agents SDK：handoff、session、run state、tracing
- LangGraph：durable execution、thread state、cross-thread store、多 agent workflow
- Google ADK：sequential、parallel、loop、graph workflow
- Temporal：durable workflow、task queue、worker routing、retry、backpressure
- A2A：task / context identity、artifact、状态、取消、订阅
- MCP Tasks：轮询、取消和延迟结果获取；当前仍应作为 adapter，不作为 steerBox 核心 Task 模型
- OpenTelemetry GenAI semantic conventions：model / agent / tool / workflow trace 对接

官方入口：

- OpenAI Agents SDK multi-agent：https://openai.github.io/openai-agents-python/multi_agent/
- OpenAI Agents SDK running / sessions / tracing：https://openai.github.io/openai-agents-python/running_agents/ 、https://openai.github.io/openai-agents-python/sessions/ 、https://openai.github.io/openai-agents-python/tracing/
- LangGraph persistence / durable execution / multi-agent：https://docs.langchain.com/oss/python/langgraph/persistence 、https://docs.langchain.com/oss/python/langgraph/durable-execution 、https://docs.langchain.com/oss/python/langchain/multi-agent
- Google ADK workflow agents：https://google.github.io/adk-docs/agents/workflow-agents/
- Temporal workflow execution / task queue：https://docs.temporal.io/workflow-execution 、https://docs.temporal.io/task-queue
- A2A specification / task lifecycle：https://a2a-protocol.org/latest/specification/ 、https://a2a-protocol.org/latest/topics/life-of-a-task/
- MCP Tasks（experimental）：https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/tasks
- OpenTelemetry GenAI / MCP semantic conventions：https://opentelemetry.io/docs/specs/semconv/gen-ai/ 、https://opentelemetry.io/docs/specs/semconv/gen-ai/mcp/
- AutoGen GraphFlow（experimental）：https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/graph-flow.html

这些资料提供模式和互操作参考，不替代 steerBox 的 GoalContract、PolicyGate、EventLog、StateStore、TaskRun、SideEffectLedger 和人类治理边界。

## 当前结论

steerBox 的看板应是可重建的执行投影，不是另一套事实数据库；多项目编排必须把 Task 与 TaskRun、Scheduler 与 Orchestrator、Supervisor 与 Control Plane 分开；并行执行必须有 workspace lease、冲突 / join、取消 / 重试和背压；done 必须有验证与 evidence；跨项目进化只能通过带来源、回归、回滚和审批的 SystemEvolutionTask 提升。
