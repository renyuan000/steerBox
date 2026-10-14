# Phase 1 Validation Plan

## 文档目的

本文件定义第一阶段如何验证。

第一阶段不以“功能很多”为成功标准，而以真实闭环、可审计、可恢复边界清晰为成功标准。

## 验证原则

- 所有完成结论必须有证据
- 所有工具调用必须可追踪
- 所有文件修改必须有 diff boundary
- 所有高风险动作必须有 policy decision
- checkpoint 只能承诺已记录范围
- side effect 必须能区分可恢复和不可恢复
- 模型路由、prompt 解析、fallback 和调用结果必须可追踪
- registry、prompt、trace、event log 和 checkpoint 不得包含 API key 或 token 明文
- 占位能力不能被写成已实现能力

## 验证环境

第一阶段验证建议使用本地仓库和本地运行数据目录。

推荐数据目录：

```text
/temp/steerbox/<project_ref>/.steerbox/
```

该目录应作为运行产物处理，不应默认提交；项目目录内的 `target/` 和 `.steerbox/` 仍需由 `.gitignore` 兜底排除。

WSL2 重启恢复检查必须先执行 `git status --short --branch`、`git log -1 --oneline --decorate` 和运行数据目录存在性检查，再继续 loop；不能把文档状态或内存状态当作 runtime 已恢复的证据。

## 最小端到端验证任务

建议使用一个低风险真实任务：

```text
修改一个 Markdown 文档中的小段内容，运行 Markdown 链接检查，生成完整 run summary。
```

选择原因：

- 文件修改真实
- 验证命令明确
- 副作用主要是本地可逆修改
- 适合测试 event、trace、checkpoint、drift guard

## 当前验证状态

本表只记录当前仓库已取得的代码/测试证据；“未验证”不等于设计无效，也不等于实现完成。

