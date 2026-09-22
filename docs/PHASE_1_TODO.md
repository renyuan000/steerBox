# Phase 1 TODO

## 文档目的

本文件是第一阶段开发任务清单。

它服务于实际开工，不承载长期理论设计。任务完成标准必须可验证。

## 当前阶段目标

第一阶段目标：

```text
实现 shared harness core 的最小闭环，
并用软件开发智能体垂直切片验证 goal、loop、event、checkpoint、policy、side-effect、trace 是否能协同工作。
```

## 任务分组

### P0. 开工前整理

- [ ] 阅读并确认 PHASE_1_CONTRACTS.md 是 schema、event、state 和 projection 的 Phase 1 规范源
- [ ] 确认当前文档变更已提交或明确保留在工作区
- [ ] 确认第一阶段只按 `PHASE_1_IMPLEMENTATION_SCOPE.md` 和 `PHASE_1_DESIGN.md` 开发
- [ ] 确认第一阶段不创建持续运维、安全评估与漏洞研究、安全运营的完整闭环
- [ ] 确认运行数据目录不会污染源码提交

验收：

- `git status --short` 输出已检查
- 第一阶段边界无歧义

### P1. 项目骨架

- [ ] 选择第一阶段主语言和构建工具
- [ ] 创建最小源码目录
- [ ] 创建最小测试目录
- [ ] 创建最小 CLI 或 runtime 入口
- [ ] 添加基础格式化 / lint / test 命令

验收：

- 能运行 `build` 或等价命令
- 能运行最小测试
- README 或开发说明中记录命令

### P2. Core Schema

- [ ] 定义 EventEnvelope、event_id、sequence_no、correlation_id 和 causation_id
- [ ] 定义 Portfolio、Project、Task、TaskRun、Attempt 和 TaskDependency
- [ ] 定义 Task 状态及合法迁移表
- [ ] 定义 BoardProjection / BoardTaskCard
- [ ] 定义 ToolUseErrorEvent、LessonCandidate 和最小 FailureCategory

- [ ] 定义 `GoalContract`
- [ ] 定义 `LoopRun`
- [ ] 定义 `LoopStep`
- [ ] 定义 `ToolCallRecord`
- [ ] 定义 `PolicyDecision`
- [ ] 定义 `SideEffectEvent`
- [ ] 定义 `CheckpointMetadata`
- [ ] 定义 `VerificationResult`
- [ ] 定义 `TraceRecord`
- [ ] 定义 `ProviderProfile`、`EndpointProfile`、`ModelProfile`
- [ ] 定义 `PromptProfile`、`AgentRolePrompt`、`TaskPromptPack`、`ResolvedPromptPack`
- [ ] 定义 `RouteDecision` 和 `ModelCallRecord`

验收：

- schema 有单元测试或序列化测试
- 字段能覆盖第一阶段端到端流程
- schema 有版本字段或预留版本策略

### P3. Local Storage

- [ ] 实现从 EventLog 重建 current state / BoardProjection 的最小 fixture

- [ ] 初始化 `events.sqlite`
- [ ] 初始化 `state.sqlite`
- [ ] 实现 append-only event write
- [ ] 实现 current state update
- [ ] 实现 trace JSONL append
- [ ] 实现 run summary 写入

验收：

- 写入一轮 fake loop 后，event/state/trace 都能查询
- 进程重启后能读取 current state
- trace 文件可被 `tail` / `rg` 阅读

### P4. ToolRegistry

- [ ] 定义 `ToolProfile`
- [ ] 注册只读工具
- [ ] 注册 shell / command 工具的受控包装
- [ ] 为每个工具配置 side-effect class
- [ ] 为每个工具绑定 risk level
- [ ] 读取 baseline tool recipes

验收：

- 未注册工具不能调用
- 工具调用前能查到 side-effect class
- `rg`、git status、Markdown link check 至少有 recipe

### P5. PolicyGate

- [ ] 实现工具调用前 policy check
- [ ] 实现文件写入前 policy check
- [ ] 实现高风险动作升级到 human approval
- [ ] 实现 forbidden tool 阻断
- [ ] 实现 policy decision event 记录

验收：

- 低风险只读工具允许执行
- 未授权写操作被阻断或要求审批
- policy decision 可在 event log 中复盘

### P6. LoopController

- [ ] 支持 Task 状态合法迁移和非法迁移拒绝

- [ ] 实现 `single_agent_loop` skeleton
- [ ] 实现 `react_tool_loop` 的最小 step 流程
- [ ] 实现 `plan_execute_loop` 的最小阶段状态
- [ ] 支持 step start / complete / fail
- [ ] 支持 stop condition
- [ ] 支持失败后 reflection summary

验收：

- 能跑完一个无文件修改的只读 loop
- 能跑完一个有文件修改和验证的 dev loop
- 每个 step 都有 event 和 trace

### P7. RecoveryCheckpoint

- [ ] 实现 checkpoint metadata 写入
- [ ] 记录 checkpoint 与 goal / loop / step 的关系
- [ ] 记录 checkpoint 前后的文件 diff boundary
- [ ] 记录 checkpoint 覆盖范围
- [ ] 记录不可恢复 side effect 引用

