# Phase 1 交付状态与剩余验收包

## 定位与证据边界

状态核对日期：2026-10-11。已提交代码基线为 `8bd0860`，当前工作树还包含未提交的 Anthropic profile binding、只读工具执行、模型动作闭环、bounded runtime tick、StreamReporter 进度刷新和 model-call 分页 CLI 切片；未提交代码仍须以本文末尾的实际验证命令为准，不因文件出现或 subagent 报告自动计入已交付能力。

本文是剩余验收的归并视图，不替代 [PHASE_1_CONTRACTS.md](PHASE_1_CONTRACTS.md)、[PHASE_1_IMPLEMENTATION_SCOPE.md](PHASE_1_IMPLEMENTATION_SCOPE.md)、[PHASE_1_TODO.md](PHASE_1_TODO.md) 或 [PHASE_1_VALIDATION_PLAN.md](PHASE_1_VALIDATION_PLAN.md)。对象、身份、授权、事件及恢复语义以 Contracts 为准，范围以 Scope 为准，逐项状态和验收用例分别回查 TODO 与 Validation。

上一批全量 **188 项测试通过** 是历史证据；其中的全目标 test、clippy、build、fmt 结果不能直接覆盖当前工作树。2026-10-11 master 本轮新复跑的 focused 结果见本文末尾；工作树中另有未审核的 acceptance/benchmark 产物，因此当前不把 `--all-targets` 全量状态标为通过。本轮没有真实网络调用或真实凭据检查。

当前可运行的是只读 observation、secret-free fixture 决策、固定只读 recipe（git status、bounded `read_file`、rg fixed search、Markdown link check），以及经过 `ModelActionEnvelope → PolicyGate → ToolExecutor → verification` 的只读动作闭环。用户输入已有 bounded `RuntimeTick` 与 controller safe-boundary 接线；StreamReporter 已支持 action/progress、非法进度拒绝和逐事件 flush；model-call 审计已有独立分页 CLI。`dev markdown` 仍是调用者预先指定路径、原文、替换内容和审批标记的 Markdown 修改／链接验证切片，不是模型自行提出任意文件修改。当前不能交付给用户作为任意 coding Agent，也不能声明完整 Phase 1 已完成。

## 六个交付包

下列编号表示验收归并，不表示剩余工时、任务条数或完成百分比。

| 包 | 交付结果 | 验收层级 | 当前边界 |
|---|---|---|---|
| 1 | 真实网络 transport 与隔离 credential 配置 | 首个受限可用版本门槛 | non-stream OpenAI-compatible/Anthropic HTTP transport、注入式 credential resolver、secret-free provider profile binding、Anthropic profile binding 和 localhost 安全集成测试已通过；live 凭据与 live 调用仍未交付 |
| 2 | prompt 解析、后端绑定与实际调用审计 | 首个受限可用版本门槛 | resolved bridge 已接入 LoopController，并以 ModelCallStore、route/prompt/wire hash/output ref 做 fail-closed 审计；真实 provider、fallback 和完整恢复绑定仍未交付 |
| 3 | 模型提出受控动作，经策略、执行、验证返回模型 | 首个受限可用版本门槛 | 只读动作已连成 `ModelActionEnvelope → PolicyGate → ToolExecutor → verification` 的 controller 闭环并有 focused 覆盖；新增 bounded `read_file`，仍尚未从真实模型响应解析动作，也没有文件写入/Bash/副作用验证闭环 |
| 4 | 用户入口、preflight、状态／取消及真实开发任务 E2E | 首个受限可用版本门槛 | fixture CLI、注入式安全进度报告、逐事件 flush、bounded RuntimeTick/controller safe-boundary 和可测试的完整行命令轮询已有；真实 OS stdin、常驻 Agent 入口、live 控制及 E2E 未交付 |
| 5 | TaskRun／Attempt、目标／计划修订与 controller 恢复 | 完整 Phase 1 补齐项；fallback lineage／总预算是包 2 前置门禁 | TaskRun／Attempt 领域 schema、父关系、跨 Attempt 总预算校验以及初始 Attempt 到 controller 完成的最小绑定已通过；尚未接入 EventLog/StateStore 恢复、目标/计划修订或跨进程 resume |
| 6 | 项目范围 recipe／memory／wiki 候选与验证回链 | 完整 Phase 1 补齐项 | LessonCandidate 失败来源回链已有，其余候选链未交付 |

