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
.steerbox/
```

该目录应作为运行产物处理，不应默认提交。

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

期望结果：

- checkpoint_id 可查询
- checkpoint 关联 goal_id / loop_run_id / step_id
- checkpoint 明确覆盖范围
- checkpoint 不承诺恢复外部不可逆动作

### V8. SideEffectLedger

步骤：

1. 执行只读工具调用
2. 执行本地文件修改
3. 模拟一个不可逆外部动作请求

期望结果：

- 只读工具标记为 read_only
- 本地文件修改标记为 reversible_local_change 或 partially_reversible_local_change
- 不可逆外部动作被阻断或要求 human approval

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
