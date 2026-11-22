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

## 当前收口依赖顺序（2026-11-22）

RDI2（基线 `73886fe`）显式 tool/side-effect 日志观察已实现，隔离树全量1219项及严格
Clippy/build/fmt/diff通过，三路独立审核无剩余阻断；提交/合入复验见收据，细项见验证计划。
新增可选文件的有界、零写入独立扫描，固定 status/class 计数与异常位置；
这不是请求/工具/副作用/快照对账，也不推断缺失终态或允许续跑。
后续仍须补 snapshot 存储/关联审查、完整对账、持久恢复输入及 fenced 恢复；
获准 live 验收仍是独立门禁。合同见 `RECOVERY_AUDIT_OBSERVATIONS.md`，Phase 1 未完成。

Workspace editor W1（基线 `c7df098`）限定 Unix 普通文件替换切片已验证：
source FD读/权限及同pinned parent的exclusive temp、文件同步、发布、清理和目录同步；
保留审批/expected/noop/PID-temp语义，发布后同步失败不回滚、不自动重试。
最终全量87个harness/1201项及严格Clippy/build/fmt/diff通过，独立审核无剩余阻断。
合同与边界见 `WORKSPACE_EDIT_IO.md`，提交/合入复验见外置W1收据；不表示snapshot存储/关联、
未获准A1专项、跨store对账、持久恢复输入、授权续跑或live验收完成，Phase 1未完成。

StateStore S1（基线 `ac0070c`）限定 Unix 普通文件切片已通过独立审核与全量验证：
安全 no-create load，同 pinned parent 的 exclusive temp/完整写入/文件同步/rename/目录同步；
普通入口10项、实际FD6项、共享helper15项通过，全量86个harness/1182项，严格Clippy/build/fmt/diff通过。
平台、诊断及发布后同步失败边界见 `STATE_STORE_IO.md`，提交/合入复验见外置 S1 收据。
workspace实际替换端限定W1范围见上方；后续仍须完成持久恢复输入与请求/工具/副作用/快照对账；
A1 未获准专项及 live provider/live mutation 门禁不由本片解除，不能把 State 投影持久化当完整续跑。

Phase 1 尚未完成。实际 runtime I/O 继续逐片验收；零写入检查不依赖全部写端完成。
RDI1 已实现事件/State 投影诊断入口，RDI2 补显式工具/副作用日志的逐记录观察；
两者均不包含请求/工具/副作用/快照对账。
实际修复与续跑仍须先交付安全写端、持久恢复输入及对账协调；supervisor/fencing
及故障演练在合法恢复判定之后，不能拿只读检查成功当恢复许可。
checkpoint/side-effect/trace 共享 JSONL 片已验收；后续 EventLog 五入口安全 I/O
片已实现并通过专项及全量测试，审核/提交状态见交付收据。
ModelCallStore 成败 JSONL 及两实际锁的 M1 已通过独立审核与全量验证（1061 项）；
最终提交/合入复验状态见交付收据。
ToolCallRecordStore T1 已通过独立审核及全量1079项、严格Clippy/build/fmt；提交/合入状态见交付收据。
RunSummary R1 限定切片已验收（基线 `d78fa6f`）：79个harness/1097项、严格Clippy/build/fmt/diff通过；提交/合入状态详见交付收据。
Inbox I1（基线 `d271f7c`）限定切片已验收：80个harness/1122项及严格Clippy/build/fmt/diff通过；提交/合入见交付收据。
RDI1（基线 `75cdb2e`）：`recovery inspect` 有界安全读取、坏尾/schema/序号诊断、
内存 loop/step 重建及可选 State 比较已实现；无修复/续跑，验证/审核见 `PHASE_1_VALIDATION_PLAN.md`
及外置 `recovery-inspect-20261122/RECEIPT.md`。独立审核、全量82个harness/1148项、
严格Clippy/build/fmt/diff通过。后续补请求/工具/副作用/快照对账与完整恢复输入。
ApprovalStore A1（基线 `ee87518`）限定 I/O 增量已实现，验证范围受限；
特殊路径/FIFO/真实降权权限专项仍未获准验收，不能勾选为完整收口。
实际结果见 `APPROVAL_STORE_IO.md`、验证计划及外置 A1 收据。
不代表审批完整专项、其余真实锁文件或 State/workspace 替换端已完成验收，
也不放开 live 写 CLI。具体检查点见 `CRASH_RECOVERY_BOUNDARIES.md`。

### 既有交付任务包（2026-10-31）

下列编号用于验收归并，不表示执行顺序、剩余工时或完成百分比；
不是“仅剩六个功能”，各包仍须逐项验收，不能替代上述安全与恢复依赖。

| 编号 | 任务包 | 收口检查点 |
|---|---|---|
| 1 | 模型连接型开发与自适应审计 | 显式 opt-in 单次相邻升档的库入口与纯观察 foreground 接线已实现；安全拒绝不重调、预算不重置；后续补齐统一测量、预留式硬上限、金额约束、生产调度入口和获准 live 验收 |
| 2 | Task 命令与投影 | 合法迁移、版本冲突、幂等及 retry/resume/fork 父链；BoardProjection 可由事实事件重建 |
| 3 | 最小恢复协调 | 独立进程恢复 TaskRun/Attempt/位置与累计预算；不重放已完成 mutation；未知副作用阻断，文件 restore 不代替任务恢复 |
| 4 | 人类接管与 revision | takeover 后停止自动执行；用户纠偏使旧 plan 失效；暂停、恢复和新 revision 不补预算、不解除审批 |
| 5 | 目标对齐、漂移与无进展 | 真实 step 关联决策/事件；重复动作、计划及失败不能借新 Attempt 绕过；每条路径有完成、失败或停止结果 |
| 6 | 失败工具到 recipe candidate | 关联真实 tool call/source event，生成可审查但不自动生效的候选；完成 baseline recipe 与完整 summary/evidence 反查验收 |

最终仍须整体复核 V0–V17 及已纳入的自适应门禁，不能把单包测试通过称为 Phase 1 完成。通用文件/Bash 能力仍按下方原待办推进，不用 bounded Markdown 冒充通用工具执行。

