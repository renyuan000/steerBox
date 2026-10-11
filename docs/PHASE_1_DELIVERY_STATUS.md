# Phase 1 交付状态与剩余验收包

## 定位与证据边界

状态核对日期：2026-10-11。代码基线仍为 `a084bee`；本轮未提交的新代码须以本文末尾的实际验证命令为准，不因文件出现或 subagent 报告自动计入已交付能力。

本文是剩余验收的归并视图，不替代 [PHASE_1_CONTRACTS.md](PHASE_1_CONTRACTS.md)、[PHASE_1_IMPLEMENTATION_SCOPE.md](PHASE_1_IMPLEMENTATION_SCOPE.md)、[PHASE_1_TODO.md](PHASE_1_TODO.md) 或 [PHASE_1_VALIDATION_PLAN.md](PHASE_1_VALIDATION_PLAN.md)。对象、身份、授权、事件及恢复语义以 Contracts 为准，范围以 Scope 为准，逐项状态和验收用例分别回查 TODO 与 Validation。

上一批全量 **188 项测试通过** 是历史证据；本轮新增的静态 prompt、冻结请求和 resolved bridge 已由 master 独立定向复跑 **44 项通过**（6+7+5+17+9），但这不是全量测试，也不能证明下述六包已完成。本轮没有真实网络调用或真实凭据检查。

当前可运行的是只读 observation、secret-free fixture 决策，以及调用者预先指定路径、原文、替换内容和审批标记的 Markdown 修改／链接验证切片。`dev markdown` 确实修改文件并产出验证与审计记录，但不是模型自行提出修改。当前不能交付给用户作为任意 coding Agent，也不能声明完整 Phase 1 已完成。

## 六个交付包

下列编号表示验收归并，不表示剩余工时、任务条数或完成百分比。

| 包 | 交付结果 | 验收层级 | 当前边界 |
|---|---|---|---|
| 1 | 真实网络 transport 与隔离 credential 配置 | 首个受限可用版本门槛 | adapter/fake transport 已有，真实网络与凭据解析未交付 |
| 2 | prompt 解析、后端绑定与实际调用审计 | 首个受限可用版本门槛 | prepared 请求、resolver 与 opt-in resolved bridge 已通过 fake 定向验收；LoopController 的实际 ModelCallStore 审计、真实 provider 和 fallback 仍未交付 |
| 3 | 模型提出受控动作，经策略、执行、验证返回模型 | 首个受限可用版本门槛 | 决策步、只读工具和预定义 Markdown 修改尚未连成模型动作闭环 |
| 4 | 用户入口、preflight、状态／取消及真实开发任务 E2E | 首个受限可用版本门槛 | fixture CLI 与历史查询已有；真实 Agent 入口及 live 控制未交付 |
| 5 | TaskRun／Attempt、目标／计划修订与 controller 恢复 | 完整 Phase 1 补齐项；fallback lineage／总预算是包 2 前置门禁 | 状态投影 reopen 已有，不等于恢复运行 |
| 6 | 项目范围 recipe／memory／wiki 候选与验证回链 | 完整 Phase 1 补齐项 | LessonCandidate 失败来源回链已有，其余候选链未交付 |

包 1–4 必须共同通过，才能称为“首个受限可用、连接真实模型的开发 Agent”；该版本只承诺经过验收的工具和任务范围，不承诺任意代码任务。包 5–6 和正式规范中的其余适用验收一并通过后，才可称完整 Phase 1。受限版本仍必须遵守现有 Contracts，不得以分包为理由取消授权、预算、证据或副作用边界。

自动 fallback 是跨包硬依赖：启用前必须先验收包 5 中的新 Attempt、`parent_attempt_id`、原失败证据保留及 TaskRun 总预算约束；切换不能清零既有消耗。首个受限版本缺少这部分时必须关闭自动 fallback，primary 失败按原证据进入明确失败或等待人工，不静默换模型或自动重试。禁用 fallback 不等于通过 Validation V13 的 fallback 用例，也不等于完整 Phase 1 验收通过。

## 包 1：真实网络 transport 与隔离 credential 配置

已有：

- [provider_adapter.rs](../src/provider_adapter.rs) 定义 `ProviderTransport`、OpenAI-compatible 请求／响应、超时和错误分类；[provider_model_adapter.rs](../src/provider_model_adapter.rs) 可将严格 decision/summary 响应转换为既有模型决策。
- [runtime_config.rs](../src/runtime_config.rs) 已有 model profile、endpoint 和 `auth_ref` 配置字段；fake transport 测试不需要真实认证信息。

剩余：