| 用例 | 当前状态 | 当前证据 |
|---|---|---|
| V0 Core Contract | 部分验证 | domain/EventEnvelope 序列化与敏感字段测试；完整 V0 仍待补齐 |
| V1 GoalContract | 部分验证 | `tests/domain_contracts.rs` |
| V2 LoopRun | 部分验证 | `tests/loop_controller.rs` |
| V3 ToolRegistry/PolicyGate | 已验证切片 | `tests/tool_policy.rs`、`tests/loop_controller.rs` |
| V4 EventLog | 已验证切片 | EventLog append/reopen 测试 |
| V5 Trace 可读性 | 已验证切片 | `tests/loop_controller.rs`、`tests/runtime_cli.rs`；完整回放工具仍未实现 |
| V6 文件修改与 diff boundary | 已验证切片 | `tests/dev_vertical_slice.rs`、`tests/runtime_cli.rs`、`tests/workspace_edit_audit_gates.rs`；bounded Markdown controller 缺 checkpoint/ledger、pending pause/cancel、终态及前置审计故障均不修改原始字节；不保证后置故障自动 rollback |
| V7 RecoveryCheckpoint metadata | 已验证写入与只读解释切片 | `tests/audit_store.rs`、`tests/dev_vertical_slice.rs`；覆盖 loop/step 最新追加查询、hash 格式/歧义、未知恢复状态、no-write 矛盾、日志错误行号及查询不修改文件；不重算 workspace hash、不验证快照、不执行 restore/replay |
| V8 SideEffectLedger | 已验证记录与基础派生解释切片 | `tests/loop_controller.rs`、`tests/dev_vertical_slice.rs`、`tests/audit_store.rs`、`tests/workspace_edit_audit_gates.rs`；ledger 缺失或 mutation 前 append 故障阻止 controller Markdown 写入。disposition/status 是 metadata 派生解释，不改变 JSONL schema、不构成执行授权或完整恢复/补偿状态机 |
| V9 GoalAlignmentCheck | 已验证切片 | `tests/drift_guard.rs`；结果写入完整 loop 仍待接线 |
| V10 DriftGuard | 已验证切片 | `tests/drift_guard.rs` |
| V11 Human Steering Mode | 已验证最小运行时切片 | `tests/user_command_inbox.rs`、`tests/loop_controller.rs`、`tests/state_store.rs`；已覆盖 Inbox 幂等、pause/resume/cancel 安全边界和状态重建，takeover、后台 worker、控制面仍未实现 |
| V12 Baseline Tool Recipes | 部分验证 | registry/recipe 边界已有；`tool_call_failed` 已记录 tool_call_id、failure category 和最小 LessonCandidate，recipe candidate、shell quoting 和 link-check recipe 仍未实现 |
| V13 Model Routing | 已验证本地路由、冻结请求、non-stream adapter 与库审计接线切片 | `tests/model_registry.rs`、`tests/model_router.rs`、`tests/model_role_fixtures.rs`、`tests/loop_model_routing.rs`、`tests/model_call_controller_audit.rs`、`tests/model_call_store.rs`、`tests/model_contracts.rs`、`tests/resolved_prompt.rs`、provider/Anthropic 与 localhost HTTP transport 测试；覆盖 binding/approval fail-closed、wire body hash、成功 resolved 调用的 usage/finish_reason 与成功 store、resolved adapter-error 的独立 `ModelCallFailureRecord`/failure JSONL、未知 response facts 为 None、原始 provider error 不落 failure 日志及失败审计错误；routed fixture fallback 有新 Attempt/真实事件/总预算和失败终态。不覆盖所有失败入口、live provider、resolved-provider fallback 或跨进程恢复；fixture 数值不是实测能力/成本/性能，脱敏不是通用 DLP |
| V18 StreamReporter | 已验证 CLI 接入切片 | `tests/stream_reporter.rs`、`tests/runtime_cli.rs`；支持 lifecycle/progress/disable/writer error/redaction，readonly/fixture 生命周期输出与事件和状态一致；ContinueObservation/RequestHuman 不伪报 loop completed；尚未接入 LoopController 内部工具/模型 streaming |
| V19 Execution History Query | 已验证最小只读切片 | `tests/execution_query.rs`、`tests/runtime_cli.rs`；EventLog 事实源、StateStore 当前状态投影、metadata-only `execution list/show/page` 和 detached snapshot 分页；覆盖非法参数、空页、越界和未知 loop；底层仍依赖 `read_all()`，live worker/control plane 未实现 |
| V14 Run Summary | 已验证最小切片 | `tests/audit_store.rs`、`tests/runtime_cli.rs`；完整 task/attempt/evidence 反查仍待实现 |
| V15 Task 状态迁移 | 已验证最小切片 | `tests/domain_contracts.rs`；TaskRun/Attempt 命令 lineage 仍待实现 |
| V16 BoardProjection | 已验证最小切片 | `tests/board_projection.rs`、`tests/runtime_cli.rs`；依赖和完整 Attempt 投影仍待实现 |
| V17 Command 幂等与版本冲突 | 已验证最小切片 | `tests/user_command_inbox.rs` 验证 command_id 幂等、冲突拒绝和状态去重；expected_object_version 冲突处理仍未实现 |
| V20 Execution Strategy | 已验证最小库接线切片 | `tests/strategy_selector.rs`、`tests/budget_calculator.rs`、`tests/loop_strategy.rs`、`tests/loop_strategy_controls.rs`、`tests/strategy_audit_failure.rs`；纯选择/输入拒绝、审批前零动作、授权 clamp、同实例计数/重复历史保留、工具额度、pause/resume不解除审批及主审计失败不改变策略/预算/revision；补充审批事件写入失败尚无独立注入测试；验证频率仅需求元数据，自定义配置、审批解析、协作执行及策略恢复尚未实现 |
| V21 Memory candidate metadata | 已验证领域与显式 controller 检索切片 | `tests/memory.rs`、`tests/loop_controller.rs`：schema/scope/provenance、摘要脱敏、稳定排序和 lifecycle 过滤；controller 校验当前 project 与指定 goal/task，记录 requested/filtered/used，Paused/terminal 或审计错误不返回候选。used=returned_metadata_only，不表示模型使用；事件不含摘要，返回值仍是候选 metadata。来源校验仅检查已授权输入集合的引用身份，不自动执行；不含 PolicyGate、prompt 注入或 promotion |
| V24 Memory candidate persistence | 已验证 JSONL 与 catalog 加载切片 | `tests/memory_store.rs`：round-trip、缺失 JSONL 不创建该日志（可创建锁文件/父目录）、锁内重复 ID 拒绝、物理行 JSON/字段错误不回显摘要；`read_catalog()` 复用 insert，稳定过滤/排序并保留 lifecycle。当前整份加载 catalog，不是分页/向量检索；不自动解析 source events、promotion 或授权 |
| V25 Goal/Plan revision boundary | 已验证最小领域契约切片 | `tests/domain_contracts.rs`：schema/unknown-field 拒绝、revision/parent/goal_revision 边界、supersede 后旧对象失效、plan 仅对 active 且 goal revision 精确匹配有效；不接后台 takeover、自动重规划或审批恢复 |
| V22 本地行为 acceptance | 已验证 smoke 与回归切片 | `tests/acceptance/` 的四个 Cargo target，共 23 项；使用生产 Redactor、精确 policy decision/reason、正确 loop/task 身份和 pause/resume/cancel 状态/预算/事件断言，takeover 明确 Rejected；同一受限 Markdown 写在交互模式需审批、自主模式可 Allow。进度延迟仅为本地 controller/in-memory sink smoke，不是 TUI/Web、模型首字节性能、在途抢占、完整接管或通用 DLP 验收 |
| V23 Routed 故障封口 | 已验证当前进程内切片 | `tests/loop_model_routing.rs`：fallback/route/loop-failed 事件写入失败及多阶段 ProgressSink 故障后不再调用模型、不伪造 evidence refs、不误覆盖 completed/cancelled/paused；审批拒绝不当成失败重试。不保证失败终态持久化，不扩展 non-routed/resolved 或跨进程恢复 |