后续新增的前台低价值上下文/compact 摘要投影清理，见 `MODEL_PROMPT_AND_CHECKPOINT_POLICY.md` §7.1；开发备份 provenance/审核后清理，见 `DEVELOPMENT_BACKUP_POLICY.md`。两者已记录但自动工具尚未实现，不替换当前主线，也不增加本阶段门槛。

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

- [x] 选择第一阶段主语言和构建工具（Rust + Cargo）
- [x] 创建最小源码目录
- [x] 创建最小测试目录
- [x] 创建最小 CLI 或 runtime 入口
- [x] 添加基础格式化 / test / build 命令（当前提供 fmt、test、build）

验收：

- 能运行 `build` 或等价命令
- 能运行最小测试
- README 或开发说明中记录命令

### P2. Core Schema

- [x] 定义 EventEnvelope、event_id、sequence_no、correlation_id 和 causation_id
- [ ] 定义 Portfolio、Project、Task 和 TaskDependency
- [x] 定义 TaskRun／Attempt 的最小领域 schema、父关系和跨 Attempt 总预算校验（初始 Attempt、routed fixture/model-adapter 与 resolved-provider local contract fallback 的子 Attempt、父链、累计预算与事件证据已接入 controller；retry/resume/fork、resolved-provider fallback 的生产接线与 live 验收、持久化恢复和完整依赖投影仍待完成）
- [x] 定义 Task 状态及合法迁移表
- [x] 定义最小 BoardProjection / BoardTaskCard
- [x] 定义 ToolUseErrorEvent、LessonCandidate 和最小 FailureCategory（schema 与最小 `tool_call_failed` 关联事件已有测试；ToolCallRecordStore 已以 opt-in metadata-only 方式接入只读工具 started/completed/failed 生命周期，并对分页 limit 施加 1024 硬上限；Unix store 已有 owner-only 权限、`O_NOFOLLOW` 和 advisory lock 边界；CLI 已提供只读 `tool-call page` 分页查询；生产默认运行接线和通用工具面仍待完成）

- [x] 定义 `GoalContract`
- [x] 定义 `LoopRun`
- [x] 定义 `LoopStep`
- [x] 定义 `ToolCallRecord`（schema、独立 store 及只读工具与 bounded workspace-edit 的 started/completed/failed runtime 接线已有 focused 测试；分页 limit 有 1024 硬上限；Unix store 的 owner-only、`O_NOFOLLOW`、advisory lock 与并发追加回归已通过；只读工具以 `tool_call_requested`、workspace-edit 以 `model_action_started` 的 EventLog sequence 生成运行内 `tool_call_id`，同一次请求生命周期复用同一 ID，重复调用获得不同 ID，mutation 与 verification identity 分离；identity 字段使用现有安全 ID 字符集和长度上限，并在请求事件前 fail-closed；CLI 已提供 metadata-only `tool-call page` 分页查询；该 identity 不代表跨进程全局唯一或完整恢复；生产默认配置及通用工具面仍待完成）
- [x] 定义 `PolicyDecision`（`PolicyGate` 已提供 Allow/Warn/RequireCheckpoint/RequireHumanApproval/Block 枚举、PolicyRequest/PolicyResult 及 controller 接线；完整跨存储恢复语义仍待完成）
- [ ] 定义 `SideEffectEvent`
- [x] 定义 `CheckpointMetadata`
- [ ] 定义 `VerificationResult`
- [x] 定义 `TraceRecord`
- [x] 定义 `ProviderProfile`、`EndpointProfile`、`ModelProfile`（已有注册/序列化及关联验证）
- [x] 定义并测试 `PromptProfile`、`AgentRolePrompt`、`TaskPromptPack`、`ResolvedPromptPack`（静态 resolver 已验证 registry/provider/endpoint/model/role/task/version 绑定、审批阻断、低信任 user JSON 分层；resolved controller 成功调用的审计路径已有 focused 验收）
- [x] 定义 `RouteDecision`、成功 `ModelCallRecord` 和独立 `ModelCallFailureRecord`（resolved controller 成功调用与 adapter-error 分支分别写入成功/failure JSONL；失败记录不保存 provider 原始错误正文，不伪造未知 response facts；routed fixture/model-adapter 与 resolved-provider local contract fallback 的 Attempt lineage 已接线，live provider、resolved-provider fallback 的生产接线、所有失败路径覆盖和恢复仍待完成）

验收：

- schema 有单元测试或序列化测试
- 字段能覆盖第一阶段端到端流程
- schema 有版本字段或预留版本策略

### P3. Local Storage

