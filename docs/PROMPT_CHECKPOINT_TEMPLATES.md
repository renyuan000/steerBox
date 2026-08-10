# Prompt, Checkpoint, and DriftGuard Templates

## 文档目的

本文件用于把 `MODEL_PROMPT_AND_CHECKPOINT_POLICY.md` 中的概念推进到更接近实现的模板层。

本文件是讨论稿，不是最终实现规格，也不代表当前仓库已经具备对应能力。

## 设计原则

模板的目标不是制造更多 prompt 文件，而是让 harness 能够按模型、任务、风险和运行模式组合出可追踪、可验证、可进化的执行配置。

核心对象：

- `ProviderProfile`
- `EndpointProfile`
- `ModelProfile`
- `PromptProfile`
- `AgentRolePrompt`
- `TaskPromptPack`
- `ResolvedPromptPack`
- `RoutePolicy`
- `RouteDecision`
- `ModelCallRecord`
- `ModelEvaluation`
- `CheckpointPolicy`
- `DriftGuard`

## 一、Provider、Endpoint 与 Model 模板

### ProviderProfile

~~~yaml
provider_id: ""
provider_name: ""
protocol_type: "openai_compatible | anthropic | local | internal | other"
region: ""
privacy_policy: "public | internal | confidential | restricted"
supported_features:
  - "streaming"
  - "tools"
  - "structured_output"
rate_limit: null
availability_policy: ""
status: "active | degraded | disabled"
auth_ref: "external-secret-reference-only"
~~~

auth_ref 只允许保存外部 secret 引用；禁止在本模板、registry、trace、event log 或 checkpoint 中保存 API key、token 或认证响应。

### EndpointProfile

~~~yaml
endpoint_id: ""
provider_id: ""
model_id: ""
deployment_name: ""
region: ""
context_window: 0
cost_profile: {}
latency_profile: {}
availability_policy: ""
capabilities_override: {}
privacy_class: "public | internal | confidential | restricted"
status: "active | degraded | disabled"
~~~

### ModelProfile

```yaml
model_id: "model-family/model-name"
model_family: "coding | reasoning | fast | local | evaluator | security"
version: ""
status: "draft | active | deprecated"
provider_ids: []
endpoint_ids: []

capabilities:
  context_window: 0
  supports_tool_calling: false
  supports_structured_output: false
  supports_streaming: false
  supports_vision: false
  supports_long_context: false
  supports_reasoning: false
  supports_background_run: false
  supports_mcp: false
  supports_batch: false

cost_profile:
  input_token_cost: null
  output_token_cost: null
  expected_latency_ms: null
  rate_limit: null

strengths:
  - ""

weaknesses:
  - ""

quality_profile:
  instruction_following_score: null
  tool_calling_quality: null
  coding_quality: null
  refactor_quality: null
  security_analysis_quality: null
  long_context_reliability: null
  hallucination_risk: null

dynamic_performance:
  historical_success_rate: null
  tool_call_success_rate: null
  verification_pass_rate: null
  average_latency_ms: null
  p95_latency_ms: null
  average_cost: null
  fallback_success_rate: null
  sample_count: 0
  measurement_window: ""
  source_trace_ids: []
  confidence: "low | medium | high"
  last_verified_at: ""

preferred_roles:
  - "planner"
  - "executor"
  - "evaluator"
  - "summarizer"
  - "security_analyst"

risk_limits:
  max_task_risk: "low | medium | high | critical"
  allowed_autonomy_modes:
    - "autonomous_loop"
    - "human_steerable_loop"
    - "human_in_the_loop"
    - "manual_takeover"

prompt_binding:
  preferred_prompt_profile: ""
  compatible_prompt_profiles: []
  forbidden_prompt_profiles: []

fallback:
  fallback_models:
    - ""
  fallback_conditions:
    - "unavailable"
    - "rate_limited"
    - "low_confidence"
    - "policy_blocked"

known_failure_patterns:
  - pattern: ""
    mitigation: ""
    confidence: "low | medium | high"

verification:
  last_verified_at: ""
  verification_dataset: ""
  notes: ""
```

## 二、PromptProfile 模板