状态更新规则：只有可复现命令、测试输出或运行日志支持时，才能把“未验证”改为“已验证”。

## 验证用例

### V0. Phase 1 Core Contract

步骤：

1. 读取 PHASE_1_CONTRACTS.md
2. 序列化一个核心对象和一个 EventEnvelope
3. 反序列化并检查 schema version、ID、关联字段和敏感数据边界

期望结果：

- 核心对象字段与契约一致
- EventEnvelope 能通过 sequence_no、correlation_id 和 causation_id 关联执行链
- fixture 不包含 API key、token 或认证响应

### V1. GoalContract 创建

步骤：

1. 创建一个开发任务 goal
2. 设置 objective、scope、out_of_scope、success_criteria、verification_plan
3. 写入 state 和 event

期望结果：

- event log 中存在 `goal_created`
- state 中存在当前 goal
- trace 中有人类可读摘要

失败条件：

- goal 只存在内存
- success criteria 缺失
- scope / out_of_scope 缺失

### V2. LoopRun 启动

步骤：

1. 基于 goal 启动 loop run
2. 创建 step 1
3. 记录 loop state

期望结果：

- event log 中存在 `loop_started` 和 `step_started`
- state 中 current step 可查询
- trace 中能看到 loop_run_id 和 step_id

### V2.1 Task / TaskRun / Attempt 身份

步骤：

1. 为一个 Task 创建 initial TaskRun 和 Attempt
2. 分别模拟 retry、fallback、resume 和 fork
3. 检查每次新 Attempt 的 attempt_no、reason、parent_attempt_id 和关联记录
4. 检查原失败 Attempt 的 verification、artifact 和 evidence 没有被覆盖

期望结果：

- retry、fallback、resume 和 fork 都创建新的 Attempt
- 每个 Attempt 只结束一次，且结束后不重新变为 running
- parent_attempt_id 能还原 Attempt lineage，失败证据保持可反查

失败条件：

- 新尝试复用旧 attempt_id
- fallback 或 resume 静默覆盖原失败记录
- 没有 reason 或 parent_attempt_id，无法解释尝试关系

### V3. ToolRegistry 与 PolicyGate

步骤：

1. 调用已注册只读工具
2. 调用未注册工具
3. 调用需要审批的写入工具

期望结果：