- [x] 私有 transcript 存储库基础：fresh-session JSONL、逐条写入、默认 1s/64KiB 批量同步、关键正文同步后 ACK、容量/敏感信息门禁及进程 SIGKILL 回归；仅库接口，详见 `PRIVATE_TRANSCRIPT_STORE.md`
- [ ] 安全 transcript 运行时接线：获准正文来源、整流敏感检查、受保护 reader/引用、轮转和 opt-in provider/CLI 采集；目前不会自动保存实际聊天或模型正文
- [x] EventLog 重开拒绝未换行封口的尾行，保留异常历史原字节；合成坏尾/UTF-8/CRLF 及已确认 append 后 SIGKILL 重开回归通过。仅检测拒绝，不自动修复、不证明 OS 掉电恢复，见 `CRASH_RECOVERY_BOUNDARIES.md`
- [x] 2026-11-12 审计文件同步片：checkpoint/ledger/trace 共用追加路径在完整记录及换行写入后，同一文件 `sync_data` 成功才返回；同步失败传播并阻止写前编辑，定向回归通过。该片本身不包含目录/路径加固、跨 store 事务或完整恢复，详见 `CRASH_RECOVERY_BOUNDARIES.md`
- [x] checkpoint/ledger/trace 共享 JSONL 安全 I/O 片：逐组件 NOFOLLOW、dirfd-relative 打开，同 fd 验证 regular/nlink=1；每次追加写前同步遍历目录链（含既有目录），完整追加后文件 `sync_data` 再父目录同步。缺失读取不创建，拒绝 symlink/hardlink/device/FIFO/`..`/无 basename 路径，非 Unix fail-closed；每次访问独立 pin，不是生命周期绑定。Linux 模块13/集成54/全量987通过；目录同步性能成本未测，详见 `CRASH_RECOVERY_BOUNDARIES.md`
- [x] EventLog 五入口安全 I/O（基线 `76f31dc`）：安全目录 fd、同 fd regular/nlink 校验，缺失查询不创建；完整追加后文件及父目录同步，再推进序号/ACK；保留只读历史、UTF-8/尾行、identity 全扫描及分页短路。定向209/全量1022与严格Clippy/build/fmt通过；失同步不自动重试，MAX写后溢出仅记录未修复，详见 `CRASH_RECOVERY_BOUNDARIES.md`
- [x] ModelCallStore M1（基线 `0a1ca50`）：成败 JSONL 与两实际锁安全 I/O 已通过独立审核、全量1061项与严格Clippy/build/fmt；同 parent 持锁至 file/parent ACK，identical retry 同步原 history read fd、不改字节。open 准备整链，操作仅最终 parent；查询仍建 RW lock，无尾修/事务/完整恢复，不防 sidecar 替换，锁无 timeout，非 Unix fail-closed，性能未测；详见交付收据
- [x] ToolCallRecordStore T1（基线 `4fe81d9`）：tool JSONL/真实锁安全 I/O、实际fd owner/mode与持锁file/parent ACK已通过独立审核、全量1079项及严格Clippy/build/fmt；metadata-only构造不构成anchored身份保证，O_WRONLY变化已明确，不扫历史/不去重，无尾修/事务/fencing/完整恢复；提交/合入状态见交付收据
- [x] RunSummary R1（基线 `d78fa6f`）：普通JSON安全原位覆盖/读取限定切片已验收，79个harness/1097项及严格Clippy/build/fmt/diff通过；保留inode/owner/mode、pretty无LF、缺失Io与InvalidJson line0，不做UID/owner-only拒绝；验证fd后才截断，file/parent同步后ACK。非atomic/无锁/重试/恢复，parent可读与整链同步成本边界已记录；提交/合入及后续复验状态见交付收据
- [x] UserCommandInbox I1（基线 `d271f7c`）：安全读取/完整JSON+LF追加、同fd与pinned parent ACK；独立审核及全量1122项、严格Clippy/build/fmt/diff通过，提交/合入见交付收据；不新增锁/并发原子判重/尾修，同intent公开重试不重新sync，不保证Applied事务或完整恢复
- [ ] 补齐其余实际 runtime I/O：审批 A1 仅完成限定实现/验证范围，扩展特殊路径、FIFO 与真实降权权限专项仍未验收；其余真实锁及 State/workspace 替换端仍须独立验收，各已交付切片不能单独解除 live 写 CLI 的安全阻断
- [ ] 只读崩溃恢复检查、持久 TaskRun/Attempt/预算/授权输入与对账式受控续跑；不以 inbox Resume、顺序 reopen 或显式单文件 restore 冒充进程重启恢复
- [x] 实现从 EventLog 重建 loop/step current state 的最小 StateStore fixture
- [x] 实现 StateStore JSON 持久化与 reopen 恢复测试
- [x] 从 EventLog 重建 BoardProjection 的最小 fixture（只读 CLI 输出；完整依赖/Attempt 投影仍待实现）

- [ ] 初始化 `events.sqlite`
- [ ] 初始化 `state.sqlite`
- [x] 实现 append-only event write（JSONL 第一切片）
- [ ] 实现 current state update
- [x] 实现 TraceRecord JSONL append/read 最小切片
- [x] 实现 run summary 写入（dev markdown 成功/失败两条路径均写入 `runs/<loop_run_id>/summary.json`）

- [x] 实现 RuntimeConfig JSON 解析、`/temp/steerbox` 路径校验和未知字段拒绝

Memory 最小实现：

- [ ] 定义 `MemoryRecord`、`MemoryCandidate`、`MemoryPromotion` 的 Phase 1 子集
- [ ] 定义 `WorkingMemory` 与 project-scoped candidate 的可见性边界
- [x] 实现 candidate 的 provenance、confidence、lifecycle state 和 source event refs（MemoryCandidate 最小领域子集；检索 EventLog 子切片已接线，promotion 与 memory 写入/promotion EventLog 仍待完成）
- [x] 实现按 project / goal / task 的确定性 metadata retrieval（`MemoryCatalog` 稳定过滤/排序；controller 显式接口约束当前运行 scope，在安全边界后返回 metadata，记录 requested/filtered/used；used 仅表示 returned_metadata_only，不是模型使用、prompt 注入或授权，PolicyGate 接线仍待完成）
- [x] 为 candidate 提供本地 JSONL metadata persistence（`MemoryStore` 在锁内校验并拒绝重复 ID，`read_catalog()` 复用 catalog insert，保留 lifecycle，错误按物理行报告；整份加载，不做 source-event 自动解析、promotion 或授权）
- [ ] 将 memory 写入、检索和 promotion decision 写入 EventLog（candidate 写入与 metadata-only promotion decision 事件已接线；检索三阶段事件已接线；实际 promotion/materialization、授权和状态迁移仍待完成，不能整体标为完成）
- [ ] 明确 Phase 1 不启用 vector index、graph index、global auto-promotion 或 cross-project implicit visibility
- [ ] 定义 `EvidencePack`、`WikiPage`、`WikiRevision` 的 Phase 1 子集
- [ ] 实现通过验证结果生成项目范围 `WikiPageCandidate`
- [ ] 实现 wiki-first 查询、source citation、expiry 和 revalidation 事件
- [ ] 实现低风险项目页自动发布，高风险/global/cross-project 只生成 candidate

验收：

- 写入一轮 fake loop 后，event/state/trace 都能查询
- 进程重启后能读取 current state
- trace 文件可被 `tail` / `rg` 阅读
- candidate 可用 `validate_source_events` 对调用方已授权事件集合做引用唯一性与 project/goal/task 身份校验；不是 EventLog 自动接线或来源真实性证明，insert 不自动执行该校验；project scope 检索不会返回其他项目的候选
- memory candidate 不会自动变为 global policy、skill 或 model routing
- Wiki 页面可以反查 EvidencePack/source ref，过期页面不会直接支撑高风险动作

