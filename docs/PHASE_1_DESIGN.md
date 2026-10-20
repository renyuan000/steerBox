# Phase 1 Design

## 文档目的

本文件把第一阶段从讨论稿收敛为开发设计。

它不是长期愿景文档，也不是最终 API 规格。第一阶段只验证一个真实可扩展的最小闭环：

```text
shared harness core + software development agent vertical slice
```

第一阶段实现契约集中在 PHASE_1_CONTRACTS.md。本文件说明模块和数据流；对象字段、事件 envelope、状态迁移、幂等和投影重建规则以契约文档为准。

## 设计目标

第一阶段必须做到：

- 能运行真实软件开发任务，而不是一次性 demo
- 关键状态不只存在 agent 内存里
- 每一步能审计、能追踪、能复盘
- 本地文件修改有 checkpoint / diff boundary
- 工具调用有 side-effect 分类
- 高风险动作能升级到人类介入
- 为持续运维、安全评估与漏洞研究、安全运营、插件和记忆进化保留接口；多模型实现静态 registry、规则式路由、AI 辅助但运行时裁决的候选选择、当前任务内有限自适应和一个真实 adapter；不实现跨任务在线学习或完整自适应系统

## 非目标

第一阶段不做：

- 完整多智能体系统
- 完整 supervisor-workers 架构
- 持续运维、安全评估与漏洞研究、安全运营的完整闭环
- 自动渗透测试执行链
- 向量数据库和图数据库
- 插件市场
- 真实多项目并行调度和交互式 TUI/Web control plane
- 企业级多租户
- OS 级 sandbox checkpoint
- 跨任务在线学习、自动训练路由权重和无人审批的高风险 fallback；但允许实现当前任务内、预算约束下、基于确定性质量信号的有限效果升级和低风险 fallback

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
- 在模型返回、工具完成、LoopStep 完成、验证开始、checkpoint 创建和 heartbeat 边界读取 `UserCommandInbox`
- 在高风险动作前重新读取控制指令；安全边界外的 pause/cancel/takeover 必须进入 `waiting_human` 或 `blocked`
- 对每个 TaskRun 维护 `LoopBudget` 和 `NoProgressObservation`，达到任一预算或重复动作上限后停止自动重试

第一阶段支持的 loop pattern：

- `single_agent_loop`
- `react_tool_loop`
- `plan_execute_loop`
- `reflection_after_failure`
- `manual_or_policy_router`

用户 steering 与 loop guard 的运行顺序：

```text
receive command -> persist UserCommandInbox
  -> classify control vs steer vs independent task
  -> apply control at safe boundary
  -> create GoalRevision and invalidate old Plan when relevant
  -> execute one bounded step
  -> record progress observation and budget counters
  -> continue, wait_human, blocked, failed, or complete
```

普通输入不能直接覆盖当前 context；相关输入必须产生 `GoalRevision`/`PlanRevision`，无关输入创建独立 Task。预算、Attempt lineage、retry count 和无进展计数在插话后保持不变。

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

Phase 1 的工具执行必须与注册和策略判断分层。当前只实现一个受控只读执行切片：`git_status` 使用固定 argv `git status --short --branch`，调用方只能提供结构化 arguments，不能传入 shell 字符串；workspace 必须是已存在目录，子进程有固定超时，stdout/stderr 通过有界管道读取并限长。策略通过但执行失败必须记录 `tool_call_failed`；成功才记录 `tool_call_completed`。`rg_fixed` 和 `markdown_link_check` 目前只有 registry/recipe 边界，尚未接入真实执行器。任何写入、网络或外部系统动作仍不得通过该执行器执行。

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

- 记录执行事实源事件；StateStore 和 BoardProjection 从事件重建
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
user_command_received
steering_request_classified
goal_revision_created
plan_invalidated
interrupt_requested
interrupt_applied
no_progress_detected
loop_budget_exhausted
human_takeover_requested
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

Trace、StateStore 和 BoardProjection 都不是独立事实源；EventLog 是执行事实源，StateStore 和 BoardProjection 必须能够从事件重建。

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
- 通过统一 `ProgressEvent` 向 CLI/TUI/Web adapter 提供短状态和详情引用
- 通过 `QualityAssessment` 与 `AdaptationController` 在当前任务内有限升级效果级别；不允许改变安全底线、endpoint、`auth_ref` 或数据出口

第一阶段实现边界：

- 至少一个真实 model adapter
- 允许配置 cheap/fast、strong/slow、specialized、evaluator、local 等 profile；未接 live endpoint 的 profile 使用静态 fixture 验证
- 不自动学习路由权重，不自动提升 prompt，不执行无人审批的高风险多模型协作；AI 只能提出候选或性能档位建议，不能改变 endpoint、auth_ref 或安全底线
- registry、prompt、trace、event log 和 checkpoint 只保存 `auth_ref`，不保存 API key 或 token

