# loops.elorm.xyz 示例学习要点

## 文档目的

本文件把当前已抓取的 loops.elorm.xyz 示例转成 `steerBox` 可学习、可吸收的中文要点。

本文件不是原站全文复制，也不代表已完整抓取 loops.elorm.xyz 的全部 40 个 loop。当前本地只整理了公开 browse 列表中稳定可访问的 9 个 loop。

原始本地索引见：`docs/loops/INDEX.md`。

## 总体观察

这些 loop 的共同价值不在于某个具体命令，而在于它们把 agent 任务写成了可反复执行的闭环：

```text
goal -> check command -> fix / act -> verify -> repeat -> exit condition
```

更重要的是，它们都强调 anti-gaming guardrails：

- 不允许修改检查命令来制造成功
- 不允许跳过、禁用、绕过检查
- 卡住时应报告 blocker，而不是篡改指标
- 测试相关 loop 不允许删除测试或把断言改成永远通过
- 优先修复真实代码，而不是为了绿灯篡改测试

这和 `steerBox` 的方向一致：agent 不是只要完成表面目标，而是必须在可验证、可审计、不可作弊的 harness 中闭环推进。

## 对 steerBox 的核心启发

### 1. Loop 必须有退出条件，但退出条件不等于轨迹正确

一个可用 loop 至少要有：

```text
objective
check_command
exit_condition
allowed_actions
forbidden_actions
failure_escalation
```

没有退出条件，agent 容易无限循环；退出条件不清楚，agent 容易用解释替代验证。

但 exit condition 只能证明“当前结果满足某个检查”，不能证明 step 1 到 step N 的执行路径没有问题。因此 `steerBox` 还需要 `LoopTrajectoryCheck`：审查过去 N 步是否存在目标漂移、scope creep、check gaming、错误假设延续或副作用链不清。

### 2. Guardrail 应该绑定到 loop，而不是只写在全局 prompt

这些示例的 guardrail 都贴近具体 loop：

- 测试 loop 防止删测试
- audit loop 防止改检查命令
- docs loop 防止只改文档不校验代码变化
- npm audit loop 防止 `npm audit fix --force`

对 `steerBox` 来说，`LoopGuardrail` 应该是 `GoalContract` 或 `LoopSpec` 的字段，而不是散落在系统提示词里。

### 3. Prompt-only 和 hook-based loop 应分开

loops.elorm.xyz 区分：

- prompt-only loop：只需要 kickoff prompt
- hook-based loop：需要安装文件，在特定事件后自动触发

对 `steerBox` 来说，这对应两类能力：

```text
ManualLoopSpec      -> 人或 agent 显式启动
EventTriggeredLoop  -> 文件编辑、commit、merge、检查失败等事件自动触发
```

### 4. Check 结果失败后不应立刻改策略

Guardrails Learning Loop 的思想是：同类失败连续出现后，才把教训写入 guardrail。

这和我们前面讨论的实时自进化一致，但它给了一个重要约束：

```text
single failure -> record event
repeated same failure -> memory candidate / guardrail candidate
validated repeated failure -> accepted guardrail / recipe
```

也就是：现场自进化可以很快，但不能每次失败都立刻改全局规则。

### 5. 事件触发 loop 是长期运行 agent 的关键能力

`Pre-Commit Guard`、`Post-Edit Test Guard`、`Post-Merge Regression Guard` 都是事件触发：

- 文件编辑后触发相关测试
- commit 前触发测试
- merge/rebase 后触发 smoke test

对 `steerBox` 来说，这意味着 LoopController 不应该只由用户指令驱动，还应支持：

```text
file_changed
command_finished
test_failed
git_commit_requested
git_merge_completed
checkpoint_created
policy_violation_detected
```

## 9 个 loop 的中文要点

### 1. Ship PR Until Green

目标：在分支上实现、测试、推送、开 PR、等待 CI，并循环修复直到检查通过。

可吸收点：

- 把开发任务从“写完代码”扩展到“CI 绿灯 + PR 可合并”
- 把远端 CI 状态纳入 verify 阶段
- 适合软件开发智能体主线

对 `steerBox` 的设计建议：