```yaml
prompt_profile_id: ""
version: ""
status: "draft | active | deprecated"
compatible_provider_types: []
compatible_model_families: []
required_capabilities: []

intended_roles:
  - "planner"
  - "executor"
  - "evaluator"
  - "summarizer"

instruction_style:
  verbosity: "low | medium | high"
  explicitness: "low | medium | high"
  autonomy_level: "low | medium | high"
  ask_before_uncertain_action: true
  require_evidence_before_claim: true
  require_stepwise_execution: true

context_style:
  max_context_items: 0
  prefer_structured_context: true
  require_source_paths: true
  avoid_large_raw_logs: true
  summarize_before_use: false

tool_calling_style:
  require_help_before_unknown_tool: true
  require_dry_run_when_available: true
  require_minimal_verification_call: true
  max_retry_count: 1
  on_tool_error: "record_lesson_candidate"

output_contract:
  require_goal_reference: true
  require_evidence_summary: true
  require_verification_status: true
  require_risk_boundary: true
  forbid_unverified_completion_claim: true

risk_limits:
  can_modify_files: true
  can_run_commands: true
  can_run_network_tools: false
  can_modify_policy: false
  can_modify_global_rules: false
  can_execute_high_impact_action: false

checkpoint_policy_override:
  require_checkpoint_before_file_write: true
  require_checkpoint_before_bulk_change: true
  require_checkpoint_before_external_side_effect: true

known_failure_patterns:
  - pattern: "over-refactor"
    mitigation: "force scope check before edits"
  - pattern: "tool schema guessing"
    mitigation: "require help/docs or verified recipe first"

verification:
  evaluation_dataset: ""
  success_metrics:
    - "verification_pass_rate"
    - "failed_tool_call_rate"
    - "unintended_file_change_count"
  last_verified_at: ""
```

## 三、AgentRolePrompt 模板

~~~yaml
role_prompt_id: ""
role: "planner | implementer | reviewer | tester | evaluator | summarizer | researcher | security_analyzer | coordinator | human_interaction"
version: ""
status: "draft | active | deprecated"
system_prompt_fragments:
  - id: "role_specific_rules"
    required: true
tool_policy: {}
output_contract: {}
checkpoint_policy: {}
evaluator_policy: {}
compatible_model_families: []
~~~

## 四、TaskPromptPack 模板

```yaml
task_prompt_pack_id: ""
task_type: "software_dev | security_ops | security_review | vuln_research | documentation | refactor | testing"
version: ""
status: "draft | active | deprecated"

provider_adapter_prompt: ""
model_prompt_profile: ""
role_prompt_id: ""

system_prompt_fragments:
  - id: "base_harness_rules"
    required: true
  - id: "task_specific_rules"
    required: true

user_prompt_template: |
  Goal: {{ objective }}
  Scope: {{ scope }}
  Success Criteria: {{ success_criteria }}
  Constraints: {{ constraints }}
  Risk Level: {{ risk_level }}

required_context_blocks:
  - "GoalContract"
  - "RelevantFiles"
  - "ToolUsageRecipes"
  - "CheckpointPolicy"
  - "DriftGuardRules"

forbidden_context_blocks:
  - "unrelated_large_logs"
  - "stale_unverified_memory"

preflight_checklist:
  - "Read GoalContract"
  - "Confirm scope and out_of_scope"
  - "Check allowed tools"
  - "Check model role and risk limit"
  - "Check existing checkpoint"
  - "Check known tool recipes before unfamiliar tool use"

execution_checklist:
  - "Use minimal necessary context"
  - "Make smallest safe change"
  - "Record tool errors"
  - "Stop if scope expands"
  - "Create checkpoint before high-risk action"

verification_checklist:
  - "Run specified verification plan"
  - "Check diff against scope"
  - "Check unintended files"
  - "Check quality bar"
  - "Record evidence"

stop_conditions:
  - "Goal achieved and verified"
  - "Scope expansion detected"
  - "Policy blocked action"
  - "Repeated tool failure"
  - "Risk exceeds model limit"
  - "Human approval required"

handoff_template: |
  Current goal: {{ goal_id }}
  Completed: {{ completed_items }}
  Pending: {{ pending_items }}
  Verification: {{ verification_status }}
  Risks: {{ risks }}
  Next action: {{ next_action }}

failure_response_template: |
  Failure type: {{ failure_type }}
  Evidence: {{ evidence }}
  Retry decision: {{ retry_decision }}
  Lesson candidate: {{ lesson_candidate }}
```

## 五、ResolvedPromptPack 模板