### P4. ToolRegistry

- [x] 定义 `ToolProfile`
- [x] 注册只读工具
- [x] 注册 shell / command 工具的受控包装（固定 argv 的只读 `git status` / `git diff`；不接受 shell 字符串）
- [x] 为每个工具配置 side-effect class
- [x] 为每个工具绑定 risk level
- [x] 读取 baseline tool recipes

验收：

- 未注册工具不能调用
- 工具调用前能查到 side-effect class
- `rg`、git status、Markdown link check 至少有 recipe

### P5. PolicyGate

- [x] 实现工具调用前 policy check（LoopController 先通过 PolicyGate，才调用只读执行器）
- [ ] 实现文件写入前 policy check（bounded Markdown controller 已接入 policy、生命周期安全边界及强制 checkpoint/ledger 审计门禁；通用模型驱动文件写入仍未完成，保留整体待办）
- [x] 实现高风险动作升级到 human approval
- [x] 实现 forbidden tool 阻断
- [x] 实现 policy decision event 记录

验收：

- 低风险只读工具允许执行
- 未授权写操作被阻断或要求审批
- policy decision 可在 event log 中复盘

### P6. LoopController

- [ ] 支持 Task 状态合法迁移和非法迁移拒绝

- [x] 实现 `single_agent_loop` skeleton（只读 observation loop；无模型、工具和文件副作用）
- [x] 实现 `react_tool_loop` 的最小 step 流程（策略检查 + 受控只读 `git status` 执行）
- [ ] 实现 `plan_execute_loop` 的最小阶段状态
- [ ] 支持统一的 step start / complete / fail（model step 已有 start/complete/fail 事件，observation path 只有 start/complete；plan_execute 和统一失败状态接线仍待完成）
- [ ] 支持 stop condition
- [ ] 支持失败后 reflection summary
- [x] 实现 LoopBudget 与 NoProgressTracker 的领域级停止规则
- [x] 实现 `UserCommandInbox` 持久化、command_id 幂等和按 loop 的 pending 读取；LoopController 已在安全边界应用 `pause` / `resume` / `cancel`，`takeover` 和 side-effect 不确定分支仍待完成
- [x] 增加注入式完整行 `CommandPoller`，并提供 poll → validate → submit → 下一安全边界 apply 的 controller API；真实 OS stdin、后台 polling 和 takeover 仍待完成
- [x] 实现 `GoalRevision` / `PlanRevision` 的最小可序列化契约与旧 revision 失效（仅领域 metadata/stale-plan boundary；不接后台 takeover、自动重规划或审批恢复）
- [ ] 实现 `InterruptRequest` 的安全边界和 `side_effect_uncertain` 分支
- [x] 实现 `LoopBudget` 与 `NoProgressObservation`，达到上限后停止自动重试（fixture/resolved model step 已接入基于 `model_id + goal_id + observation` fingerprint 的重复请求检测；完整 action payload、plan revision、失败分类和跨进程恢复仍待完成）

验收：

- 能跑完一个无文件修改的只读 loop
- 能跑完一个有文件修改和验证的 dev loop
- 每个 step 都有 event 和 trace
- 插话不会清零 TaskRun budget、Attempt lineage、retry 或 no-progress counter
- [ ] 重复动作、重复失败和重复计划均能触发 `no_progress_detected`（当前仅已验证 model-step fingerprint 的重复请求检测；其余场景待实现）

### P7. RecoveryCheckpoint

- [x] 实现 checkpoint metadata JSONL 写入
- [x] 记录 checkpoint 与 goal / loop / step 的关系（workspace-edit 路径写入三者标识）
- [x] 记录 checkpoint 前后的文件 diff boundary（workspace-edit checkpoint JSONL 只保存路径绑定的边界 hash，不保存正文或实际 diff；独立 snapshot private record 保存 bounded 正文，采用 content-only hash，与旧 checkpoint hash 不通用）
- [x] 记录 checkpoint 覆盖范围（当前为 workspace-edit action 指定的相对文件路径，非通用多文件范围）
- [x] 记录不可恢复 side effect 引用（当前写入 step 级 side_effect_ref；`SideEffectAuditStatus::Uncertain` 与 restore-audit typed signal 已实现；外部不可逆动作的完整处理仍待完成）

验收：

- bounded Markdown controller 修改前强制 checkpoint/ledger 配置与审计成功，policy Allow/approved 不能绕过；修改后记录 before/after hash 和验证结果。默认 controller 不自动捕获 snapshot，不保证后置审计故障自动 rollback
- 独立 `WorkspaceSnapshotStore` 已有 Unix-only 单个既有 UTF-8 Markdown 文件的 capture/显式 restore primitive：1 MiB 正文、6 MiB + 64 KiB record 上限，workspace 外私有 owner/权限与 project/goal/workspace/scope/path 绑定，并校验当前 content hash。一致时才执行恢复；审批不属于 storage primitive，而由 controller 唯一授权入口通过 PolicyGate、`ApprovalScope::Once`、ApprovalStore 和 SideEffectLedger 完成。人工修改导致 hash mismatch 时拒绝；正文不进入公共 metadata/错误或自动模型/事件上下文。权限非加密/同用户隔离、不抗恶意并发，replacement 后 sync 错误须检查。
- [x] 以 opt-in 方式将 `WorkspaceSnapshotStore` capture/显式 restore 接入 bounded Markdown controller：仅在调用方配置 snapshot store 时 mutation 前 capture，restore 仅由 trusted caller 显式批准并通过 controller binding、snapshot integrity 与 expected current content hash 检查；verification failure 不自动 rollback，不恢复 TaskRun/Attempt、进程或外部副作用
- [x] 将 controller 的显式 restore 纳入统一 PolicyGate、approval store 与 SideEffectLedger 授权/审计路径（`restore_workspace_snapshot_with_authorization` 是唯一 controller restore 入口，强制 metadata-only `ApprovalScope::Once`、snapshot preflight 和 ledger 生命周期；不接受模型 action）
- `CheckpointStore::latest_for` / `latest_for_loop_step` 可按 loop/step 流式查询最后追加的 checkpoint，解释已记录 hash 和 recoverability，给出 `NoAction` / `RestoreRequired` / `ManualIntervention`；非法或歧义 hash、未知 changed-hash 恢复状态和矛盾的 no-write 标记不自动放行
- `CheckpointStore::latest_for*` 查询不重算当前 workspace hash、不校验 snapshot、不执行 restore/replay；这些限制属于只读查询，不否定独立 primitive 的显式文件恢复。查询仍全日志扫描，单条记录大小仍影响内存
- checkpoint 不声称恢复未记录的外部动作；`side_effect_uncertain` 现在会进入 typed uncertain signal 并禁止自动执行，但统一人工处理、补偿或恢复决策分支仍待完成