- 已注册只读工具允许
- 未注册工具阻断
- 高风险工具要求 approval 或 checkpoint
- 每次决策写入 `policy_decision`

### V4. EventLog 持久性

步骤：

1. 写入多类事件
2. 停止 runtime
3. 重新启动 runtime
4. 查询历史事件

期望结果：

- 历史事件仍可查询
- event 顺序稳定
- state 能恢复当前 loop 位置

### V5. Trace 可读性

步骤：

1. 执行一次 loop
2. 打开对应 JSONL trace
3. 用 `rg` 搜索 goal_id / step_id / tool_call_id

期望结果：

- trace 可人工阅读
- trace 能定位关键事件
- EventLog 是执行事实源，trace 只作可读诊断和定位，不替代 EventLog

### V6. 文件修改与 diff boundary

前置门禁验收：在 policy Allow 与显式 approval 两条路径分别缺少 checkpoint、ledger 或两者；提交 pending pause/cancel 或使 loop completed/failed；使 policy EventLog、checkpoint、trace、ledger 的 mutation 前写入失败。目标原始字节必须保持，不能产生 `file_change_recorded`/`model_action_verified`。trace/ledger 故障可留下前置 started 事件，不承诺审计原子性。首次 policy append 故障已有测试，第二次 action_started append 尚无独立故障注入。

Focused 命令：`CARGO_TARGET_DIR=/tmp/steerbox-phase1-target CARGO_BUILD_JOBS=2 cargo test --locked --offline --test workspace_edit_audit_gates --test loop_controller --test loop_approval`。

步骤：

1. 修改一个低风险文件
2. 记录修改前后摘要
3. 生成 diff boundary

期望结果：

- event log 中存在 `file_change_recorded`
- checkpoint metadata 关联 diff
- run summary 能说明修改了什么

失败条件：

- 文件已改但没有记录
- 只能看到最终状态，看不到修改边界

### V7. RecoveryCheckpoint metadata

步骤：

1. 在文件修改前创建 checkpoint metadata
2. 执行修改
3. 在修改后记录 checkpoint / diff 关系
4. 使用 `CheckpointStore::latest_for` / `latest_for_loop_step` 查询最后追加的匹配记录，检查只读恢复建议
5. 构造非法/歧义 hash、未知 changed-hash 恢复状态、no-write 与 files 矛盾及不匹配记录的 JSON 错误

期望结果：

- checkpoint_id 可查询
- checkpoint 关联 goal_id / loop_run_id / step_id
- checkpoint 明确覆盖范围
- checkpoint 不承诺恢复外部不可逆动作
- 写入失败或验证失败且 hash 改变时建议 `RestoreRequired`；有效已验证记录、相同 hash 或一致的 no-write 记录可为 `NoAction`，非法/歧义元数据不自动放行
- 非匹配记录的 JSON 错误也传播，并保留物理行号；查询不创建缺失日志、不读取或重算 workspace hash、不修改 workspace/log，不执行恢复
- 查询仍为全日志线性扫描，只保留最新匹配记录；单条记录大小仍影响内存，不代表已做内存压测

### V8. SideEffectLedger

步骤：

1. 执行只读工具调用
2. 执行本地文件修改
3. 模拟一个不可逆外部动作请求

期望结果：

- 当前持久化 class 经派生解释后，`None/WorkspaceRead → ReadOnly`，`WorkspaceWrite → Reversible`；后者不证明已有恢复快照或 restore 能力
- 外部/未知 class 派生为 `ManualRequired`；未知 audit status 不允许 metadata 自动执行判断。这些仅是 metadata 解释，不新增执行授权或完整恢复/补偿状态机
- 不可逆外部动作被阻断或要求 human approval
- 即使自主模式 policy Allow 或人工 approved，controller Markdown 写入缺 checkpoint/ledger 仍拒绝；前置审计失败不执行 mutation，不声称后置故障自动 rollback

### V9. GoalAlignmentCheck

步骤：

1. 在 goal scope 内执行一步
2. 尝试执行明显 out_of_scope 的动作

期望结果：

