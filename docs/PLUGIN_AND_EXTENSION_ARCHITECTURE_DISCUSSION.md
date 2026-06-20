# steerBox Plugin and Extension Architecture Discussion Draft

## 文档目的

本文件用于沉淀 `steerBox` 未来如何支持主流思想、模型、工具、skills、MCP、memory、retrieval、evaluator、policy、UI/control plane 等插件式接入。

本文件是讨论稿，不是最终插件 API 规格，也不代表当前仓库已经具备插件系统。

## 核心判断

`steerBox` 要长期演进，不能把能力写死在核心里。

更合理的方向是：

```text
minimal core + capability registry + policy gate + extension adapters + audit/tracing
```

核心负责：

- 生命周期
- 状态
- 事件
- 权限
- 策略
- 审计
- tracing
- checkpoint / restore
- 插件注册与隔离

插件负责：

- 模型接入
- 工具接入
- skill 接入
- MCP 接入
- 领域能力
- evaluator
- memory backend
- retrieval backend
- code indexer
- UI/control plane

## 插件不等于信任

插件系统不能被设计成“装上就可信”。

每个插件都应被视为 capability provider，而不是 trusted core。

因此必须具备：

- capability 声明
- 权限范围
- 输入输出 schema
- side effect 声明
- 风险等级
- 审计记录
- 版本号
- 可禁用
- 可回滚
- 可替换

尤其是 MCP、外部 tool、自动生成 skill、第三方 evaluator，都必须通过 policy gate，而不是直接进入核心信任边界。

## 建议插件类型

### 1. Model Provider Plugin

用于接入不同模型和推理服务。

可能来源：

- OpenAI
- Anthropic
- local model
- vLLM / Ollama / llama.cpp 类本地服务
- 企业内部模型服务

接口关注点：

- model id
- context window
- tool calling 能力
- streaming 能力
- structured output 能力
- cost profile
- latency profile
- safety profile
- retry policy
- rate limit

设计要求：

- 不把某个模型供应商写死进核心
- 模型能力差异必须显式进入调度和 loop 策略
- 同一 GoalContract 可根据风险和成本选择不同模型
- 多模型接入必须服务效率工程，而不是为了堆模型数量

### 2. Model Router Plugin

用于在多个模型之间做任务分配和动态路由。

核心目标：

- 根据费用选择更经济的模型
- 根据任务难度选择足够强的模型
- 根据模型特长分配不同任务类型
- 根据可访问性和限流情况做 fallback
- 根据历史效率指标持续调整路由策略
- 根据架构需求决定是否需要强推理、长上下文、工具调用、结构化输出或低延迟

路由因素：

- `cost_per_input_token`
- `cost_per_output_token`
- `latency_profile`
- `availability`
- `rate_limit`
- `context_window`
- `tool_calling_quality`
- `structured_output_quality`
- `coding_quality`
- `security_analysis_quality`
- `reasoning_strength`
- `long_context_reliability`
- `local_deployment_available`
- `privacy_requirement`
- `task_difficulty`
- `risk_level`
- `architecture_need`
- `historical_success_rate`
- `tokens_per_solved_task`
- `wall_time_to_verified_result`

典型分配策略：

- 简单分类、摘要、格式整理：优先低成本模型或本地模型
- 代码定位、日志初筛、重复性检查：优先低延迟和低成本模型
- 复杂架构设计、疑难 bug、安全分析：使用强推理模型
- 高风险动作决策：使用强模型 + evaluator + policy gate
- 长上下文代码库理解：使用长上下文模型或 retrieval + 中等成本模型组合
- 隐私敏感数据：优先本地模型或受控内部模型
- 模型不可用或限流：自动降级到备用模型，但必须记录降级原因

设计要求：

- router 决策必须进入 trace
- router 不能只按价格选择模型
- router 应记录选择理由、替代模型和预期成本
- router 应能基于任务结果反向更新模型画像
- 关键任务的模型降级必须符合 policy
- 同一任务可使用多模型协作，但要控制额外 token 和延迟成本

### 3. Tool Plugin

用于接入具体执行工具。

例子：

- shell command runner
- git tool
- test runner
- linter / formatter
- browser automation
- network scanner
- log parser
- vulnerability database lookup
- static analyzer

接口关注点：

- tool name
- input schema
- output schema
- permissions
- timeout
- side effects
- idempotency
- rollback support
- dry-run support
- audit level

设计要求：

- 默认最小权限
- 高风险 tool 默认需要 policy gate
- 工具输出必须可摘要、可追踪、可复现
- 工具失败必须进入效率指标和进化候选

### 4. Skill Plugin

用于沉淀可复用工作流和操作经验。

例子：

- code review skill
- test repair skill
- release check skill
- incident triage skill
- protocol parser analysis skill
- memory consolidation skill