- 实现至少一个真实网络 transport，在调用边界解析隔离的 credential 引用；不得在 registry、prompt、事件、trace、checkpoint、summary 或错误摘要保存认证正文。
- 将配置中的 provider／endpoint／model 与真实 adapter 建立可执行绑定；配置有效不能冒充 endpoint 可达或凭据可用。
- 验证超时、网络失败、认证失败及非成功响应的终止和安全分类，不把上游敏感正文透传给日志。

可复现验收证据：现有隔离回归入口为 `cargo test --test provider_adapter --test provider_model_adapter`，覆盖 fake 请求体与安全错误分类，不能代替 live 验收。补充本地假 HTTP 服务的真实 transport 测试，并在获准的隔离环境下记录一次 live provider 请求的模型标识、结果、耗时和安全审计引用；不记录 credential 正文。新的 live 命令须在入口实现后写入使用说明，本文不虚构可执行命令。

正式依据：Scope「10. 多模型接入最小垂直切片」；Contracts「十一、Secret 与敏感数据边界」；TODO P12；Validation V13。

## 包 2：prompt 真实解析、后端绑定与实际 ModelCallStore

已有：

- [prompt_registry.rs](../src/prompt_registry.rs) 提供 ID/version、role/task binding 和模板登记查询；[model_registry.rs](../src/model_registry.rs)、[model_router.rs](../src/model_router.rs) 与 controller 已有本地规则路由。
- [model_contracts.rs](../src/model_contracts.rs)、[model_call_store.rs](../src/model_call_store.rs) 提供 `RouteDecision`／`ModelCallRecord` 字段校验及独立 append-only 存储。
- provider bridge 的固定输出契约位于 system；用户 objective／observation 经 JSON 编码放入 user。该分层修复不等于完整 prompt resolution 或提示注入防护。

剩余：

- 本批 `PreparedProviderRequest`、`ResolvedPromptPack` resolver 与 opt-in provider bridge 已通过 master 定向 fake 验收；这只证明冻结请求、静态解析和实际 wire 绑定，不证明 controller 的完整 prompt／实际调用审计链。不得把独立模块测试或测试作者自报等同于 ModelCallStore 已接线。
- 按 provider／model／role／task 和适用 skill／context 层解析实际使用的 `ResolvedPromptPack`，校验后端兼容性，并验证模板确实进入最终请求，不止返回 registry 条目。
- 将实际模型调用与 prompt 版本、route、结果和验证状态写入 `ModelCallStore`，并能经 event／trace 反查；当前独立 store 的测试写入不是 controller 的实际调用审计。
- 完成能力兼容、非静默的 fallback；自动执行必须先通过包 5 的新 Attempt／`parent_attempt_id`／TaskRun 总预算门禁，保留原失败证据和既有消耗，再创建新 route、重新解析 prompt 和重新验证。前置门禁未通过时必须关闭自动 fallback，不能把 lineage 当作切换完成后的补记事项。

可复现验收证据：本轮已执行 `CARGO_TARGET_DIR=/tmp/steerbox-phase1-target CARGO_BUILD_JOBS=2 cargo test --locked --test prepared_provider_request --test resolved_prompt --test resolved_provider_model_adapter --test provider_adapter --test provider_model_adapter`，结果 44 passed、0 failed。下一步仍需捕获 controller 的最终调用并从落盘记录反查同一次调用的 provider、endpoint、model、prompt ID/version、route 和结果；不匹配、缺失 binding、审批未解析及审计写入失败应有明确的拒绝或失败记录。fallback 验收对应 Validation V13，并须同时验证包 5 的 Attempt 父关系、原失败证据和 TaskRun 总预算；缺少这些条件时验证自动 fallback 被阻断，不能仅展示新 schema 或把阻断用例替代成功切换验收。

正式依据：Scope「10. 多模型接入最小垂直切片」；TODO P2／P12；Validation V13 及「最小验收命令」。

## 包 3：模型提出动作 → PolicyGate → 工具／修改 → 验证 → 下一模型步

已有：

- [model_adapter.rs](../src/model_adapter.rs) 的响应只有 continue_observation／complete_loop／request_human 与摘要，不包含可执行动作提案。
- [loop_controller.rs](../src/loop_controller.rs) 已有模型步骤、policy 检查、固定 argv 的只读 `git_status` 执行，以及事件／trace／side-effect 记录。
- [workspace_edit.rs](../src/workspace_edit.rs)、[main.rs](../src/main.rs) 已有受审批的 Markdown 精确修改、链接验证、checkpoint metadata 和成功／失败 RunSummary；[drift_guard.rs](../src/drift_guard.rs) 已有静态对齐／偏离检查。

剩余：