- scope 内动作通过或 warning 低
- out_of_scope 动作触发 warning / block / human approval
- 检查结果写入 event log

### V10. DriftGuard

步骤：

1. 做小范围文档修改
2. 模拟大范围无关 diff
3. 模拟核心配置文件修改

期望结果：

- 小范围修改允许
- 大范围无关 diff 触发 warning 或 block
- 核心配置修改要求 checkpoint 或 human approval

### V11. Human Steering Mode

步骤：

1. 默认以 `human_steerable_loop` 运行
2. 高风险动作切换到 `human_in_the_loop`
3. 模拟人工接管 `manual_takeover`

期望结果：

- 当前模式写入 state
- 模式切换写入 event log
- manual takeover 后 agent 不继续自动执行高风险动作

### V12. Baseline Tool Recipes

步骤：

1. 读取 baseline recipe
2. 用 recipe 执行 `rg -F` 或 Markdown link check
3. 模拟一次工具误用并生成 candidate

期望结果：

- tool call 能引用 recipe_id
- recipe 生效
- 失败调用能生成 recipe candidate，但不会直接升级成全局 policy

当前回归：`tests/loop_controller.rs` 的失败工具测试核对内嵌 `LessonCandidate.source_event_ids` 存在于实际 EventLog，且精确引用该失败事件；这不是 recipe 生效、长期记忆或 compact 管线的完整验收。

### V13. Model Routing、Prompt Resolution 与 Fallback

步骤：

1. 注册一个 cheap/fast profile、一个 strong/slow profile 和一个 evaluator profile fixture
2. 让低风险只读任务和高风险复杂任务分别经过规则式路由
3. 为同一模型分别解析 planner、implementer、reviewer prompt
4. 模拟 primary model 不可用，触发能力兼容的 fallback
5. 检查 registry、event、trace、checkpoint 和输出 artifact

期望结果：

- 每次选择生成 `RouteDecision`，包含候选、选择、拒绝原因、成本 / 延迟估算和风险约束
- 高风险任务不会只按最低成本选择模型
- 每次调用关联 `ResolvedPromptPack`、provider、endpoint、model、角色、prompt 版本和 `ModelCallRecord`
- fallback 创建新 attempt 和新 route decision，并重新解析 prompt、重新验证
- 不兼容或高风险降级被阻断或要求 human approval
- 仅存在 `auth_ref`；验证数据中不存在真实 API key、token 或认证响应

失败条件：

- 模型选择或 fallback 静默发生
- 同一套 system prompt 无差别用于所有 provider、model、role 和 task
- 只记录 model name，无法追溯 endpoint、路由理由、prompt 版本或验证状态
- secret 明文进入 registry、prompt、trace、event log 或 checkpoint

当前输入层级回归：`tests/provider_model_adapter.rs` 使用 fake transport 捕获最终请求 JSON；含引号、换行及伪 role JSON 的用户目标/observation 不进入 system/developer，固定 system 仅描述 decision/summary 输出契约，user 数据解码后与原输入一致。这只验证请求层级与编码，不证明真实模型抵抗提示注入，也不代表 provider/model/role/task/skill 解析、实际 ModelCallStore 接线或 fallback 已完成。

### V14. End-to-End Run Summary

步骤：

1. 执行完整开发任务
2. 生成 run summary
3. 从 summary 反查 event、trace、checkpoint、side effect、verification

期望结果：

- summary 包含 goal、steps、tool calls、file changes、verification、checkpoint、side effects、final status
- 每个关键结论能反查证据

### V15. Task 状态迁移

步骤：

1. 按契约依次执行一条合法 Task 状态迁移路径
2. 对每个终态和至少一个非法迁移尝试执行 mutating command
3. 检查 state、EventLog、policy/state decision 和 object_version

期望结果：

- 合法迁移被接受并产生对应事件
- 非法迁移被拒绝，保留当前 object_version，不产生伪造成功事件
- `succeeded` 只有在 success criteria、VerificationResult、evidence、PolicyGate 和 Attempt 结束条件全部满足时出现
- TODO / DOING / DONE 只作为 BoardProjection 显示列，不被当作领域状态

### V16. BoardProjection 重建

