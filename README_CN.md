# steerBox

[README.md](./README.md) | [English](./README_EN.md)

在一个安全、可控、可引导的环境中，通过软件开发、持续运维、安全评估与漏洞研究、安全运营四个专业方向，练习 Harness Engineering。

## 项目定位

`steerBox` 当前首先是一个 `learning-first` 的 Harness Engineering 学习项目。

它关注的核心问题不是“让 agent 会不会做事”，而是如何把下面这些能力稳定组合成一个可持续运行的 harness：

- 长时执行
- 约束控制
- 反馈闭环
- 支持人在环、无人值守、被动观察、主动审批与人工接管
- 行为可审计
- 决策与执行流程可追踪

## 四个专业方向

### 1. 软件开发智能体

当前关注：

- 长时间自动化开发
- 代码管理
- 测试执行
- 重构
- 开发工作流控制
- 持续反馈与持续修正

### 2. 持续运维智能体

当前关注：

- 服务健康、可用性、性能、容量和成本治理
- 部署和变更观察
- 故障定位、恢复编排和恢复后验证
- 混合云资源治理

### 3. 安全评估与漏洞研究智能体

当前关注：

- 授权安全审查与攻击面分析
- 漏洞研究与受控 PoC 模拟验证
- 授权渗透测试与攻击路径验证
- 有证据的修复复测

### 4. 安全运营智能体

当前关注：

- 持续接入并关联日志、告警、资产和威胁情报
- 调查漏洞、后门、病毒和 APT 相关线索
- 编排受控验证和分级处置
- 允许用户随时查询、审批、暂停、恢复或接管长期调查

说明：

- 与高影响安全动作相关的自动化，应默认建立在受控、可审计、可追踪和可介入的前提下
- 需要主动动作的场景，应按风险等级决定无人值守、被动观察、主动审批或人工接管模式

## 核心原则

- `safe`
- `steered`
- `auditable`
- `traceable`
- `human-steerable`
- `long-running`
- `feedback-driven`

## 当前状态

- 第一阶段主语言和构建工具已确定为 `Rust` + `Cargo`
- 当前已实现并验证领域契约、配置校验、append-only JSONL EventLog（含分页读取）、StateStore 重建/持久化、Trace/Checkpoint/SideEffect 记录、GoalAlignment/DriftGuard 最小报告和只读 BoardProjection fixture
- 当前开发垂直切片可执行受审批的 Markdown 文件修改与链接验证，成功/失败均写入 run summary；验证失败会保留事件、状态和摘要并以非零退出
- 当前尚未完成连接真实模型并自主开发的端到端验收，研发目标仍是模型连接型 Agent：已有本地 fixture、OpenAI-compatible/Anthropic adapter、fake/localhost HTTP transport、注入式 credential resolver、secret-free profile/auth_ref 配置绑定、规则路由，以及 opt-in 的 `ResolvedPromptPack` → LoopController → ModelCallStore 审计路径。新增 `loop provider` 是一个 bounded foreground runner 入口，但默认 CLI 使用永久拒绝的 resolver，未配置可信 credential resolver 时必然 fail-closed；它不能被称为 live provider CLI。成功调用保留 binding/wire hash/usage/finish_reason，resolved adapter-error 分支另存 `ModelCallFailureRecord`，不保存原始 provider error 或伪造未知响应字段；审计错误 fail-closed。routed fixture/model-adapter fallback 已有 child Attempt 与共享预算，只读动作 → PolicyGate → 工具 → 验证闭环已有；尚未完成真实 credential store/live 调用、resolved-provider fallback、真实模型驱动编辑、通用文件写入/Bash、后台控制与完整恢复
- 候选记忆已有 JSONL metadata 持久化、`read_catalog()` 和 controller project/goal/task 范围检索；requested/filtered/used 事件不包含摘要正文，used 仅表示返回候选 metadata。尚不注入 prompt、提升为 active memory、授予权限或形成 Wiki/EvidencePack 完整链
- 历史查询支持 metadata-only `execution list/show/page`，分页是读取全量事件后的 detached snapshot，不是流式读取或 live cursor；CLI 生命周期输出仅在真实完成时报告 loop completed
- 执行策略已有可验证的库 API 切片：按显式任务特征选择策略，记录选择/调整事件，将倍率分配限制在既有授权预算内，并在需审批时拒绝动作；调整不清零消耗或重复动作历史。验证频率与协作/专业领域策略目前只是需求元数据，不代表自动验证、多 agent 编排或专业操作已执行
- 用户中途接入后台任务已有设计契约，但没有常驻后台 worker 或可连接它的交互控制面；见 [Human Steering Modes](./docs/HUMAN_STEERING_MODES.md) 与 [Control Plane Discussion](./docs/CONTROL_PLANE_DISCUSSION.md)

最小验证命令：

```bash
cargo test
cargo run -- runtime init demo-project
cargo run -- config validate /temp/steerbox/demo-project/config.json
cargo run -- event list /temp/steerbox/demo-project/config.json
cargo run -- event append /temp/steerbox/demo-project/config.json note_added note note-1 event-1 corr-1 '{"summary":"fixture"}'
cargo run -- loop readonly /temp/steerbox/demo-project/config.json demo-project goal-1 loop-1 task-run-1 step-1 'inspect repository' 'repository inspected'
cargo run -- loop fixture /temp/steerbox/demo-project/config.json demo-project goal-1 loop-2 task-run-2 local-fixture complete_loop 'repository inspected' 'fixture complete'
cargo run -- execution list /temp/steerbox/demo-project/config.json
cargo run -- execution show /temp/steerbox/demo-project/config.json loop-1
cargo run -- execution page /temp/steerbox/demo-project/config.json loop-1 0 10
```