包 1–4 必须共同通过，才能称为“首个受限可用、连接真实模型的开发 Agent”；该版本只承诺经过验收的工具和任务范围，不承诺任意代码任务。包 5–6 和正式规范中的其余适用验收一并通过后，才可称完整 Phase 1。受限版本仍必须遵守现有 Contracts，不得以分包为理由取消授权、预算、证据或副作用边界。

自动 fallback 是跨包硬依赖：启用前必须先验收包 5 中的新 Attempt、`parent_attempt_id`、原失败证据保留及 TaskRun 总预算约束；切换不能清零既有消耗。首个受限版本缺少这部分时必须关闭自动 fallback，primary 失败按原证据进入明确失败或等待人工，不静默换模型或自动重试。禁用 fallback 不等于通过 Validation V13 的 fallback 用例，也不等于完整 Phase 1 验收通过。

## 包 1：真实网络 transport 与隔离 credential 配置

已有：

- [provider_adapter.rs](../src/provider_adapter.rs) 定义 `ProviderTransport`、OpenAI-compatible 请求／响应、超时和错误分类；[provider_model_adapter.rs](../src/provider_model_adapter.rs) 可将严格 decision/summary 响应转换为既有模型决策。
- [runtime_config.rs](../src/runtime_config.rs) 已有 model profile、endpoint 和 `auth_ref` 配置字段；fake transport 测试不需要真实认证信息。
- `RuntimeConfig::build_http_provider_adapter` 已将 profile 的 provider／endpoint／model／opaque `auth_ref` 绑定到 `ProviderModelAdapter<HttpProviderTransport<_>>`；factory 不调用 resolver、不读取环境变量、不发起网络请求。
- [http_transport.rs](../src/http_transport.rs) 已提供 non-stream OpenAI-compatible HTTP transport；credential resolver 可注入，生产默认拒绝普通 HTTP，显式禁用 redirect、自动 retry 和 Referer，并限制响应体大小。
- `tests/http_transport.rs` 的 localhost-only 测试已覆盖 credential 失败零连接、请求头／body、HTTPS/loopback policy、redirect、响应上限、timeout 和 streaming 明确拒绝。

剩余：

- 在获准隔离环境中验证真实 provider endpoint 与 credential resolver；不得把 resolver 返回的 credential 正文写入 registry、prompt、事件、trace、checkpoint、summary 或错误摘要。
- 配置绑定成功不能冒充 endpoint 可达、凭据可用或 live provider 已验收。
- 验证超时、网络失败、认证失败及非成功响应的终止和安全分类，不把上游敏感正文透传给日志。

可复现验收证据：`cargo test --locked --test config_provider_binding --test provider_model_adapter --test resolved_provider_model_adapter` 已通过 17 项（3+5+9），其中配置绑定 focused 测试为 3 项；全量 test、clippy、build、fmt 和 diff 门禁亦已由 master 复跑通过。这些测试覆盖 secret-free metadata binding、fake 请求体与安全错误分类，不能代替 live 验收。后续仍需在获准隔离环境中记录一次 live provider 请求的模型标识、结果、耗时和安全审计引用；不记录 credential 正文。

正式依据：Scope「10. 多模型接入最小垂直切片」；Contracts「十一、Secret 与敏感数据边界」；TODO P12；Validation V13。

## 包 2：prompt 真实解析、后端绑定与实际 ModelCallStore

已有：

- [prompt_registry.rs](../src/prompt_registry.rs) 提供 ID/version、role/task binding 和模板登记查询；[model_registry.rs](../src/model_registry.rs)、[model_router.rs](../src/model_router.rs) 与 controller 已有本地规则路由。
- [model_contracts.rs](../src/model_contracts.rs)、[model_call_store.rs](../src/model_call_store.rs) 提供 `RouteDecision`／`ModelCallRecord` 字段校验、独立 append-only 存储及有界 `read_page(offset, limit)` 查询。
- provider bridge 的固定输出契约位于 system；用户 objective／observation 经 JSON 编码放入 user。该分层修复不等于完整 prompt resolution 或提示注入防护。
- `ReadOnlyLoopController::run_resolved_model_step` 已将 `ResolvedPromptPack`、`ProviderModelAdapter`、`ModelCallRecord` 和 `ModelCallStore` 接成 opt-in 路径；成功、provider 失败、审计写入失败、binding mismatch 和审批阻断均有 focused 回归。