- 形成结构化、可校验的有限动作提案，将模型输出与执行授权分离；未知工具、非法参数、越界路径及未经审批的动作必须在副作用发生前阻断。
- 把经 PolicyGate 允许的有限工具／修改、checkpoint／side-effect boundary、验证结果和失败原因连回下一模型步；模型说 complete 不得替代真实验证。
- 将 goal alignment／drift 检查及 ToolCallRecord／VerificationResult 的适用关联接入实际链；限制模型、工具、步骤、重试和无进展，不允许无限自循环。

可复现验收证据：现有入口为 `cargo test --test loop_controller --test tool_policy --test tool_executor --test workspace_edit --test dev_vertical_slice --test drift_guard`。新增受控闭环验收须让模型提出一次限定文件修改，验证器检查实际产物后将结果回传，最终 summary 可反查各步骤、policy、工具、diff boundary、验证及副作用。另覆盖越界／未注册／未审批的零副作用路径、验证失败后不伪报成功和预算耗尽停止。现有手工指定 Markdown 替换输入不能作为模型提出动作的证据。

正式依据：Contracts「八、Verification、Artifact 与 Evidence」「九、错误、失败和恢复」；Scope「2. 软件开发智能体垂直切片」及「第一阶段成功标准」；TODO P4–P10／P13；Validation V3／V6–V10／V14。

## 包 4：用户可运行入口、preflight、状态／取消与真实开发任务 E2E

已有：

- [main.rs](../src/main.rs) 提供 runtime init、config validate、event、execution list/show/page、board、loop readonly/fixture 和 dev markdown；没有已交付的真实模型开发任务入口。
- [stream_reporter.rs](../src/stream_reporter.rs) 已接入 readonly／fixture CLI 的生命周期反馈；[execution_query.rs](../src/execution_query.rs) 是 metadata-only detached 历史投影，不是后台 live worker。
- inbox/controller 已支持 pause／resume／cancel 安全边界应用，但同步模型调用期间不会读取命令，CLI 尚未提供可连接运行任务的控制入口。

剩余：

- 提供实际可运行的受限 Agent CLI 和无认证正文的配置示例；preflight 核对 workspace／scope、model binding、credential 引用可用性、工具／权限及运行数据路径，在必要条件不足时拒绝启动动作。
- 用户可查运行状态并提交取消，返回值须区分已排队、待安全边界、已应用和副作用不确定；取消不能被误报为立即完成或清零预算。
- 接通内部工具／模型阶段的即时反馈；当前 provider `stream()` 取得完整正文后解析事件，不应称为网络增量 streaming。响应灵敏度通过实测记录反馈和取消延迟，不虚构已达到的性能值。
- 用包 1–3 的真实 provider／prompt／受控动作运行至少一个低风险真实开发任务，并保留成功及失败／取消路径的可复盘证据。

可复现验收证据：现有入口为 `cargo test --test runtime_cli --test execution_query --test stream_reporter --test user_command_inbox`，证明现有切片边界。最终验收必须提供用户能直接复跑的配置／preflight／run／status／cancel 命令、退出码与产物位置，并反查 event、state、trace、checkpoint、side-effect、route、prompt、model call 与 summary。命令在实现后确定；`loop fixture`、历史查询及预定义 `dev markdown` 不等于真实 Agent E2E。

正式依据：Scope「当前可执行切片」「12. User steering 与有限 Loop」；TODO P6／P13／P14 及「完成定义」；Validation V11／V13／V14／V18／V19 及「验证通过标准」。

## 包 5：TaskRun／Attempt、目标／计划修订与 controller 恢复

已有：

- [domain.rs](../src/domain.rs) 已有 Task 状态迁移、GoalContract、LoopRun、LoopBudget 和 NoProgressTracker；[board_projection.rs](../src/board_projection.rs) 可重建最小只读 board。
- [user_command_inbox.rs](../src/user_command_inbox.rs) 提供持久化、command_id 幂等和冲突拒绝；controller 目前应用 pause／resume／cancel，其余命令未实现的分支会拒绝。
- [state_store.rs](../src/state_store.rs) 可重建／持久化／reopen loop/step projection；[audit_store.rs](../src/audit_store.rs) 的 checkpoint 是覆盖范围 metadata，不是完整 controller/process snapshot。

剩余：