~~~yaml
prompt_pack_id: ""
base_prompt_version: ""
provider_prompt_version: ""
model_prompt_version: ""
role_prompt_version: ""
task_prompt_version: ""
policy_summary_hash: ""
tool_summary_hash: ""
context_package_hash: ""
resolution_reason: ""
created_at: ""
~~~

## 六、RoutePolicy、RouteDecision 与 ModelCallRecord 模板

~~~yaml
route_policy_id: ""
version: ""
candidate_constraints: {}
selection_weights:
  quality: 0
  cost: 0
  latency: 0
  reliability: 0
cost_budget: null
latency_target_ms: null
quality_requirement: "low | medium | high | critical"
privacy_requirement: "public | internal | confidential | restricted"
risk_rules: {}
fallback_rules: {}
collaboration_rules: {}
approval_rules: {}
status: "draft | active | deprecated"
~~~

~~~yaml
route_decision_id: ""
task_id: ""
task_type: ""
agent_role: ""
candidate_models: []
selected_provider: ""
selected_model: ""
selected_endpoint: ""
selection_reason: ""
rejected_candidates: []
cost_estimate: null
latency_estimate_ms: null
quality_requirement: ""
risk_level: ""
privacy_requirement: ""
context_requirement: {}
tool_requirement: {}
fallback_chain: []
approval_required: false
prompt_profile_id: ""
prompt_profile_version: ""
created_at: ""
~~~

~~~yaml
model_call_id: ""
task_run_id: ""
provider_id: ""
endpoint_id: ""
model_id: ""
route_decision_id: ""
resolved_prompt_pack_id: ""
input_context_hash: ""
output_artifact_ref: ""
latency_ms: null
input_tokens: null
output_tokens: null
estimated_cost: null
finish_reason: ""
tool_call_count: 0
verification_status: "not_run | passed | failed | blocked"
error_class: ""
created_at: ""
~~~

### ModelEvaluation

~~~yaml
evaluation_id: ""
model_id: ""
prompt_profile_id: ""
task_type: ""
sample_count: 0
measurement_window: ""
source_trace_ids: []
metrics: {}
confidence: "low | medium | high"
regression_status: "not_run | passed | failed | blocked"
promotion_status: "not_eligible | proposed | approved | rejected"
created_at: ""
~~~

## 七、CheckpointPolicy 模板

```yaml
checkpoint_policy_id: ""
version: ""
status: "draft | active | deprecated"

fixed_checkpoints:
  enabled: true
  every_loop_iterations: 5
  every_minutes: 10
  every_tool_calls: 20
  every_file_changes: 5

event_triggered_checkpoints:
  before_file_write: true
  before_bulk_change: true
  before_delete: true
  before_public_api_change: true
  before_config_change: true
  before_policy_change: true
  before_skill_or_tool_update: true
  before_external_side_effect: true
  after_evaluator_pass: true
  after_goal_update: true

risk_triggered_checkpoints:
  when_diff_size_exceeds_lines: 300
  when_modified_files_exceed: 5
  when_core_files_touched: true
  when_security_boundary_touched: true
  when_loop_not_converged_iterations: 8
  when_evaluator_failed_times: 2
  when_scope_expansion_detected: true

operator_marked_checkpoint:
  enabled: true
  require_label: true
  require_reason: true

checkpoint_types:
  soft_checkpoint:
    includes:
      - "GoalContract"
      - "LoopState"
      - "EventCursor"
      - "MemoryCursor"
      - "TraceCursor"
  hard_checkpoint:
    includes:
      - "soft_checkpoint"
      - "workspace_snapshot"
      - "side_effect_ledger_cursor"
      - "tool_state_summary"

restore_policy:
  require_restore_precheck: true
  require_side_effect_reconciliation: true
  forbid_auto_replay_irreversible_side_effects: true
  allow_fork_from_checkpoint: true
  require_human_approval_for_high_risk_restore: true

retention_policy:
  keep_operator_marked: true
  keep_last_n_hard: 10
  keep_last_n_soft: 50
  compact_old_soft_checkpoints: true
```

## 八、DriftGuard 模板