受约束自适应的详细状态机、质量信号、效果级别和 Phase 1/2 边界见 [ADAPTIVE_MODEL_ROUTING_AND_QUALITY_POLICY_2026_10_20.md](ADAPTIVE_MODEL_ROUTING_AND_QUALITY_POLICY_2026_10_20.md)。Phase 1 只做顺序自适应，不做并行 quorum 或跨任务在线学习。

交互输出约束：默认输出紧凑的一行状态/动作摘要，长参数和历史通过详情查询获取；CLI/TUI/Web 只渲染结构化事件，不各自重建事实。交互式 TUI/Web control plane 仍不属于 Phase 1 交付。

### 15. Portfolio、Task 与 BoardProjection

职责：

- 保存单 Portfolio / Project 下可调度 Task 的最小身份
- 区分 Task、TaskRun 和 Attempt 的生命周期
- 将 EventLog 和可重建 StateStore 派生为只读 BoardProjection
- 为未来 Scheduler、worker、TUI 和 Web adapter 保留稳定查询边界

第一阶段边界：

- 只实现单项目、本地顺序 fixture
- 只提供只读 board / JSON projection
- 不实现多项目并行 worker、workspace lease、join 或 backpressure runtime

## 存储布局

第一阶段推荐运行数据目录（必须位于持久化 `/temp`，不能放在易因 WSL2 重启丢失的临时目录）：

```text
/temp/steerbox/<project_ref>/.steerbox/
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

当前已实现的最小切片：`RuntimeConfig` 使用 JSON 配置文件，配置只接受 `auth_ref` 而不接受认证明文；`EventLog` 以 append-only JSONL 落盘并支持分页读取；`StateStore` 可从事件重建 loop/step projection，并以 JSON 原子写入方式持久化后 reopen；`TraceRecord`、`CheckpointMetadata`、最小 `SideEffectRecord`、`UserCommandInbox` 和 dev loop 的 `RunSummary` 可持久化；`UserCommandInbox` 已验证 command_id 幂等、不同内容冲突拒绝、状态追加和按 loop 的 pending 去重；`GoalAlignmentCheck` 与 `DriftGuard` 已有静态规则报告和测试；软件开发垂直切片可执行受审批的 Markdown 修改、链接验证并在验证失败时以非零退出。SQLite、LoopController 安全边界读取与命令应用、pause/resume/takeover 控制面、完整 side-effect 执行接线、真实模型 provider 和完整恢复 runtime 仍未实现，不能把当前切片描述为完整 Phase 1 Agent。

说明：

- 具体目录名后续实现可调整
- 运行数据目录应默认不污染项目源码提交；`.gitignore` 必须排除项目内的 `target/` 和 `.steerbox/` 兜底路径
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

## 多 LLM／中转／调度扩展约束（2026-10-12）

用户要求的多后端能力必须按可验收边界拆开，不能把静态 `RouteDecision` 或单一 provider adapter 误称为完整多模型 Agent：

- **并行接入**：需要独立的并发执行器、每个模型独立的 `ModelCallRecord`、共享 TaskRun/Attempt 预算、聚合与取消语义；当前尚未实现并行 worker 或 quorum。
- **中转平台**：需要显式 `TrustedRelay` endpoint contract，声明协议、请求路径、逻辑 provider/model 身份、独立 `auth_ref` 和健康/错误分类；不能仅凭 `base_url` 是否自定义来推断中转能力。
- **自动切换**：fallback 必须创建新 Attempt，保留原失败 evidence、route/prompt/wire hash 和累计预算；当前 `next_fallback_model()` 仍只是纯规划函数，不能自动重试。
- **策略调试**：每次选择要能解释输入特征、候选、拒绝原因、预算/成本/延迟边界和 revision；当前已提供 secret-free deterministic `RouteExplanation`，但还没有跨进程完整策略回放。
- **能力／花费调度**：能力、风险、隐私、估算成本和延迟只能作为显式约束；真实 usage、成本结算、延迟反馈和 reliability feedback 仍需独立验收，不能用静态估算冒充实时计费。
- **厂商差异**：OpenAI-compatible、Anthropic、其他后端必须保留独立 wire contract、headers、stream/finish/usage 语义和错误分类；统一层只接收经过验证的 provider-neutral response/action envelope。
- **缓存与复用**：prompt cache 需要 provider capability、隔离的 cache key、TTL/失效、hit/miss/bypass 审计以及 project/goal 可见性边界；当前明确未实现 prompt cache，不得在真实账号验证前宣称支持缓存强化。

真实 LLM 验证门槛：先完成上述离线 fake/localhost contract tests，再由用户提供专用测试账号或中转测试凭据；凭据只通过注入式 resolver 使用，不写入配置、事件、日志、测试 fixture 或提交。真实验证计划必须覆盖至少两个 provider/endpoint、并行或切换、能力/成本约束、厂商特殊请求格式和流式/非流式边界。
