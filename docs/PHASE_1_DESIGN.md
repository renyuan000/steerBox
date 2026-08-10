# Phase 1 Design

## 文档目的

本文件把第一阶段从讨论稿收敛为开发设计。

它不是长期愿景文档，也不是最终 API 规格。第一阶段只验证一个真实可扩展的最小闭环：

```text
shared harness core + software development agent vertical slice
```

## 设计目标

第一阶段必须做到：

- 能运行真实软件开发任务，而不是一次性 demo
- 关键状态不只存在 agent 内存里
- 每一步能审计、能追踪、能复盘
- 本地文件修改有 checkpoint / diff boundary
- 工具调用有 side-effect 分类
- 高风险动作能升级到人类介入
- 为安全智能体、插件、记忆进化保留接口；多模型实现静态 registry、规则式路由和一个真实 adapter，不实现完整自适应系统

## 非目标

第一阶段不做：

- 完整多智能体系统
- 完整 supervisor-workers 架构
- 完整安全运维闭环
- 自动渗透测试执行链
- 向量数据库和图数据库
- 插件市场
- 完整 UI control plane
- 企业级多租户
- OS 级 sandbox checkpoint
- 历史表现驱动的自适应多模型路由和无人审批的高风险 fallback

## 推荐语言边界

语言暂不锁死，但第一阶段建议按职责选择：

- Rust：核心 runtime、事件模型、状态机、策略检查、长期稳定组件
- Python：快速集成模型、工具适配、原型验证、脚本型工具
- Go：后续服务化、控制面、长时后台 worker 可选
- C：第一阶段不主动引入，除非接入底层 sandbox / 系统组件

第一阶段可以先用单一主语言实现最小闭环，但 schema 和存储边界不要写死在某个语言生态里。

## 逻辑模块

### 1. Supervisor

职责：

- 启动 harness runtime
- 管理 agent / worker 生命周期
- 捕获异常退出
- 在状态可恢复时重启内部执行体
- 保持自身极简，不承载业务逻辑

第一阶段实现边界：

- 可以先是一个本地进程入口
- 不实现复杂守护进程
- 不实现分布式调度

### 2. Goal Service

职责：

- 创建和读取 `GoalContract`
- 记录 objective、scope、out_of_scope、success_criteria、verification_plan
- 给 LoopController、PolicyGate、DriftGuard 提供目标依据

最小字段：

```text
goal_id
objective
scope
out_of_scope
success_criteria
constraints
risk_level
allowed_tools
forbidden_tools
verification_plan
human_steering_mode
checkpoint_policy
side_effect_boundary
created_at
updated_at
```

### 3. LoopController

职责：

- 按 `observe -> plan -> act -> record -> verify -> reflect -> update -> checkpoint decision` 推进
- 控制每轮 step
- 调用 ToolRegistry
- 调用 PolicyGate
- 记录 EventLog 和 TraceRecord
- 决定是否触发 GoalAlignmentCheck、DriftGuard、RecoveryCheckpoint、人类介入

第一阶段支持的 loop pattern：

- `single_agent_loop`
- `react_tool_loop`
- `plan_execute_loop`
- `reflection_after_failure`
- `manual_or_policy_router`

### 4. ToolRegistry

职责：

- 注册可用工具
- 描述工具输入输出
- 描述工具副作用等级
- 绑定 baseline tool recipe
- 给 PolicyGate 提供工具风险信息

最小字段：

```text
tool_id
name
description
input_schema
output_schema
side_effect_class
risk_level
allowed_in_modes
recipe_ref
version
```

### 5. PolicyGate

职责：

- 在工具调用、文件修改、checkpoint、恢复、人类介入前做判断
- 防止越权动作
- 防止高风险动作绕过审批

最小决策：

```text
allow
warn
require_checkpoint
require_human_approval
block
```

### 6. EventLog

职责：

- 记录 source of truth 事件
- 支持审计、恢复、复盘、后续 memory candidate

第一阶段建议使用 `events.sqlite`。

最小事件类型：

```text
goal_created
loop_started
step_started
context_loaded
model_route_decided
prompt_pack_resolved
model_call_started
model_call_completed
plan_created
tool_call_requested
tool_call_completed
file_change_recorded
verification_started
verification_completed
checkpoint_created
policy_decision
human_intervention
side_effect_recorded
step_completed
loop_completed
loop_failed
```

### 7. StateStore

职责：

- 保存当前 session / loop / step 状态
- 保存当前 checkpoint 指针
- 保存当前 human steering mode
- 支持 runtime 重启后恢复

第一阶段建议使用 `state.sqlite`。

### 8. TraceStore

职责：

- 保存人类可读 trace
- 便于 grep / tail / 手工复盘

第一阶段建议使用 `traces/*.jsonl`。

Trace 不是 source of truth；source of truth 是 event log 和 state store。