```text
LoopSpec:
  type: ci_until_green
  verify_sources:
    - local_tests
    - remote_ci
    - pr_status
  exit_condition: all_required_checks_green
```

风险控制：

- 不允许跳过 CI
- 不允许删除测试换绿灯
- 不允许把失败检查从 required checks 中移除

### 2. Flaky Test Triage

目标：重复运行失败测试，区分 flaky failure 和真实回归，只修复确认的真实回归。

可吸收点：

- 不把单次失败直接当成稳定事实
- 通过重复采样提高判断置信度
- 把 failure classification 写成 loop 阶段

对 `steerBox` 的设计建议：

```text
FailureClassifier:
  labels:
    - flaky
    - real_regression
    - environment_issue
    - unknown
```

风险控制：

- 不允许为了通过而削弱测试
- 不允许把真实回归误标成 flaky 后跳过
- 多次不一致结果应升级 evaluator 或人类审查

### 3. Pre-Commit Guard

目标：commit 前运行测试，阻止红灯代码进入提交。

可吸收点：

- 把质量门禁前移到提交前
- 事件触发优于事后补救
- 适合 `PolicyGate` 和 `EventTriggeredLoop`

对 `steerBox` 的设计建议：

```text
EventTriggeredLoop:
  trigger: git_commit_requested
  action: run_required_checks
  failure_action: block_commit
```

风险控制：

- 不允许 agent 加 `--no-verify` 绕过
- 不允许改 hook 让检查失效
- 可允许人类接管并显式 override，但必须审计

### 4. Docs Sync After Edits

目标：代码变更后找出受影响文档，并同步 README、API references、inline comments。

可吸收点：

- 文档同步不是最后阶段人工补写，而是代码变更后的自动 loop
- 需要 code-to-doc impact analysis
- 适合我们未来的 Code Logic Index Plugin

对 `steerBox` 的设计建议：

```text
DocImpactAnalyzer:
  inputs:
    - changed_files
    - changed_public_api
    - changed_config
    - changed_behavior
  outputs:
    - affected_docs
    - doc_update_plan
```

风险控制：

- 不允许只改 README 而不验证代码事实
- 不允许文档写超出当前实现的能力
- 文档应标注讨论稿、已实现、待实现的边界

### 5. Guardrails Learning Loop

目标：同类检查失败两次后，把 guardrail 追加到 `.ralph/guardrails.md`，避免后续重复犯错。

可吸收点：

- 这是现场自进化的最小可行形式
- 失败不是只用于本轮修复，也用于改进下一轮行为
- 和我们讨论的 tool recipe / memory candidate / guardrail candidate 高度一致

对 `steerBox` 的设计建议：

```text
GuardrailCandidate:
  trigger: repeated_same_failure
  evidence: failure_events
  proposed_rule: string
  scope: local_task | project | global
  status: candidate | accepted | rejected
```

风险控制：

- 本地 guardrail 可以快写
- 项目级 guardrail 需要验证
- 全局 guardrail 必须审批
- guardrail 不能覆盖原始目标或安全边界

### 6. npm Audit Fix Loop

目标：逐个修复 high / critical npm audit findings，并用测试验证，不盲目 `npm audit fix --force`。

可吸收点：

- 安全修复要逐项处理，不应一键大改依赖
- 每个漏洞修复后要验证行为没有破坏
- 适合安全智能体的 defensive remediation loop

对 `steerBox` 的设计建议：

```text
SecurityRemediationLoop:
  select_one_finding
  propose_minimal_fix
  apply_fix
  run_tests
  verify_finding_resolved
  record_side_effects
```

风险控制：

- 不允许无审查大版本升级
- 不允许隐藏 audit 输出
- 不允许只看 audit 绿灯而忽略测试失败

### 7. Post-Merge Regression Guard

目标：merge 或 rebase 后运行 smoke tests，尽早发现集成回归。

可吸收点：

- 回归常发生在集成点，不一定发生在单分支本地测试
- merge/rebase 是高价值事件触发点
- 适合 `RecoveryCheckpoint` 和 `DriftGuard` 联动

对 `steerBox` 的设计建议：