- 补齐单 Portfolio／Project／Task／TaskRun／Attempt 及依赖身份；retry／fallback／resume／fork 创建新 Attempt，并保留原 evidence、父关系及 TaskRun 总预算。
- 上述 fallback lineage／总预算子集是包 2 自动 fallback 的先行依赖，即使包 5 的其余修订／恢复能力尚未交付，也不能推迟此门禁或通过重新创建 controller 补额度。验收未通过前首个受限版本必须关闭自动 fallback。
- 相关 steer 创建 GoalRevision，使旧 PlanRevision 失效后重新规划；无关请求进入独立 Task。补齐 object_version 冲突、审批／接管及 InterruptRequest 的 side_effect_uncertain 分支。
- 从事件和已记录边界恢复 controller 的目标、计划、策略修订、预算消耗及无进展历史；执行恢复前协调 workspace／checkpoint／外部副作用，无法确认时等待人工，不自动重放。

可复现验收证据：现有入口为 `cargo test --test domain_contracts --test board_projection --test user_command_inbox --test state_store --test loop_strategy_controls --test strategy_audit_failure`。新增验收需中断／重启进程后创建正确的新 Attempt，确认目标和计划版本、已耗预算、重复动作历史和已执行副作用未被清零或重做；同一 command 重试不产生第二次修改，版本冲突拒绝。StateStore reopen、同实例恢复审计路径或 start_with_strategy 新建运行均不能替代这项验收。

正式依据：Contracts「四、核心对象关系」「七、命令、幂等与并发」「九、错误、失败和恢复」；Scope「11. Portfolio、Task 与 BoardProjection 最小切片」「12. User steering 与有限 Loop」；TODO P2／P3／P6／P7／P12.1；Validation V2.1／V7／V11／V15–V17／V20。

## 包 6：项目范围 recipe／memory／wiki 候选与验证回链

已有：

- [tool_registry.rs](../src/tool_registry.rs) 已保留 `recipe_ref`；controller 的失败事件内嵌 LessonCandidate，其 source_event_ids 精确引用实际持久化失败事件，已有回归。
- [MEMORY_SCOPE_AND_LIFECYCLE_CONTRACTS.md](MEMORY_SCOPE_AND_LIFECYCLE_CONTRACTS.md)、[RAG_WIKI_KNOWLEDGE_COMPILATION.md](RAG_WIKI_KNOWLEDGE_COMPILATION.md) 已规定 scope、来源、生命周期、promotion 和派生知识边界；设计存在不等于运行时已实现。

剩余：

- 实现 baseline recipes 及 `ToolUseErrorEvent → LessonCandidate → verified local recipe` 的候选链；tool call 能关联 recipe ID，提升有验证、版本和回滚引用，不自动修改全局 policy。
- 实现 run WorkingMemory／project MemoryCandidate 的最小记录、provenance／confidence／lifecycle、source refs 与确定性 metadata retrieval；先执行 PolicyGate 和项目可见性过滤，写入和 promotion decision 可审计。
- 实现项目范围 EvidencePack／WikiPageCandidate／WikiRevision、验证回链、expiry／revalidation 与低风险项目页发布；高风险、global、cross-project 只产生候选，不自动提升。仅创建候选 schema 不等于完整 wiki 验收。

可复现验收证据：现有 `cargo test --test loop_controller --test tool_policy` 只能复核失败来源和 registry 边界。新增最小验收需从一次失败找到原 tool/event，生成并验证 local recipe；从验证结果形成项目 memory/wiki 候选，反查 source 和 EvidencePack；不同 project 检索不得泄漏，失效页面不得直接支撑高风险动作，promotion 必须有事件。实现后登记专属回归命令；当前不存在可称为 memory/wiki 完整通过的测试入口。

正式依据：Contracts「十三、Memory Contract 边界」；Scope「9. Baseline Tool Recipes」；TODO P3 Memory／Wiki 与 P11；Validation V12；上述 Memory 与 RAG/Wiki 补充规范。

## 验收记录与范围保护

本轮列出的 `cargo test` 命令均为现有回归的复跑入口，**本轮尚未执行**，不等于各交付包已验收。获准复跑时将构建产物放在 `/tmp/steerbox-phase1-target`，使用 `CARGO_TARGET_DIR=/tmp/steerbox-phase1-target CARGO_BUILD_JOBS=2`，不要将构建或运行产物加入源码提交。

每包交接记录应给出代码基线、实际执行命令、结果／退出码、产物位置与证据引用、已验证范围及未通过项。全量测试通过是回归证据，不能替代真实 provider、真实任务或跨进程恢复的专门验收。

这六包不新增 SQLite 迁移、多 agent、TUI/Web、分布式 worker、向量／图数据库或无审批自动进化的交付要求。现有 JSONL／JSON 切片应先闭合规范要求，存储迁移不能替代真实模型闭环。CCG 和其他参考项目仅提供可核查的设计线索，不作为已证明最优的性能 benchmark，也不改变 Phase 1 正式范围。