### 9. RecoveryCheckpoint Manager

职责：

- 创建 checkpoint metadata
- 记录 checkpoint 与 event / step / diff 的关联
- 记录可恢复范围
- 记录不可恢复副作用

第一阶段只实现：

- goal / loop state checkpoint
- workspace diff boundary
- checkpoint metadata

不实现：

- OS 级进程快照
- 容器级 checkpoint
- 分布式 checkpoint

### 10. SideEffectLedger

职责：

- 记录工具调用或外部动作的副作用
- 区分 read-only、本地可逆、部分可逆、可补偿、不可逆、人工处理
- 为 restore / replay / compensation 提供依据

第一阶段所有 tool call 都必须至少有 side-effect class。

### 11. GoalAlignmentCheck

职责：

- 检查当前动作是否仍服务 root goal / stage goal
- 检查是否 scope creep
- 检查是否把辅助问题升级成主任务

第一阶段可以输出人工可读报告，不要求复杂 evaluator。

### 12. DriftGuard

职责：

- 检查 diff 规模和文件范围
- 检查是否触碰核心配置、规则、文档或架构文件
- 检查是否出现过度重构、风格 churn 或不必要大改

第一阶段可以先做静态规则和人工可读报告。

### 13. Baseline Tool Recipes

职责：

- 预置高频工具正确用法
- 减少基础工具误用
- 为后续 tool recipe evolution 提供起点

第一批 recipe：

- shell safe quoting
- `rg -F` fixed-string search
- git status / diff read-only inspection
- Markdown link check
- test command record
- file write verification

### 14. Model Gateway、Registry 与 Router

职责：

- 以 `ProviderProfile -> EndpointProfile -> ModelProfile` 分层描述模型接入
- 通过静态 `ModelRegistry` 和 `PromptRegistry` 加载多个模型与提示 profile
- 按 task type、agent role、能力、风险、成本和延迟约束执行规则式路由
- 按 provider、model、role、task 解析 `ResolvedPromptPack`
- 对 fallback 做能力兼容检查，禁止静默切换
- 记录 `RouteDecision`、`ModelCallRecord` 和 prompt 版本

第一阶段实现边界：

- 至少一个真实 model adapter
- 允许配置 cheap/fast、strong/slow、specialized、evaluator、local 等 profile；未接 live endpoint 的 profile 使用静态 fixture 验证
- 不自动学习路由权重，不自动提升 prompt，不执行无人审批的高风险多模型协作
- registry、prompt、trace、event log 和 checkpoint 只保存 `auth_ref`，不保存 API key 或 token

## 存储布局

第一阶段推荐运行数据目录：

```text
.steerbox/
  state.sqlite
  events.sqlite
  traces/
    <session_id>.jsonl
  checkpoints/
    <checkpoint_id>/
      metadata.json
      diff.patch
  recipes/
    baseline-tools.jsonl
  registries/
    models.yaml
    prompts.yaml
  runs/
    <loop_run_id>/
      summary.json
```

说明：

- 具体目录名后续实现可调整
- 运行数据目录应默认不污染项目源码提交
- 文档阶段只定义逻辑结构，不提前创建目录

## 最小数据关系

```text
GoalContract
  -> LoopRun
    -> Step
      -> ToolCall
      -> PolicyDecision
      -> SideEffectEvent
      -> VerificationResult
      -> TraceRecord
      -> CheckpointRef
      -> RouteDecision
        -> ResolvedPromptPack
        -> ModelCallRecord
```

## 第一阶段端到端流程

```text
create goal
  -> start loop run
  -> load context
  -> select provider / endpoint / model
  -> resolve model / role / task prompt
  -> record route decision
  -> plan next step
  -> policy check
  -> run tool or edit file
  -> record event and trace
  -> classify side effect
  -> run verification
  -> run goal alignment / drift check when needed
  -> create checkpoint when needed
  -> complete / continue / ask human / fail
```

## 第一阶段开发切片

建议选择一个真实但小的开发任务，例如：

- 修改一个 Markdown 文档并验证链接
- 修改一个小型代码函数并运行单元测试
- 添加一个简单 CLI 子命令并验证输出

验收重点不是任务难度，而是完整记录和可复盘。

## 关键设计原则

- 事件先行：没有事件记录，就不算真正执行过
- 状态外置：agent 可以重启，状态不能只在 agent 内存里
- 工具受控：每个 tool call 都要有 policy 和 side-effect 语义
- 恢复有边界：只承诺能恢复明确记录的范围
- 人类可掌舵：自动执行不是无人负责
- 模型可替换但不可静默：模型、endpoint、路由理由和 prompt 版本必须进入 trace
- secret 外置：模型注册表和运行记录只保存 auth_ref，不保存认证明文
- 复杂能力占位：未来能力留接口，不进入第一阶段交付压力