```yaml
drift_guard_id: ""
version: ""
status: "draft | active | deprecated"

scope_rules:
  require_goal_scope_match: true
  forbid_out_of_scope_files: true
  require_reason_for_scope_expansion: true
  require_human_approval_for_scope_expansion: true

stable_artifacts:
  protected_paths:
    - ""
  protected_patterns:
    - ""
  require_checkpoint_before_touch: true
  require_evaluator_after_touch: true

architecture_invariants:
  - id: ""
    description: ""
    check_method: "manual | evaluator | script | test"
    blocking: true

behavior_change_policy:
  require_behavior_change_declaration: true
  require_test_update_for_behavior_change: true
  forbid_silent_behavior_change: true

security_invariants:
  forbid_permission_weakening: true
  forbid_audit_removal: true
  forbid_policy_bypass: true
  require_security_evaluator_for_security_boundary: true

diff_limits:
  warn_lines_changed: 200
  block_lines_changed: 800
  warn_files_changed: 5
  block_files_changed: 15

over_refactor_detection:
  detect_large_rename: true
  detect_style_only_churn: true
  detect_unrequested_rearchitecture: true
  detect_generic_pattern_replacement: true

loop_drift_detection:
  max_iterations_without_verification: 4
  max_repeated_failed_tool_calls: 2
  require_stop_on_goal_uncertainty: true
  require_human_steering_on_repeated_scope_questions: true

response_when_triggered:
  create_checkpoint: true
  switch_to_read_only: true
  run_evaluator: true
  ask_human: true
  block_high_risk_action: true
```

## 九、固定检查项清单

每轮 loop 都应检查：

- 当前动作是否服务 `GoalContract.objective`
- 当前动作是否在 `scope` 内
- 当前动作是否触碰 `out_of_scope`
- 当前模型是否允许执行该风险等级任务
- 是否需要更换模型或启用 evaluator
- 是否使用了正确的 prompt profile
- 是否需要 checkpoint
- 是否已有未处理 tool error lesson candidate
- 是否有足够验证证据
- 是否需要人工介入

## 十、事件触发检查项清单

发生以下事件时触发额外检查：

- 文件写入
- 删除文件或代码块
- 批量修改
- 公共接口变化
- 配置变化
- 安全策略变化
- 权限变化
- 外部系统调用
- MCP server 接入或权限变化
- skill / tool / policy / memory 更新
- model routing 降级
- evaluator 失败
- loop 多轮无验证
- agent 尝试扩大任务目标

## 十一、Prompt 组合记录模板

每次执行应记录 prompt 组合：

```yaml
prompt_assembly_id: ""
goal_id: ""
loop_id: ""
provider_id: ""
endpoint_id: ""
model_id: ""
agent_role: ""
route_decision_id: ""
prompt_profile_id: ""
task_prompt_pack_id: ""
resolved_prompt_pack_id: ""
fragments:
  - id: ""
    version: ""
    reason: ""
context_blocks:
  - id: ""
    source: ""
    token_estimate: 0
policy_summary_id: ""
checkpoint_policy_id: ""
drift_guard_id: ""
assembled_at: ""
effectiveness_metrics:
  tokens_used: null
  verification_result: null
  tool_error_count: null
  scope_violation: null
```

## 十二、第一阶段建议

第一阶段建议先落地 schema、静态 manifest、规则式 prompt resolution / routing 和一个真实 model adapter，不急着实现自适应路由或动态 prompt 进化。

建议最小文件：

```text
configs/providers/*.yaml
configs/endpoints/*.yaml
configs/models/*.yaml
configs/prompts/profiles/*.yaml
configs/prompts/roles/*.yaml
configs/prompts/task_packs/*.yaml
configs/routing/*.yaml
configs/policies/checkpoint/*.yaml
configs/policies/drift_guard/*.yaml
```

第一阶段最小验证：

- 能为一个软件开发任务选出 model + prompt profile + task prompt pack
- 能生成 RouteDecision、ResolvedPromptPack 和 ModelCallRecord
- 能让低风险只读任务与高风险复杂任务走不同的规则式路由
- 能在 fallback 时创建新 attempt、重新解析 prompt 并重新验证
- registry、prompt、trace、event log 和 checkpoint 中只有 auth_ref，没有认证明文
- 能在写文件前触发 checkpoint decision
- 能在大 diff 或越 scope 时触发 drift guard
- 能记录 prompt assembly trace

## 当前结论

这套模板的目标是把“多模型高遵循执行”和“自主 loop 不跑偏”做成工程对象，而不是靠人工记忆。

后续进入代码实现时，应优先实现 schema、静态 registry、trace 记录和检查点决策，再实现自动 prompt profile 进化。