### P8. SideEffectLedger

- [x] 定义最小 side-effect record schema
- [x] 实现 tool call 到 side-effect event 的记录（只读工具和 workspace-edit 执行路径已写入 TraceStore/SideEffectLedger；并非所有 tool-call registration 都自动产生 ledger record）
- [ ] 支持 read-only、本地可逆、部分可逆、可补偿、不可逆、人工处理的完整分类与状态机（已有基础 `SideEffectDisposition` 派生映射、`SideEffectAuditStatus` 解析及 typed `Uncertain` fail-closed 门禁；部分可逆、补偿、不可逆动作的完整恢复/补偿状态机及执行协调仍未实现）
- [x] 高风险副作用要求 policy gate（workspace-write 已经过 PolicyGate，并在需要时走 approval/checkpoint；uncertain 状态禁止自动执行，但不可逆动作和统一人工处理仍待完成）

验收：

- 已接入的只读工具和 workspace-edit 执行路径有 side-effect 记录或明确 class；通用 tool-call registration 覆盖仍待完成
- 当前未实现的不可逆动作默认不能自动执行；完整不可逆分类和人工处理状态仍待完成
- checkpoint metadata 能查询当前记录的 side-effect boundary；restore/replay 前的完整边界协调仍待完成

### P9. GoalAlignmentCheck

- [x] 实现 root/stage goal 对齐检查报告
- [x] 检查 scope / out_of_scope
- [ ] 检查是否需要 human intervention
- [ ] 将检查结果写入 event log

验收：

- 故意偏离任务时能产生 warning 或 block 建议
- 检查结果能关联到 step_id

### P10. DriftGuard

- [x] 检查修改文件数量
- [x] 检查 diff 行数
- [x] 检查核心文件触碰
- [ ] 检查大规模重命名 / 删除
- [x] 输出 allow / warn / require_checkpoint / require_human_approval / block

验收：

- 小范围文档修改允许
- 大范围无关修改触发 warning 或 approval
- 核心配置修改触发更高风险决策

### P11. Baseline Tool Recipes

- [x] ToolUseErrorEvent 能关联原始 tool call、failure category 和 LessonCandidate（通过 `tool_call_failed` 事件 payload；candidate 的 source_event_ids 精确引用实际持久化失败事件，已有回归；recipe candidate 仍待实现）

- [ ] 写入 shell safe quoting recipe
- [x] 写入 `rg -F` recipe（`rg_fixed` 固定字符串、固定 argv、workspace scope 与 bounded 输出）
- [x] 写入 git status / diff read-only recipe（固定 executable/argv、workspace scope、bounded 输出与超时）
- [x] 写入 Markdown link check recipe
- [x] 写入 file write verification recipe（固定 workspace path + `sha256:` hash，bounded 读取，只返回验证元数据；不开放任意文件写入）

验收：

- ToolRegistry 能读取 recipe
- tool call 记录能引用 recipe id
- 失败工具调用能生成 recipe candidate

### P12. Model Registry、Prompt Resolution 与规则路由