剩余：

- 本批 `PreparedProviderRequest`、`ResolvedPromptPack` resolver、opt-in provider bridge 及 controller 审计接线已通过 master focused 验收；这仍不等于真实 provider 可达、live credential 可用或 fallback 已完成。
- 按 provider／model／role／task 和适用 skill／context 层解析实际使用的 `ResolvedPromptPack`，校验后端兼容性，并验证模板确实进入最终请求，不止返回 registry 条目。
- 将实际模型调用与 prompt 版本、route、结果和验证状态继续扩展到真实 provider、attempt lineage、trace/output artifact 和验证结果；当前 opt-in 路径已覆盖 fake/localhost transport，不代表完整 coding loop。
- 完成能力兼容、非静默的 fallback；自动执行必须先通过包 5 的新 Attempt／`parent_attempt_id`／TaskRun 总预算门禁，保留原失败证据和既有消耗，再创建新 route、重新解析 prompt 和重新验证。前置门禁未通过时必须关闭自动 fallback，不能把 lineage 当作切换完成后的补记事项。

可复现验收证据：本轮已执行 `cargo test --locked --test http_transport --test model_call_controller_audit --test task_attempt_lineage --test prepared_provider_request --test resolved_prompt --test resolved_provider_model_adapter --test provider_adapter --test provider_model_adapter`，共 49 passed、0 failed；随后 `cargo test --locked --all-targets --all-features --quiet`、`cargo clippy --locked --all-targets --all-features -- -D warnings` 和 `cargo build --locked --all-targets --all-features` 均退出 0。live provider、fallback、跨进程恢复和完整 coding loop 仍未验收。

正式依据：Scope「10. 多模型接入最小垂直切片」；TODO P2／P12；Validation V13 及「最小验收命令」。

## 包 3：模型提出动作 → PolicyGate → 工具／修改 → 验证 → 下一模型步

已有：

- [model_adapter.rs](../src/model_adapter.rs) 保留旧的 continue_observation／complete_loop／request_human 与摘要兼容接口，并新增可选的 `ModelResponseWithAction`；[provider_model_adapter.rs](../src/provider_model_adapter.rs) 对 action 做严格 JSON、路径、shell 和敏感字段校验。
- [loop_controller.rs](../src/loop_controller.rs) 新增 `run_model_step_with_action`：模型响应中的有限只读 action 会经过 `PolicyGate → ReadOnlyToolExecutor → verification`，并记录 `model_action_started`／`model_action_failed`／`model_action_verified` 事件；旧 `run_model_step` 不执行 action，保持兼容。
- [loop_controller.rs](../src/loop_controller.rs) 已有模型步骤、policy 检查、固定 argv 的只读 `git_status` 执行，以及事件／trace／side-effect 记录。
- resolved action 与只读工具的持久化审计只保留安全元数据；原始 stdout、stderr、query、路径结果不再进入 EventLog、TraceStore 或 SideEffectLedger。`CommandPoller::poll_many` 和 controller 批量提交接口已限制每个 tick 的命令数，但尚未接入常驻后台 worker。
- [workspace_edit.rs](../src/workspace_edit.rs)、[main.rs](../src/main.rs) 已有受审批的 Markdown 精确修改、链接验证、checkpoint metadata 和成功／失败 RunSummary；[drift_guard.rs](../src/drift_guard.rs) 已有静态对齐／偏离检查。

剩余：