接口关注点：

- skill name
- trigger condition
- required tools
- required context
- expected output
- verification method
- risk level
- owner
- version

设计要求：

- skill 不能只是自然语言 prompt
- skill 应声明依赖工具和验证方式
- 自动生成 skill 默认是 candidate，需要验证或人工批准后生效
- skill 使用效果应进入 efficiency metrics

### 5. MCP Adapter Plugin

用于接入 MCP server。

接口关注点：

- server identity
- transport
- exposed tools
- exposed resources
- exposed prompts
- roots
- sampling behavior
- permission scope
- startup cost
- idle cost
- health check

设计要求：

- MCP server 不应默认常驻
- MCP server 不应默认获得全局文件或网络权限
- MCP tool 描述必须防 prompt injection 和 tool poisoning
- MCP 调用必须进入 event log 和 trace
- 高成本 MCP sidecar 应支持按需启动和回收

### 6. Memory Backend Plugin

用于接入不同记忆后端。

可能实现：

- SQLite
- JSONL
- vector database
- graph store
- object store
- hybrid retrieval index

接口关注点：

- write API
- read API
- manage / consolidate API
- provenance
- confidence
- expiry / decay
- privacy / redaction
- backup / restore

设计要求：

- 原始证据不能被摘要覆盖
- 摘要必须可追溯到原始事件
- 向量检索不能作为唯一事实来源
- memory 写入必须区分 raw event、summary、pattern、policy candidate

### 7. Retrieval Plugin

用于决定 agent 当前轮次应该读取什么上下文。

接口关注点：

- query
- filters
- ranking
- context budget
- provenance
- confidence
- freshness
- deduplication

设计要求：

- retrieval 必须服务 GoalContract
- retrieval 结果应可解释
- 高风险任务不能只依赖语义相似度
- 低相关上下文应被计入效率损耗

### 8. Evaluator Plugin

用于验证结果质量。

类型包括：

- test evaluator
- static analysis evaluator
- security policy evaluator
- code review evaluator
- incident response evaluator
- regression evaluator
- human review evaluator

接口关注点：

- evaluation target
- criteria
- evidence required
- pass / fail / uncertain
- confidence
- blocking level
- remediation hint

设计要求：

- evaluator 与 generator 职责应分离
- evaluator 不是所有任务都强制开启，但高风险任务必须有验证策略
- evaluator 结果应进入 trace、audit 和 efficiency metrics

### 9. Policy Plugin

用于控制权限、风险、审批和运行模式。

接口关注点：

- action type
- risk level
- actor
- capability
- context
- decision
- reason
- required approval
- logging level

运行模式应支持：

- `autonomous_loop`
- `human_steerable_loop`
- `human_in_the_loop`
- `manual_takeover`
- `blocked`

其中旧描述可映射为：`unattended -> autonomous_loop`，`passive oversight -> human_steerable_loop`，`active approval -> human_in_the_loop`。

设计要求：

- policy decision 必须可审计
- policy plugin 不能被普通 agent 绕过
- policy 修改应有版本、diff、验证和回滚

### 10. Code Logic Index Plugin

用于帮助开发智能体理解 IDE/LSP 不容易完整表达的复杂代码关系。

能力包括：

- symbol graph
- call graph
- macro expansion mapping
- generated-code mapping
- config-to-code mapping
- protocol-field mapping
- test-to-code mapping
- error-log-to-code mapping
- cross-language FFI mapping

设计要求：

- 可以接入 LSP，但不能只依赖 LSP
- 索引结果必须带来源和置信度
- 索引更新要可增量执行
- 索引应能服务 retrieval、evaluator 和 memory

### 11. Control Plane Plugin

用于提供前台控制、审计查看、人工介入和长期运行管理。

能力包括：

- 查看 agent 状态
- 查看当前 GoalContract
- 查看 trace 和 event log
- 审批高风险动作
- 暂停 / 恢复 / 接管
- 标记 checkpoint
- 查看效率指标
- 查看 evolution proposal

设计要求：

- 控制面不应直接绕过 policy
- 人类操作必须进入 audit log
- 低风险任务可运行在 `autonomous_loop` 或 `human_steerable_loop`，高风险任务应切换到 `human_in_the_loop` 或 `manual_takeover`

## Tool Call, Function Call, and Tool Evolution

### Tool Call 与 Function Call 的关系

`tool call` 是 agent 调用外部能力的总称。

包括：

- shell command
- file read/write
- browser
- MCP tool
- HTTP API
- database query
- code index
- evaluator

`function call` 通常指模型按结构化 schema 调用一个函数或 API。

可以理解为：

```text
function call 是 tool call 的一种结构化形式
```

### ToolProfile

每个工具都应有 profile。