- [x] 实现静态 `ModelRegistry` 和 `PromptRegistry`（注册及 registry JSON 反序列化重验标识/关联；`ModelRegistry`/`RuntimeConfig` 对 `auth_ref` opaque reference 和 endpoint URL 认证材料的 focused 回归已通过；opt-in resolved bridge 已接线，所有入口自动解析与 live provider E2E 尚未完成）
- [x] 建立 provider-neutral OpenAI-compatible adapter、fake transport 和严格错误分类；新增 non-stream HTTP transport、注入式 credential resolver、localhost 安全集成测试和显式 no-redirect/no-retry/response-limit 边界；完成 secret-free profile → provider adapter 绑定（focused 测试通过；配置绑定不调用 resolver、不读取环境变量、不发起网络请求；wire body 不包含 `auth_ref`，transport 只在出站边界解析引用并加入请求头；非 `openai`/`openai-compatible` endpoint 会在 resolved bridge 发送前拒绝）
- [x] 增加独立 Anthropic non-stream protocol adapter（独立 `x-api-key` / `anthropic-version` envelope、状态分类、响应解析和 secret-free auth_ref 校验）；HTTP transport、profile binding 和 localhost/fake focused coverage 已有，live credential/provider 调用仍未验证
- [x] 建立 provider-to-`ModelAdapter` bridge（仅接受严格 `decision`/`summary` JSON envelope；固定输出契约放 system，用户目标与 observation 作为 JSON 编码数据放 user，不升格为 system/developer；fake transport 实际请求体已有回归，不代表完整 prompt 解析或提示注入防护）
- [x] 增加 bounded foreground provider runner 库/CLI 入口（加载并重验 profile、冻结 prompt/route、接入 resolved LoopController、ModelCallStore 与 metadata-only ToolCallRecordStore；OpenAI-compatible 与 Anthropic 均有 library-only 注入式 transport seam；bounded Markdown 的 mutation/verification ToolCallRecord 生命周期有 focused 回归；CLI 默认使用拒绝型 resolver，未配置可信 resolver 时 fail-closed，不代表 live provider 已接通）
- [ ] 接入至少一个真实网络 model provider（HTTP transport 和 secret-free profile/auth_ref 绑定已通过 focused 回归；仍未完成获准 live 调用）
- [x] 将既有resolved多步只读循环接入双协议provider库入口：显式workspace/Markdown授权、action/verifier预检、同TaskRun次数/token预算、语义验证失败停止、真实StateStore/ExecutionQuery投影证据；只读范围不等于通用coding agent，Unix以外fail-closed，契约见`BOUNDED_READONLY_PROVIDER_LOOP_CONTRACT.md`
- [x] 新增该bounded只读runner的独立CLI：打开store前校验七条runtime路径及state路径、协议自动分派、保留原执行错误、输出真实停止原因；合成双协议与路径/状态回归已通过，见`BOUNDED_READONLY_PROVIDER_CLI.md`；获准live验收仍待完成，不放开HTTPS门禁
- [x] 新增 `loop preflight-readonly` 离线本地准备检查：复用上述参数、profile/scope 和八目的地门禁；不解析凭据、不联网、不打开 stores、不预留身份。9 项合成回归通过；可写性、transport、历史内容/身份及实际工具语义均不在预检范围，不能据此勾选 live 或完整 Agent E2E
- [x] 验收 bounded Markdown library 的纯 pre-open 十路径/可选 state 检查、冻结 workspace 身份、模型前后复核及双协议 resolver；14 项新增及既有 15 项定向、全量 907 项通过，契约见 `BOUNDED_MARKDOWN_RUNTIME_GATES.md`。用户写入 CLI/live 前须先解决共享审计写入端的路径替换边界；本地 metadata 检查不等于原子保护或零审计副作用
- [x] 接入 secret-free `LocalFixtureModelAdapter`，并记录结构化 model decision 事件
- [x] 配置 cheap/fast、strong/slow、specialized、evaluator 和 local profile fixture（`tests/fixtures/model_roles.json`；`tests/model_role_fixtures.rs` 验证角色/能力、风险、预算/延迟、隐私及 fallback 候选过滤；均为虚构配置，非实测模型能力/价格或 live provider 验收）
- [x] 实现按 task type、agent role、能力、风险、成本和延迟过滤与排序的规则式路由，并通过 `ModelAdapterRegistry` 接入 LoopController（需审批时 fail-closed；审批恢复尚未实现）
- [x] 定义、校验并持久化 `RouteDecision` 到 EventLog；routed 模型调用的开始/成功/失败事件关联 route ID
- [x] 实现按 prompt ID/version、role/task 的最小 prompt binding 查询
- [x] 接入 provider/model 兼容条件及角色、任务、skill 和上下文分层，形成可冻结的 `ResolvedPromptPack` 并由 opt-in resolved bridge 使用（fake/localhost 请求已验证模板实际进入最终 messages、logical model 与 wire model 分离、wire body hash 一致；resolved controller 审计路径已接线）
- [x] 定义并校验 `ModelCallRecord`、prompt 版本和验证状态，提供独立 append-only `ModelCallStore` 及有界 `read_page(offset, limit)` 查询（opt-in resolved controller 成功记录与 adapter-error 分支的独立 `ModelCallFailureRecord`/failure JSONL 已接线；失败审计不存原始 provider error 或伪造 response facts，审计错误 fail-closed；resolved-provider local contract fallback 已建立子 Attempt 并保留失败 JSONL；live provider、resolved fallback 的生产接线与 live 验收及完整验证结果仍待完成）
- [x] 定义并校验版本化 `ModelActionEnvelope`、路径范围和 verification contract；fixture/resolved provider 的只读 action 已接入模型输出→PolicyGate→ReadOnlyToolExecutor→verification，并写入最终 `ModelCallRecord` 审计；bounded Markdown runner 已接入模型动作 controller 的 PolicyGate、checkpoint／side-effect ledger、写后验证与失败证据，失败不自动恢复；只读工具 raw output 已从 EventLog/TraceStore/SideEffectLedger 脱敏；模型驱动 Markdown CLI／生产接线、多步写入、通用文件写入／Bash、真实 provider、完整 artifact 与跨进程恢复仍待完成
- [x] 实现能力兼容、非静默的 fallback 事件（routed fixture/model-adapter 路径仅对 retryable provider failure 按声明顺序切换；新建父链可追踪的 Attempt，只引用实际追加的事件，累计预算与 no-progress 不因 fallback 重置；不可恢复失败/额度耗尽后关闭运行；resolved-provider local contract fallback 已有候选 route/prompt 重新解析、子 Attempt／父链、共享累计预算、失败 JSONL 和非静默事件；生产接线、相关分支与 live 验收及跨进程恢复仍待完成）
- [ ] 端到端验证 registry、prompt、provider request、trace、event log 和 checkpoint 均不保存认证明文（registry/config 的 auth_ref 与 endpoint 边界 focused 回归已通过；`ToolCallOutput::redacted_for_model()` 已提供模型交接边界，多步只读 `run_resolved_model_loop()` 已强制通过该边界构造下一步 observation；全链路审计脱敏与 live 验收仍未完成）

### P12.2 紧凑交互与结构化进度边界

- [x] 明确 CLI/TUI/Web 共用结构化 `ProgressEvent`/execution projection，不由各 UI 重新解释事实（设计已记录；当前不代表 TUI/Web 已实现）
- [ ] 将 CLI 默认输出收敛为短状态、当前动作、耗时/预算和结果摘要；长参数、trace 和历史改为详情查询
- [ ] 验证 progress/action 生命周期逐事件 flush，且不输出 API key、endpoint、auth_ref、凭据响应或未授权正文
- [ ] 验证低风险只读动作、路由建议和性能档位升级不触发重复人工询问；安全边界触发时输出短拒绝原因和可查询审计引用
- [ ] 定义窄屏、宽屏和非交互 stdout 的稳定格式，避免大段竖向输出和无意义横向空白

本项不提前实现完整 TUI/Web control plane；OpenCode 的鼠标交互、DeepSeek Harness 的历史时间线和 Codex 的 TUI 可达性作为后续 adapter 设计参考。

### P12.3 受约束的主动自适应路由