- resolved provider 的 opt-in 路径已接入 `run_resolved_model_step_with_action`：fake provider 响应经 `PolicyGate → ReadOnlyToolExecutor → verification`，最终 `ModelCallRecord` 写入实际 `tool_call_count`、`verification_status` 和错误分类；旧 resolved 入口遇到 action 会 fail-closed。真实 provider、文件写入／Bash、完整 artifact 持久化和跨进程恢复仍待完成。
- 扩展文件读／写／edit、Bash 等副作用工具；每项都必须有 PolicyGate、checkpoint／side-effect ledger、写后验证和失败恢复边界。模型说 complete 不得替代真实验证。
- 将 goal alignment／drift 检查及 ToolCallRecord／VerificationResult 的适用关联接入实际链；限制模型、工具、步骤、重试和无进展，不允许无限自循环。当前只读工具的 raw output 脱敏已接线，文件写入／Bash 的副作用验证与恢复仍待完成。

可复现验收证据：现有入口为 `cargo test --test loop_controller --test tool_policy --test tool_executor --test workspace_edit --test dev_vertical_slice --test drift_guard`。新增受控闭环验收须让模型提出一次限定文件修改，验证器检查实际产物后将结果回传，最终 summary 可反查各步骤、policy、工具、diff boundary、验证及副作用。另覆盖越界／未注册／未审批的零副作用路径、验证失败后不伪报成功和预算耗尽停止。现有手工指定 Markdown 替换输入不能作为模型提出动作的证据。

正式依据：Contracts「八、Verification、Artifact 与 Evidence」「九、错误、失败和恢复」；Scope「2. 软件开发智能体垂直切片」及「第一阶段成功标准」；TODO P4–P10／P13；Validation V3／V6–V10／V14。

## 包 4：用户可运行入口、preflight、状态／取消与真实开发任务 E2E

已有：

- [main.rs](../src/main.rs) 提供 runtime init、config validate、event、execution list/show/page、board、loop readonly/fixture 和 dev markdown；没有已交付的真实模型开发任务入口。
- [stream_reporter.rs](../src/stream_reporter.rs) 已接入 readonly／fixture CLI 的生命周期反馈；[execution_query.rs](../src/execution_query.rs) 是 metadata-only detached 历史投影，不是后台 live worker。
- inbox/controller 已支持 pause／resume／cancel 安全边界应用；`ReadCommandReader` 和 Unix `UnixCommandReader` 已把非阻塞 `Read`／stdin/pipe 接到 `CommandPoller` 的 reader 边界，并保留注入式测试路径，但尚未接到常驻后台 loop、同步模型调用期间轮询或 CLI live control。

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

- 补齐单 Portfolio／Project／Task／TaskRun／Attempt 及依赖身份；当前已具备 TaskRun／Attempt 最小 schema、初始 Attempt controller 绑定及完成／失败状态写回；retry／fallback／resume／fork 的 controller 创建新 Attempt、依赖身份和完整 evidence 链仍待实现。
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

2026-10-11 master 复跑记录：本轮合并 focused 命令中 `loop_controller` 28 项、`user_command_inbox` 22 项、`stream_reporter` 7 项、`tool_executor` 9 项、`runtime_cli` 9 项通过，共 75 项；本轮覆盖 bounded RuntimeTick/controller safe-boundary、model-call 分页 CLI、StreamReporter action/progress/flush、bounded `read_file`，以及既有 resolved provider 动作闭环和 raw tool output 脱敏。随后同范围 clippy `-D warnings`、rustfmt 和 `git diff --check` 均通过。构建产物位于 `/tmp/steerbox-phase1-target`，未写入项目目录。工作树中另有未审核的 `tests/acceptance/` 与 `tests/benchmarks/` 产物，不能据此声称当前 `--all-targets` 全量通过；真实 provider、真实任务、文件写入／Bash、常驻后台 worker、跨进程恢复仍未验收。

每包交接记录应给出代码基线、实际执行命令、结果／退出码、产物位置与证据引用、已验证范围及未通过项。全量测试通过是回归证据，不能替代真实 provider、真实任务或跨进程恢复的专门验收。

这六包不新增 SQLite 迁移、多 agent、TUI/Web、分布式 worker、向量／图数据库或无审批自动进化的交付要求。现有 JSONL／JSON 切片应先闭合规范要求，存储迁移不能替代真实模型闭环。CCG 和其他参考项目仅提供可核查的设计线索，不作为已证明最优的性能 benchmark，也不改变 Phase 1 正式范围。