验收：

- 文件修改前后能生成 checkpoint metadata
- 能说明某次修改是否可恢复
- checkpoint 不声称恢复未记录的外部动作

### P8. SideEffectLedger

- [ ] 实现 side-effect class 枚举
- [ ] 实现 tool call 到 side-effect event 的记录
- [ ] 支持 read-only、本地可逆、部分可逆、可补偿、不可逆、人工处理分类
- [ ] 高风险副作用要求 policy gate

验收：

- 每个 tool call 都有 side-effect 记录或明确 none
- 不可逆动作默认不能自动执行
- restore / replay 前能查询 side-effect boundary

### P9. GoalAlignmentCheck

- [ ] 实现 root goal 对齐检查报告
- [ ] 实现 stage goal 对齐检查报告
- [ ] 检查 scope / out_of_scope
- [ ] 检查是否需要 human intervention
- [ ] 将检查结果写入 event log

验收：

- 故意偏离任务时能产生 warning 或 block 建议
- 检查结果能关联到 step_id

### P10. DriftGuard

- [ ] 检查修改文件数量
- [ ] 检查 diff 行数
- [ ] 检查核心文件触碰
- [ ] 检查大规模重命名 / 删除
- [ ] 输出 allow / warn / require_checkpoint / require_human_approval / block

验收：

- 小范围文档修改允许
- 大范围无关修改触发 warning 或 approval
- 核心配置修改触发更高风险决策

### P11. Baseline Tool Recipes

- [ ] ToolUseErrorEvent 能关联原始 tool call、failure category 和 LessonCandidate

- [ ] 写入 shell safe quoting recipe
- [ ] 写入 `rg -F` recipe
- [ ] 写入 git status / diff read-only recipe
- [ ] 写入 Markdown link check recipe
- [ ] 写入 file write verification recipe

验收：

- ToolRegistry 能读取 recipe
- tool call 记录能引用 recipe id
- 失败工具调用能生成 recipe candidate

### P12. Model Registry、Prompt Resolution 与规则路由

- [ ] 实现静态 `ModelRegistry` 和 `PromptRegistry`
- [ ] 接入至少一个真实 model adapter
- [ ] 配置 cheap/fast、strong/slow、specialized、evaluator 或 local profile fixture
- [ ] 按 task type、agent role、能力、风险、成本和延迟执行规则式路由
- [ ] 生成并持久化 `RouteDecision`
- [ ] 按 provider、model、role、task 解析 `ResolvedPromptPack`
- [ ] 记录 `ModelCallRecord`、prompt 版本和验证状态
- [ ] 实现能力兼容、非静默的 fallback 事件
- [ ] 验证 registry、prompt、trace、event log 和 checkpoint 不保存认证明文

验收：

- 低风险只读任务可路由到 cheap/fast profile
- 高风险或高质量任务不会只按最低成本选择模型
- 切换模型会创建新的 attempt / route decision，而不是静默覆盖
- 每次调用能反查 provider、endpoint、model、prompt 版本、路由理由和验证结果
- 仅保存 `auth_ref`；测试 fixture 不包含真实 API key 或 token

### P13. Dev Agent Vertical Slice

- [ ] 输出只读 BoardProjection / JSON board fixture

- [ ] 定义一个小型真实开发任务
- [ ] 创建 goal
- [ ] 加载上下文
- [ ] 执行文件修改
- [ ] 运行验证命令
- [ ] 记录 checkpoint 和 side effect
- [ ] 输出 run summary

验收：

- 任务能端到端完成
- event log 可复盘
- trace 可读
- checkpoint metadata 存在
- 验证结果明确

### P14. Documentation Update

- [ ] 更新 PHASE_1_CONTRACTS.md 的引用和当前实现状态

- [ ] 更新 README 或开发说明中的运行命令
- [ ] 更新第一阶段完成状态
- [ ] 记录已实现能力和占位能力
- [ ] 记录已知限制

验收：

- 用户能按文档跑最小验证
- 文档不把占位能力写成已实现能力

## 第一阶段暂不做 TODO

以下不要在第一阶段拆任务：

- 历史表现驱动的自适应多模型路由
- 多智能体 supervisor-workers
- 向量记忆检索
- 图记忆推理
- 自动 skill 生成
- 安全响应自动处置
- 插件市场
- 交互式 TUI/Web control plane
- 分布式 worker

## 完成定义

第一阶段完成必须同时满足：

- 一个真实开发任务端到端完成
- 事件日志完整
- trace 可读
- checkpoint metadata 可查
- side-effect ledger 可查
- policy decision 可查
- goal alignment 或 drift guard 至少触发一次有效检查
- 验证命令有明确结果
- 至少一次模型路由、prompt 解析和调用记录能通过 event / trace 复盘
- Task 状态迁移和 BoardProjection 重建至少各有一次通过记录
- 失败工具调用至少生成一次可审查 LessonCandidate