当前 fixture loop 只使用本地结构化决策，不调用真实模型、不执行工具、不修改文件；它会记录 `model_call_started`、`model_call_completed`，以及 `complete_loop` 或 `request_human` 对应事件。运行数据固定写入 `/temp/steerbox/<project_ref>/.steerbox/`，不写入仓库工作区。

`continue_observation` 和 `request_human` 都不表示 loop 已完成；后者已有人工介入事件，但当前 loop 状态仍为 Running，不应误报 Paused。路由入口目前是库 API，不是上述 fixture CLI 自动使用的入口。审计 schema 拒绝未知认证字段并校验标识/成本，不等于对任意字符串内容做凭据检测。

策略同样显式启用：调用 `ReadOnlyLoopController::start_with_strategy(log, goal, loop_run_id, task_run_id, base_budget, authorized_budget, features)`，或在安全边界调用 `select_strategy(features, &base_budget)`。旧 `start()` 和当前 fixture CLI 不自动启用选择器。`features` 必须由调用者提供，当前不从自然语言猜测复杂度、风险或可逆性；实际风险不会低于 GoalContract。需审批的策略可完成选择并留下审计，后续动作入口返回 `StrategyApprovalRequired`；没有审批 resolver，重选及pause/resume不会解除该门禁。

`base_budget` 是请求的基础分配，`authorized_budget` 是硬上限；同实例后续调整只会收紧当前上限，不重置tracker或action fingerprint。新建 start_with_strategy 不是跨实例恢复入口。策略路径已前置约束 step/model/tool 额度及进展/失败 guard，但 Attempt、GoalRevision 和自动验证失败统计的完整运行时接线仍未完成，不能称八维都已执行。自定义策略配置、模型协作和策略恢复也尚未实现。

策略切片的可复现验证（构建产物不放源码目录）：

```bash
CARGO_TARGET_DIR=/tmp/steerbox-phase1-target CARGO_BUILD_JOBS=2 cargo test --test strategy_selector --test budget_calculator --test loop_strategy --test loop_strategy_controls --test strategy_audit_failure
```

## 文档

- [Harness Principles](./docs/HARNESS_PRINCIPLES.md)
- [Harness Sources](./docs/HARNESS_SOURCES.md)
- [Architecture Direction](./docs/ARCHITECTURE_DIRECTION.md)
- [Architecture Questions](./docs/ARCHITECTURE_QUESTIONS.md)
- [Documentation Map](./docs/DOCUMENTATION_MAP.md)
- [Agent R&D Roadmap](./docs/AGENT_RND_ROADMAP.md)
- [Continuous Operations Agent Design](./docs/CONTINUOUS_OPERATIONS_AGENT_DESIGN.md)
- [Security Assessment Agent Design](./docs/SECURITY_ASSESSMENT_AGENT_DESIGN.md)
- [Security Operations Agent Design](./docs/SECURITY_OPERATIONS_AGENT_DESIGN.md)
- [Shared Harness Security Model](./docs/SECURITY_MODEL_DISCUSSION.md)
- [Tracks and Supporting Capabilities](./docs/TRACKS_AND_CAPABILITIES.md)
- [Goal and Loop Engineering Discussion](./docs/GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md)
- [Goal Alignment Checkpoints](./docs/GOAL_ALIGNMENT_CHECKS.md)
- [AI Agent Efficiency Engineering](./docs/AI_AGENT_EFFICIENCY_ENGINEERING.md)
- [Plugin and Extension Architecture Discussion](./docs/PLUGIN_AND_EXTENSION_ARCHITECTURE_DISCUSSION.md)
- [Model Prompt and Checkpoint Policy](./docs/MODEL_PROMPT_AND_CHECKPOINT_POLICY.md)
- [Prompt, Checkpoint, and DriftGuard Templates](./docs/PROMPT_CHECKPOINT_TEMPLATES.md)
- [Checkpoint Policy Algorithm](./docs/CHECKPOINT_POLICY_ALGORITHM.md)
- [Side Effect Ledger Discussion](./docs/SIDE_EFFECT_LEDGER_DISCUSSION.md)
- [Drift Guard Discussion](./docs/DRIFT_GUARD_DISCUSSION.md)
- [Reference Frameworks and Observability Notes](./docs/REFERENCE_FRAMEWORKS_AND_OBSERVABILITY.md)
- [Agent Evolution Reading List and Notes](./docs/AGENT_EVOLUTION_READING_LIST.md)
- [Agent Evolution Innovation Synthesis](./docs/AGENT_EVOLUTION_INNOVATION_SYNTHESIS.md)
- [Storage and Recovery Discussion](./docs/STORAGE_AND_RECOVERY_DISCUSSION.md)
- [Phase 1 Implementation Scope](./docs/PHASE_1_IMPLEMENTATION_SCOPE.md)
- [Human Steering Modes](./docs/HUMAN_STEERING_MODES.md)
- [Control Plane Discussion](./docs/CONTROL_PLANE_DISCUSSION.md)