- [x] 定义 `EffectLevel`（fast/balanced/strong/x_high）、`QualityAssessment` 和 `AdaptationDecision` 的版本化契约（`src/adaptive_routing.rs`；focused contract tests）
- [ ] 将结构化输出、verification、工具结果、超时、截断、usage、延迟和无进展计数接入质量评估（`QualityObservation` 与 fail-closed `QualityAssessment::from_observation()` 已完成；普通/resolved model step 已写入 metadata-only `model_quality_assessed`，usage/latency/truncation/no-progress 的统一运行时填充仍待完成）
- [x] 将 profile 显式固定效果配置接入配置 HTTP factory、foreground 和 bounded Markdown 共用装配 helper（配置与公开变异重验；四档/no-effect 请求 body/hash、非法 Markdown 零执行、Anthropic 明确拒绝已有回归；仅在测试模块向生产共用 helper 注入 loopback transport 验证 HTTP 出站，生产 HTTPS 门禁不变，不代表 live 或自动升级）
- [ ] 实现当前 TaskRun 内的单级效果升级，受总预算、最大 Attempt、最大重试和冷却窗口约束（显式 opt-in 的 `run_resolved_upgrade_model_step()` 库入口已接入：宿主质量驱动单次相邻升档、真实新 Attempt/Route/PromptPack/Call、父证据保留、累计次数预算及 child 再验收；一次上限不允许反复升降，无额外循环。纯观察 foreground 新入口接线见 `FOREGROUND_EFFECT_UPGRADE_CONTRACT.md`，必须由可信宿主提供 evaluator；CLI自动质量判断、Markdown/Anthropic升档、发送前预留硬上限、金额预算和统一测量仍未完成，故整项不关闭；安全/动作/脱敏及解析拒绝不升级重调）
- [x] 实现TaskRun已报告token累计与后续发送门禁（`REPORTED_TOKEN_BUDGET_CONTRACT.md`）：默认关闭、首次调用前显式启用、逐字段未知不计零、checked累计、重复ID不重发、成败共用账本、审计/进度故障阻断；恢复与升档累计有独立回归。库片历史全量64套/774项、严格clippy、lib/bin build、fmt/diff通过。不是单请求硬上限、金额预算或live验收。
- [x] 将已报告token门禁接入顶层可选`reported_token_limit`和既有runner（`REPORTED_TOKEN_RUNNER_CONFIG_CONTRACT.md`）：缺省/null关闭、0启用首发拒绝、非法值固定诊断；prepare冻结额度，OpenAI/Anthropic foreground及Markdown双协议共用门禁，零send/零resolver/零invocation及正额度私有快照有回归。该历史切片master独立全量783项、严格clippy、lib/bin build、fmt/diff通过；本身不新增自动升档或CLI参数。新显式foreground升档入口复用同一TaskRun账本，不提供跨运行累计配额或完整TaskRun持久恢复；串行同历史身份重用门禁见下一项。
- [x] 增加 provider runner 历史身份启动前门禁（`PROVIDER_RUN_IDENTITY_GUARD_CONTRACT.md`）：OpenAI/Anthropic foreground与Markdown共用检查，任意事件顶层或typed aggregate的loop/task身份、成功/失败模型历史的task或精确next call ID冲突即发送前拒绝；坏历史固定错误无source。仅可信串行caller、同一日志集合，不是并发原子认领、日志丢失后安全重发、跨运行配额或完整恢复。
- [x] 独立复现并修复跨loop同名step的StateStore投影冲突（`LOOP_SCOPED_STEP_PROJECTION_CONTRACT.md`）：投影按loop→step嵌套索引，公开`step(loop,id)`无歧义查询；明确loop的terminal不跨loop回退，缺省loop只接受全部候选唯一匹配。同名双步骤及双协议foreground共享日志查询有回归；新嵌套JSON可round-trip，旧非空flat投影需从原EventLog重建，不做自动迁移。不改变provider/controller/call ID，不代表完整CLI/live/任务恢复。
- [x] 实现 retryable provider failure 的新 Attempt／新 RouteDecision／失败证据保留；禁止静默覆盖原 Attempt（普通 routed 与 resolved routed controller 已接入 `plan_adaptation()` 门禁，并保留父 Attempt 失败 evidence；完整质量驱动 controller 仍待完成）
- [x] 修复resolved模型白名单旁路（`RESOLVED_MODEL_ALLOWLIST_CONTRACT.md`）：共用路径同时约束request/route模型，routed首选与fallback在resolver前检查，拒绝fallback不创建child且保留原失败；空白名单仍不限制，四个候选resolver/binding错误出口由下方独立增量处理。
- [x] resolved routed候选失败收口（`RESOLVED_ROUTED_FAILURE_CLOSURE_CONTRACT.md`）：primary/fallback的resolver及候选校验失败终结三层状态，保留预算/证据，审计失败优先返回且修复日志后仍不可重发；不含后续route binding/adaptation等出口。
- [x] 普通OpenAI-compatible/Anthropic foreground调用失败的runner终态收尾（`FOREGROUND_FAILURE_CLOSURE_CONTRACT.md`）；保留合法routed fallback，不将共享resolved错误路径一律提前终结。错误时保留预算/证据，非Running不覆盖；主代理全量852项通过。
- [ ] controller构造期间初始进度回调失败的生命周期收尾；这是独立边界，不属于上述普通foreground的已创建controller错误收尾。
- [x] 校验自适应决策不能改变 provider allowlist、endpoint、auth_ref、隐私等级、工具权限和安全底线（契约决策不携带这些字段；route planner 只返回已声明 fallback）
- [ ] 对低质量、不可重试、安全阻断、预算耗尽分别生成可解释的 metadata-only 审计事件（当前 routed retryable fallback 已有 `model_route_fallback` 与 `model_adaptation_stopped`；NeedsUpgrade、Blocked、Failed 的统一自适应事件仍待完成）
- [ ] 用 fake/localhost provider 覆盖 Verified、NeedsUpgrade、Retryable、Blocked、Failed 和预算耗尽分支
- [x] 将两协议决策 parser 的 JSON 语法、结构、空摘要、动作合同及脱敏拒绝区分为无正文 typed 诊断（公开错误契约变化；controller 审计仍为 `response_unmappable`、质量仍保守失败，不授予重调/升级；保留首先拒绝而非完整安全结论）
- [x] 让 resolved 解析/后置边界失败保留已知的安全 usage/hash/finish reason 证据；未知用量保持未知，本failure-evidence切片不实现累计预算；已报告token累计与门禁见独立条目，金额预算仍待完成，不能将八维次数预算当作计费预算（`ResolvedCallFailure`/`ResolvedCallEvidence` 已接入 OpenAI-compatible、Anthropic、Markdown wrapper 和 controller；focused 101项、全量62套/733项为该切片历史验收，严格 clippy、lib/bin build、fmt/diff 均通过；不代表自动升级生产入口、金额预算或 live provider 已完成）