步骤：

1. 从一个包含依赖、尝试、验证和失败事件的 EventLog fixture 重建 StateStore 和 BoardProjection
2. 保存 projection 结果后删除 projection，再次从 EventLog 重建
3. 比较两次的 task domain state、board column、current attempt、dependency summary、verification status、evidence completeness 和 blocking issues

期望结果：

- 两次重建结果一致
- BoardProjection 损坏或删除不影响 EventLog
- 显示列符合契约映射，ATTENTION 能区分 blocked 和 failed

### V17. Mutating Command 幂等与 object_version conflict

步骤：

1. 提交一个带 command_id 的状态变更命令两次
2. 使用旧 expected_object_version 提交另一个变更命令
3. 检查命令结果、事件数量、副作用数量和冲突记录

期望结果：

- 相同 command_id 的重试返回第一次结果，不重复执行副作用
- expected_object_version 不匹配时返回 conflict，不覆盖较新的状态
- 查询命令不写 EventLog；暂停、恢复、取消、重试、审批和 reprioritize 都留下可审计事件

### V20. Execution Strategy Selection

步骤：

1. 显式提供低风险、复杂/不确定、高风险、Production、不可逆和需审批的 TaskFeatures
2. 使用策略启动接口，检查 GoalContract 风险下限和策略事件身份/理由/修订/预算
3. 用有限授权的 fixture adapter 验证 research 倍率、策略调整及pause/resume
4. 检查零/已耗尽工具预算在执行器前阻断，且策略不替代 PolicyGate
5. 检查 NaN/Infinity、范围越界、未知字段、倍率不足、零额度和 u32 最大值
6. 在测试自建临时目录内让主策略审计 append 失败，检查旧状态/日志不变，同实例恢复原日志路径后策略revision仍连续

期望结果：

- 选择规则、五个核心策略及两个已定义领域映射符合文档；没有给缺少判定输入的领域策略编造自动映射
- 需审批时不分配动作或调用模型/工具；没有resolver，重选或resume不构成批准
- 策略只能收紧当前预算上限，消耗计数及重复动作fingerprint保持；进展/失败 guard 不乘倍率
- 验证要求只是元数据，不能把它当作已经运行验证或据此宣告任务成功
- 旧 start/CLI 保持兼容；自定义配置、协作执行与策略恢复仍标未实现

失败条件：策略授权扩容、调整清零、审批绕过、PolicyGate 被替代，或把策略验证元数据当作任务验证成功证据。

## 最小验收命令

具体命令等代码实现后确定。

命令应至少覆盖：

```text
build or typecheck
test
run minimal dev-agent task
inspect events
inspect state
inspect trace
inspect checkpoint metadata
inspect side-effect ledger
inspect model registry
inspect route decisions
inspect resolved prompt versions
inspect model call records
inspect task state and allowed transition result
inspect BoardProjection / JSON board fixture
```

## 验证通过标准

第一阶段验证通过必须满足：

- V0 到 V17 至少各有一次通过记录
- 失败用例不能被口头解释为通过
- event log、state、trace 三者能互相对应
- BoardProjection 能从 EventLog 重建且不成为事实源
- checkpoint metadata 与 side-effect ledger 不矛盾
- 高风险动作没有绕过 PolicyGate
- run summary 中的完成结论能追溯到验证命令结果

## 失败处理

如果验证失败：

1. 记录失败事件
2. 保留原始错误输出或摘要
3. 标记失败属于 schema、storage、policy、tool、checkpoint、side-effect、loop 哪一类
4. 生成下一步修复建议
5. 不把失败状态写成完成

## 第一阶段不验证

以下不作为第一阶段验收项：

- 历史表现驱动的自适应多模型路由质量
- 多智能体协作质量
- 向量记忆召回率
- 图记忆推理能力
- 自动安全响应闭环
- 插件市场兼容性
- 企业级多租户隔离
- OS 级 checkpoint / restore
- 交互式 TUI/Web control plane

这些能力只检查是否有接口占位或文档边界；第一阶段的静态 registry、规则式路由、prompt resolution 和调用审计仍属于必验项。