```text
EventTriggeredLoop:
  trigger: git_merge_completed | git_rebase_completed
  precondition: create_or_confirm_checkpoint
  action: run_smoke_tests
  failure_action: stop_and_report_regression
```

风险控制：

- merge 前后应有 checkpoint / diff boundary
- 失败后不要盲目回滚，先定位是集成冲突、环境问题还是真实回归

### 8. Post-Edit Test Guard

目标：文件编辑后运行相关测试，尽早捕获局部回归。

可吸收点：

- 每次编辑后不一定跑全量测试，但应跑相关测试
- 需要 changed-file-to-test mapping
- 适合长期自动化开发中的快速反馈

对 `steerBox` 的设计建议：

```text
RelatedTestSelector:
  inputs:
    - changed_files
    - code_index
    - test_history
  outputs:
    - minimal_related_tests
    - confidence
```

风险控制：

- 相关测试选择置信度低时，应升级到更大范围测试
- 连续局部通过但最终失败，应更新 test selection memory

### 9. A11y Audit Until Clean

目标：对变更路由运行自动无障碍检查，修复 violations，并重复直到 audit clean。

可吸收点：

- 质量类 loop 可以和开发 loop 并行或作为后置 gate
- audit clean 是清晰 exit condition
- 适合 evaluator / check command / evidence 结合

对 `steerBox` 的设计建议：

```text
QualityAuditLoop:
  target_scope: changed_routes
  check: accessibility_audit
  exit_condition: zero_blocking_violations
```

风险控制：

- 不允许关闭 a11y 检查
- 不允许隐藏 violations
- 无法自动修复时应报告 blocker 和证据

## 可抽象出的 steerBox 对象

### LoopSpec

```text
loop_id
title
type
trigger_mode
objective
check_command
exit_condition
allowed_actions
forbidden_actions
guardrails
failure_escalation
verification_policy
human_steering_mode
```

### EventTriggeredLoop

```text
trigger_event
preconditions
actions
failure_action
checkpoint_policy
policy_gate
```

### LoopGuardrail

```text
guardrail_id
scope
rule
reason
evidence
severity
created_from_loop
status
```

### FailurePattern

```text
failure_pattern_id
source_loop
failure_signature
repeat_count
classification
suggested_guardrail
suggested_recipe
confidence
```

### LoopRunRecord

```text
loop_run_id
loop_id
goal_id
started_at
ended_at
iterations
checks_run
failures
fixes_applied
exit_status
blockers
```

## 对当前文档体系的影响

这些 loop 示例应连接到现有文档：

- `GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md`: LoopController 与 exit condition
- `GOAL_ALIGNMENT_CHECKS.md`: 检查 loop 是否仍服务原始 goal
- `LOOP_TRAJECTORY_CHECKS.md`: 检查 step 1 到 step N 的执行轨迹是否存在累积偏差或投机取巧
- `HUMAN_STEERING_MODES.md`: 卡住或高风险时切换到审批/接管
- `MEMORY_STORAGE_AND_CONSOLIDATION.md`: 失败模式、guardrail candidate、tool recipe candidate 的沉淀
- `DRIFT_GUARD_DISCUSSION.md`: 防止为了通过检查而破坏项目
- `SIDE_EFFECT_LEDGER_DISCUSSION.md`: 安全修复、依赖升级、外部动作的副作用记录

## 第一阶段建议

第一阶段不需要实现完整 hook 系统，但应先实现文档级 schema：

```text
docs/loops/
  INDEX.md
  LEARNING_NOTES_CN.md
  *.md
```

未来实现时，可以优先支持：

1. prompt-only loop
2. manual check command loop
3. file-change triggered loop
4. git-command triggered loop
5. CI-status triggered loop
6. repeated-failure guardrail learning loop

## 当前结论

loops.elorm.xyz 的示例最值得吸收的不是具体工具，而是它的 loop 结构：

```text
目标明确
检查命令明确
退出条件明确
禁止作弊明确
失败升级明确
```

`steerBox` 后续应把这些思想工程化为 `LoopSpec`、`LoopGuardrail`、`EventTriggeredLoop`、`FailurePattern` 和 `LoopRunRecord`，让 agent 在长程开发和安全任务中能稳定闭环，而不是靠一次性 prompt 自由发挥。