Phase 1 不实现跨任务在线学习、并行 quorum、自动修改全局路由配置或根据单次异常永久调整模型权重。

历史验证记录（2026-10-11）：`loop_controller` 25 项、`provider_model_adapter` 7 项、`resolved_provider_model_adapter` 11 项、`model_call_controller_audit` 6 项 focused 测试通过；当轮 focused clippy 通过，构建产物写入 `/tmp/steerbox-phase1-target`。此记录不是当前工作树的完整验收。

此前核验（2026-10-13）：master 独立执行 `CARGO_TARGET_DIR=/tmp/steerbox-phase1-target CARGO_BUILD_JOBS=2 cargo test --offline --tests --quiet`，退出码 0，所有测试目标通过；现有 acceptance 测试仍有 unused variable / unused mut 警告。该次执行包含 Anthropic 7 项、provider bridge 12 项、resolved provider 11 项、本地脱敏 14 项及审批接入 3 项；之后新增测试或修补须重新验收，不能沿用这次结果。该记录当时尚未覆盖后续 checkpoint hash、routed fallback 和 side-effect disposition 切片。

本次 focused 复核（2026-10-13）：`model_registry` 6 项、`config_anthropic_binding` 8 项、`model_call_store` 23 项、`model_call_controller_audit` 18 项全部通过；该结果只证明当前 focused 切片，不替代全量 fmt/clippy、live provider、常驻 runtime、完整审计脱敏、fallback 或跨进程恢复验收。

历史核验（2026-10-13，非当前复跑）：`cargo fmt --all -- --check`、`CARGO_TARGET_DIR=/tmp/steerbox-phase1-target CARGO_BUILD_JOBS=2 cargo clippy --locked --offline --lib -- -D warnings`、`CARGO_TARGET_DIR=/tmp/steerbox-phase1-target CARGO_BUILD_JOBS=2 cargo test --locked --offline --tests --quiet` 和 `git diff --check` 均通过。新增 routed fallback、checkpoint before/after hash、side-effect disposition/status focused 测试通过；全量 acceptance 仍有既有 `ui_responsiveness_test.rs` 的 unused `mut` / unused variable 警告。该结果仍不代表 live provider、真实 credential store、resolved-provider fallback、Attempt lineage 恢复、后台常驻接管、文件写入/Bash 全工具面或完整 Phase 1 已完成。

验收：

- 低风险只读任务可路由到 cheap/fast profile
- 高风险或高质量任务不会只按最低成本选择模型
- 切换模型会创建新的 attempt / route decision，而不是静默覆盖
- 每次调用能反查 provider、endpoint、model、prompt 版本、路由理由和验证结果
- 仅保存 `auth_ref`；测试 fixture 不包含真实 API key 或 token

### P12.1 执行策略选择器

- [x] 定义九个策略标识、显式 TaskFeatures 和纯规则选择器；风险复用 domain RiskLevel，有限值/范围校验及倍率不足拒绝已有回归
- [x] 实现安全预算分配：整数比值运算分配、既有授权/当前上限 clamp、消耗保留、进展/失败 guard 不放宽
- [x] 接入 opt-in `start_with_strategy` / `select_strategy`，记录选择/调整及审批介入事件；GoalContract 风险下限、审批前零动作、step/model/tool 预算和调整不补额度已有测试
- [x] pause/resume 不解除审批，不清零消耗或重复动作历史；PolicyGate forbidden/unregistered/allowed_tools 约束保持生效
- [ ] 自定义策略配置注册与验证调度接线（当前只有固定配置和验证频率元数据，不撤销 goal 验证要求）
- [ ] 策略状态/修订、预算和重复动作历史的完整恢复

验收：`tests/strategy_selector.rs`、`tests/budget_calculator.rs`、`tests/loop_strategy.rs`、`tests/loop_strategy_controls.rs`、`tests/strategy_audit_failure.rs`；策略主审计写入失败保留旧策略、额度和revision，同实例恢复原日志路径后下一修订仍连续；补充审批事件写入失败暂无独立注入测试。审批 resolver 尚未实现。协作和专业领域策略仅是元数据，不代表多 agent 或专业领域 runtime。旧 start/fixture CLI 不自动选择策略；新 start_with_strategy 不是跨实例恢复入口。

### P13. Dev Agent Vertical Slice

- [x] 输出只读 BoardProjection / JSON board fixture

- [x] 定义一个小型真实开发任务（Markdown 修改与链接验证）
- [x] 创建 goal
- [x] 加载上下文
- [x] 执行文件修改
- [x] 运行验证命令
- [x] 记录 checkpoint 和 side effect
- [x] 输出 run summary

验收：

- 任务能端到端完成
- event log 可复盘
- trace 可读
- checkpoint metadata 存在
- 验证结果明确

### P14. Documentation Update

- [x] 更新 PHASE_1_CONTRACTS.md 的引用（当前实现状态在本 TODO、Scope 和 Validation Plan 中维护）

- [x] 更新 README 或开发说明中的运行命令
- [x] 更新第一阶段完成状态
- [x] 记录已实现能力和占位能力
- [x] 记录已知限制
- [x] 明确运行数据写入 `/temp/steerbox/<project_ref>/.steerbox/`
- [x] 增加 metadata-only execution history/query 投影、`execution list/show/page` CLI 和 detached snapshot 分页（底层仍使用 `read_all()`，不是 live cursor）
- [x] 明确用户插话、计划失效和死循环停止契约

验收：

- 用户能按文档跑最小验证
- 文档不把占位能力写成已实现能力

## 第一阶段暂不做 TODO

以下不要在第一阶段拆任务：

- 跨任务历史表现驱动的在线自适应多模型路由
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