建议字段：

```text
tool_id
description
input_schema
output_schema
side_effect_type
required_permissions
risk_level
timeout
retry_policy
known_failure_patterns
examples
```

### ToolUsePolicy

用于决定某工具何时可调用。

建议字段：

```text
allowed_contexts
forbidden_contexts
approval_required
dry_run_supported
read_only_mode
max_retry_count
checkpoint_required
```

### ToolCallRecord

每次工具调用都应记录。

建议字段：

```text
tool_call_id
tool_id
loop_run_id
step_id
input_summary
output_summary
exit_status
side_effect_type
policy_decision
checkpoint_ref
source_refs
```

### Baseline Tool Recipes

基础、稳定、高频、跨项目通用的工具用法，不应等 agent 犯错后再学习。

`steerBox` 应在开发阶段预置 `baseline tool recipes`。

适合预置：

- shell 安全引用
- shell 特殊字符处理
- `rg` / `grep` 搜索模式
- `find` / `fd` 文件查找
- `git status` / `git diff` / `git add` / `git commit`
- JSON / YAML / TOML 校验
- Python / Node / Go / Rust 常见测试命令
- SQLite 只读查询
- HTTP 只读请求
- 文件读写安全规则
- 临时文件写入和原子替换规则
- destructive command 防护

推荐目录草案：

```text
tools/
  profiles/
    shell.yaml
    rg.yaml
    git.yaml
    curl.yaml
    sqlite.yaml
  recipes/
    shell_quoting.yaml
    rg_fixed_string_search.yaml
    git_safe_commit.yaml
    json_validation.yaml
  policies/
    destructive_commands.yaml
    file_write_safety.yaml
```

每条 recipe 建议包含：

```text
recipe_id
tool_id
use_case
safe_usage
examples
known_pitfalls
preflight_checks
postflight_checks
risk_level
scope
last_verified_at
version
```

设计原则：

- 已知稳定的基础工具知识应预置
- 项目特有工具知识进入 project recipe
- 新失败模式进入 ToolRecipeCandidate
- 高风险规则进入审批流程
- 工具版本升级后需要重新验证 baseline recipe

### Tool Evolution

工具用错后，不应只修当前轮。

推荐流程：

```text
tool_call_failed
  -> classify failure
  -> create ToolRecipeCandidate
  -> validate
  -> accept / reject
  -> update tool memory or registry
```

设计规则：

- 高频通用低风险用法可晋升到 global recipe
- 项目专用用法只能进入 project recipe
- 高风险工具策略变更必须人工审批
- MCP tool 不应被视为天然可信

## 插件注册元数据草案

每个插件至少应声明：

```text
plugin_id
plugin_type
version
provider
capabilities
model_profile
routing_profile
required_permissions
side_effects
risk_level
input_schema
output_schema
startup_mode
resource_budget
audit_level
health_check
rollback_plan
compatibility
```

## 插件生命周期

建议生命周期：

```text
discovered -> registered -> disabled -> enabled -> evaluated -> promoted -> deprecated -> removed
```

关键要求：

- 新插件默认 disabled
- enabled 需要 policy 允许
- promoted 需要有验证证据
- deprecated 不应立刻删除，先保留回滚窗口
- removed 前必须确认没有活跃任务依赖

## 插件与自动进化

自动进化可以提出插件相关变更，但不能无约束生效。

可生成的 proposal：

- 新增 tool spec
- 修改 skill trigger
- 收紧 MCP 权限
- 替换低效 model provider
- 调整 retrieval ranking
- 新增 evaluator
- 降低 sidecar 常驻成本
- 增加 code index rule

每个 proposal 必须包含：

- 触发证据
- 目标插件
- 预期收益
- 风险等级
- 验证计划
- 回滚方案
- 是否需要人工审批

## 第一阶段建议

第一阶段不需要实现完整插件市场，但应把边界设计出来。

建议先设计以下最小对象：

- `PluginManifest`
- `ModelProfile`
- `ModelRouter`
- `RoutingDecision`
- `PromptProfile`
- `TaskPromptPack`
- `Capability`
- `PermissionScope`
- `PolicyDecision`
- `PluginRegistry`
- `ToolRegistry`
- `SkillRegistry`
- `McpRegistry`
- `EvaluationResult`
- `PluginEvent`

第一阶段可以先用静态 manifest + 本地 registry，不急于做动态安装。

## 当前结论

`steerBox` 的核心不应变成一个巨大的全能 agent，而应是一个可控、可审计、可恢复、可扩展的 harness。

插件系统的目标不是“什么都能接”，而是“任何能力接入后都能被约束、观测、评估、回滚和演进”。

这也是未来支持主流思想、主流模型、主流工具、skills、MCP 和领域能力的基础。
