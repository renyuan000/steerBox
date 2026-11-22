# Phase 1 Validation Plan

## 2026-11-22 Workspace editor W1（限定普通文件切片已验证）

基线 `c7df098`；旧 edit/audit-gates/snapshot 基线70项通过。仅接read seam后，普通父目录
替换测试真实RED为0通过/1失败：读取原文件后替换parent，旧实现报告成功却未更新原目录。
同测试在pinned-parent实现后通过；另补失败清理不触碰替身目录temp sentinel的回归。
实际source/temp FD 10项覆盖read、partial/full write、权限复制、file-sync、publish前失败、
publish后parent-sync失败及成功/匹配短路；检查真实字节/inode/权限/阶段与普通环境temp清理。
新增公开兼容7项覆盖approval优先级、noop/mismatch与预存temp、冲突保全、Unicode/CRLF字节数、缺失零创建。

最终全量87个harness/1201项，0失败/跳过，严格all-targets Clippy、实际lib/bin构建、fmt/diff通过。
三路独立源码/兼容/故障审查及master实际diff审查无剩余阻断；移除四条不能证明发布后所有权保留的
UID/GID断言后已重跑完整门禁。本片未实施生产mutant；上层故障传播仅源码核查，不冒称故障E2E。
非Unix行为收窄（approved noop也拒绝）、可读parent、诊断、post-create校验/cleanup失败残留及
发布后错误边界见 `WORKSPACE_EDIT_IO.md`。不重试未获准专项，不放开live写，不提供完整恢复。
命令/日志、提交与合入复验见 `/temp/codex/steerbox-validation/workspace-edit-path-20261122/RECEIPT.md`。

## 2026-11-22 StateStore S1（限定普通文件切片已验证）

基线 `ac0070c`。定向基线4个harness/46项通过；新公共回归RED为8通过/1失败，
裸相对basename子进程无法确认成功。修复后同组加新回归共55项通过；最终新增普通文件
harness为10项入口（含子进程probe及exclusive冲突后显式再调用），不是自动重试能力。
真实FD6项覆盖partial/full write报错、file-sync/publish/parent-sync失败和成功时
parent rename/decoy/witness；检查旧bytes/inode保留、同实际FD同步、阶段短路、
发布后报错但新投影可读、内存投影不变及普通环境temp清理。
共享helper15项通过，含新增exclusive冲突与pinned-parent清理2项；不将共享测试
外推为新的State特殊路径/权限专项，也不重试被拒绝的A1专项。

最终全量86个harness/1182项通过，0失败/跳过；严格all-targets Clippy、实际lib/bin
build、fmt/diff通过。三路独立规格/安全/故障与文档审查、master实际diff审核无剩余阻断。
本片未实施临时mutant。非Unix磁盘IO改为fail-closed；IO文本改stage/kind；目录可读要求、
整链同步成本未测、cleanup失败及post-create校验失败可能残留，见 `STATE_STORE_IO.md`。
不证明掉电恢复、CAS/fencing、跨store事务、持久预算或授权恢复；不放开live写门禁。
命令、日志及提交/合入复验以
`/temp/codex/steerbox-validation/state-path-20261122/RECEIPT.md` 为准。

## 2026-11-22 ApprovalStore A1（限定增量，验证受限）

基线 `ee87518`。普通文件兼容、真实进程锁竞争/锁内重载与私有 FD 故障测试
分别记录结果，不把测试已编写或共享 helper 已通过外推为审批入口已完整验收。
故障范围为 partial write、完整写入后报错、文件同步失败、parent 同步失败；
检查短路顺序、缓存不提前推进，以及完整 Consumed 可读后的同实例/重开重试
不再授权、不再追加。成功路径检查真实 inode/fd 和临时 parent rename+witness。
这些只覆盖当前可读字节的判定，不证明 OS 掉电持久性或跨 store 事务。

最终定向48项、真实FD5项通过；process4项默认并行连续30轮通过。
省略真实锁与锁内重载的mutant各0通过/1失败，省略parent同步为3通过/2失败；
三处精确恢复后复验通过。一次持锁测试在drop后收到BUSY，测试最小修正为
显式unlock后再RETRY，不增加延时或业务重试，不改变生产锁行为。
全量85个harness/1164项通过，0失败/跳过；严格all-targets Clippy、
实际lib/bin build、fmt/diff通过。限定规格、安全及文档审核无剩余阻断。

扩展特殊路径矩阵、FIFO、真实降权权限验证被审批层拒绝，尚未实施和验收；
不以既有 symlink 回归或共享 helper 测试替代。最终专项、mutant 恢复复验、
全量、Clippy/build/fmt/diff 与审核状态分别以实际证据为准，详见
`/temp/codex/steerbox-validation/approval-path-20261122/RECEIPT.md`。
不开放 live mutation，不验收完整恢复或 Phase 1。

## 2026-11-22 恢复检查 RDI1（限定诊断切片已验收）

基线 `75cdb2e`。首批黑盒旧 CLI RED 为0通过/6失败，均为 unsupported command；
另外 API 测试首次编译确认尚无模块（E0432），不冒充运行时失败。
最终全量82个harness/1148项通过，0失败/跳过，严格all-targets Clippy、实际lib/bin构建、
fmt/diff通过；专项CLI18入口（含一个子进程probe）、API5、真实fd unit3通过。
省略metadata变化判断的mutant为0通过/1失败，精确恢复后unit3通过；两轮独立源码及文档审核
无剩余阻断，采纳停止位置措辞澄清。代码审核快照与最终核心指纹一致。
专项覆盖 events/state 缺失与零创建、完整/空日志、坏尾、内部损坏、schema/序号、
unknown 计数、非零 step 投影、投影错误、State match/mismatch/invalid、
symlink/dangling/hardlink/FIFO/directory/父目录陷阱、实际降权只读目录，
默认及缩小 byte/line/record 上限、参数先拒绝、报告/Debug 无正文身份路径、stdout 关闭管道。
真实 fd 单测检查不越过 cap 读探针、EOF、读取后长度/权限/nlink 变化；
禁止把检测不到变化称一致性快照或任意同 UID 防护。FIFO 测试不阻塞和 not_compared
不单独证明未 open；该保证另由源码提前返回分支核查。
完整门禁、定点 mutant、独立审核及最终提交/合入复验以
`/temp/codex/steerbox-validation/recovery-inspect-20261122/RECEIPT.md` 为准。
验收不覆盖 OS 掉电、跨 store 一致性、请求/副作用对账、修复或跨进程续跑；
未连接真实 provider，不读取凭据，不解除 live 写门禁。

## 2026-11-22 UserCommandInbox I1（限定切片已验收）

基线 `d271f7c`。旧实现公开路径 RED 为2通过/5失败，证明不安全读取被接受和非法路径被当空历史，
不是误写实证。GREEN 为既有22项、新路径19项（含一个子进程探针入口）、真实fd unit6项。
省略最终目录 ACK 的 mutant 为1通过/5失败，精确恢复并补历史前缀/Debug脱敏断言后6项通过。
最终全量80个harness/1122项通过、0失败/跳过，严格Clippy、实际lib/bin构建、fmt/diff通过；
独立审核无剩余阻断。初轮Clippy的测试多余clone已精确修正，最终门禁全量重跑通过。
完整门禁日志、提交/合入及后续复验状态见
`/temp/codex/steerbox-validation/inbox-path-20261122/RECEIPT.md`。

- 公开路径覆盖 parent/leaf symlink、dangling、hardlink、directory、非法全路径、missing零创建；
  FIFO有界子进程；实际降权验证0200读失败、0400读取及幂等可用但新写拒绝、0644保持身份/权限。
- 保留 requested_at 忽略、最新状态/pending顺序、重复状态追加、blank/物理行号/UTF-8 Io、
  无LF JSON可读及后续拼接风险；全历史坏尾不被重复ID的提前返回绕过。
- 私有append单独覆盖四类不安全leaf；JSON+LF、同fd、父目录rename+witness、write/file/parent
  失败短路及原历史前缀保留。open注入为安全打开后的hook；sync错误为seam，不是实际断电实验。
- 公开同intent在完整记录可读时返回历史且不追加、不重新sync；不是持久re-ACK保证。
  无sidecar/原子判重/尾修/Applied事务，读与追加分别pin；parent可读、整链同步成本未测，
  非Unix实际I/O fail-closed，未跨平台实测。live写、审批/State/workspace与完整恢复门禁不变。

## 2026-11-22 RunSummary R1（限定切片已验收）

基线 `d78fa6f`，仅普通JSON读写及最小安全helper；不复用历史切片数字。
公共API真实RED为6通过/7失败；GREEN为R1路径13、audit14/既有路径20、CLI16通过。
真实故障unit5通过；省略目录ACK屏障的mutant为2通过/3失败，精确恢复后5通过。
最终 `cargo test --locked --offline --all-targets` 为79个harness、1097通过、0失败/跳过；
严格all-targets Clippy、lib/bin build、fmt/diff通过。提交/合入及后续复验状态与日志见
`/temp/codex/steerbox-validation/run-summary-path-20261122/RECEIPT.md`。

- exact pretty字节、无末尾LF、长转短无尾留、直接覆盖保持dev/inode/UID/GID/mode；既有0644不拒绝，
  不引入UID/owner-only策略。缺失leaf/parent读取Io且不创建；无效UTF-8为Io，非法JSON为line0。
- 完整路径在mkdir前验证；parent symlink不跟随，leaf symlink/dangling/hardlink/FIFO/directory
  双读写拒绝且sentinel无损，FIFO用有界子进程和Drop回收；不外推为设备或父类型全矩阵。
- 新目录/文件请求0700/0600（umask可收窄）；实际降权核验0200可写不可读、0400相反、0000拒绝，
  只读parent区分既有/缺失leaf，既有对象不chmod。parent需可读是安全遍历带来的权限收窄。
- O_WRONLY安全打开不含提前truncate，regular/nlink验证后截断；测试truncate/部分write/file sync/
  parent sync失败短路、不误ACK及残留字节，同fd和重命名后的pinned parent目标保持。

写入同步完整遍历目录链，性能未测；非Unix实际I/O fail-closed，不称跨平台实测。
失败为uncertain，可留空/部分/完整新正文；无atomic替换、旧正文保全、锁、自动重试/恢复，
不解除live写或State/workspace/inbox/审批及其余真实锁验收门禁。

## 2026-11-22 ToolCallRecordStore T1（限定切片已验收）

基线 `4fe81d9`，仅 tool JSONL 与一个实际 `.lock`。最终
`cargo test --locked --offline --all-targets` 为 78 个 harness、1079 项通过，0 失败/跳过；
`cargo clippy --locked --offline --all-targets -- -D warnings`、
`cargo build --locked --offline --lib --bins`、`cargo fmt --all -- --check` 与 diff 检查通过。
外置 target、`CARGO_BUILD_JOBS=1` 串行执行；独立审核无剩余阻断。
真实 seam 5 项、扩展路径 13 项通过；目录同步省略 mutant 为1通过/4失败，精确恢复后通过。
日志及提交/合入复验见
`/temp/codex/steerbox-validation/tool-store-path-20261122/RECEIPT.md`，不复用 M1 计数。

- constructor 保留 metadata-only 即时诊断，不要求 data 读写权限，也不是 anchored 身份保证。
  实际 data/lock 由一个 pinned parent 安全打开，regular/nlink后再验证 UID/owner-only。
- data/lock leaf symlink/dangling/hardlink/FIFO/directory 均拒绝；parent symlink 不跟随，
  非法全路径在 mkdir 前拒绝；检查受保护目标与另一 leaf 字节、inode/mode/nlink 不变。
  这不是设备节点或所有父路径类型的完整实测矩阵。
- 真实降权核验 0200 append/0400 query、0000 metadata-only 构造、RW lock及只读 parent；
  foreign-owner data/lock 测试需特权夹具，非 root 明示不可用，不隐称已覆盖。
- cap 在方法 I/O 前、zero 在安全锁后跳过 data、missing data 不创建业务文件；
  缺失 parent 不重建。保留 offset/blank/物理行号/UTF-8/早停和无LF尾行读取语义。
- 真实跨进程锁回归包含有界观察窗口；append 另以独立 contender 逐阶段 WouldBlock
  断言持锁及返回后释放，不把进程调度窗口表述为无调度依赖的证明。
- JSON+LF → 同 fd 文件同步 → pinned parent同步 → ACK；注入部分写/file/parent失败，
  检查短路、不误ACK、uncertain字节保留与不自动重试。rename后用目录witness核对目标。
- 不按 tool_call_id 排重，不扫描旧历史；相同记录、同步失败后的显式再次append仍新增。

目录同步性能未测，非 Unix 实际 I/O fail-closed；无sidecar替换fencing、timeout或事务。
无LF尾行后追加拼接风险未修复；不新增尾修、provider/live写CLI或完整崩溃恢复。

## 2026-11-22 ModelCallStore M1（限定切片已验收）

基线 `0a1ca50`。本片仅为成功/failure JSONL 与两实际 advisory lock 的安全 I/O；
最终 `cargo test --locked --offline --all-targets` 为 77 个 harness、1061 项通过，0 失败/跳过；
`cargo clippy --locked --offline --all-targets -- -D warnings`、
`cargo build --locked --offline --lib --bins`、`cargo fmt --all -- --check` 和 diff 检查通过。
Cargo 使用外置 target、`CARGO_BUILD_JOBS=1` 串行运行；独立规格/质量审核无剩余阻断。
日志与提交/合入复验见 `/temp/codex/steerbox-validation/model-store-path-20261122/RECEIPT.md`。
以下检查点限定于本片，不复用下方历史切片计数，也不将源码审查当作测试通过。

- 实现复用逐组件 NOFOLLOW、dirfd-relative、同 fd regular/nlink=1 及非法路径守卫。
  新增四路径专项矩阵覆盖 leaf symlink/dangling/hardlink/FIFO/directory；不把共享守卫的
  源码审查称为四路径各自 device 或父路径全矩阵测试通过。新目录 0700、新数据/锁 0600，
  非 Unix 实际 I/O fail-closed。
- 同一 pinned parent 内获取真实 RW lock、扫描历史、判重并追加；锁持有至 file/parent
  屏障返回。重命名原父目录后，用数据 inode 与目录 witness 检查没有转向 replacement。
- 新追加为完整 JSON+LF、同 fd 文件同步、最终 pinned parent 同步后 ACK；注入部分写入、
  文件同步和目录同步失败，检查阶段、类型、短路顺序及不自动删除/回滚留下字节。
- 两类 identical retry 均须完整校验历史，原 history read fd 的 file/parent 屏障成功才 ACK，
  无 write、无字节/记录变化；屏障仍失败不得提前成功。相同记录后的坏历史/冲突仍报错。
- 保留 `open(directory)` 成功而实际数据访问报错、缺父目录操作报错、只读数据查询与
  identical retry 可用；查询仍要求 RW lock，可创建锁 sidecar，缺失业务记录不创建 JSONL。
  权限测试须确认执行身份实际无写权限，不能仅用 root 下 chmod 推断。
- 分页零 limit 先锁后跳过数据；offset 前仍解析并校验，保留 UTF-8/物理行号、blank 计数及
  页满停止。identity 完整扫描成功与失败历史，成功命中不掩盖后续 failure 损坏。
- `open` 父目录准备同步完整链（含既有目录），后续操作不重做整链，仅最终 parent 屏障；
  性能与锁等待成本未测。锁无 timeout，不防 sidecar unlink/替换，不宣称跨文件原子身份认领。

retry 目录屏障与锁 NOFOLLOW 两个定点 mutant 均被测试拦截，精确恢复后通过。
不新增严格 LF 尾部门禁或尾修；完整无 LF 尾 JSON 后再追加不同记录仍会拼接损坏，
本片只加特征测试，未修复；相同记录补屏障不代表任意失败均可安全重试。
不验收真实 provider、live 写 CLI、事务、恢复检查器或持久 TaskRun/Attempt/预算续跑。

## 2026-11-08 受限只读 provider CLI 验收

基线 `034967f` 的独立 CLI 切片：master 已收取会话 82191 退出 0，
`cargo test --locked --offline --tests --quiet -- --test-threads=1` 共
70 个 harness / 884 passed / 0 failed / 0 ignored；严格 all-targets clippy、
lib/bin build、fmt 和 diff 检查通过。新增 5 个 CLI unit 与 8 个 integration
包含在上述 884 项中，不与 focused 77 项重复相加。
独立规格审核和后续质量／安全审核均通过，零 Critical/Important/Minor。

代码／测试覆盖参数先于配置加载、固定非回显错误、八条目标的 pre-open 门禁、
缺父目录、目录／软链接／硬链接／别名／工作区碰撞、默认凭据拒绝与显式 env、
双协议 action→verification→complete 的真实预算与 model/tool/state 身份、
human/no-action 的实际停止摘要、执行错误优先和成功后 projection 目标变更。
HTTP 的真实双协议分派另由源码审核核实；注入式成功响应不等于 live 验收。

证据根目录：`/temp/codex/steerbox-validation/readonly-provider-cli-20261108/`。
`readonly-cli-{red,green,scope-red,scope-green,final-focused}.log` 为实施者证据；
master 的 `restart-{tests,clippy,build}.log` 与对应 `.exit` 记录实际结果。
旧会话 43658 已失效、`master-tests.log` 为零字节，不作为通过证据。
release CLI／合成 probe 编译会话 78089 已退出 0，日志及退出标记为
`release-cli.{log,exit}`、`release-probe.{log,exit}`。
所有 Cargo 使用 jobs=1 与项目外既有 target；未删除旧工作树或构建产物。

本片为 Unix 上的受限只读 CLI；无真实配置／凭据读取、无 live 发送。
真实模型、通用写入/Bash、后台控制、非环境变量凭据存储和完整恢复仍待验收。
使用契约见 `BOUNDED_READONLY_PROVIDER_CLI.md`。release 资源两轮各 86 次成功，
direct wall 与纠正后的 GNU time 峰值分别取值，不合并成配对样本。CPU/内存、
采样误差纠正、环境及本机原始证据详见
[资源基线](CLI_RESOURCE_BASELINE_2026_11_08.md)。真实 CLI 默认凭据拒绝另为
预期退出 1 的单样本，不算成功 provider 验收；同步退出不代表后台 daemon 空闲性能。

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
- endpoint、`auth_ref` 和完整连接配置不得进入模型输入；普通软件配置、日志和性能数据可在授权范围内分析
- 默认交互输出应紧凑且逐事件刷新，详情通过查询展开；低风险自主路径不得因重复审批造成停顿
- 当前任务内的模型升级和 fallback 必须共享预算、保留失败 evidence，并在安全底线之外自动执行
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
| V7.1 单文件 snapshot/显式恢复 | 已验证独立 primitive 与当前回归 | `tests/workspace_snapshot.rs`、`tests/workspace_edit.rs`；focused 42 + 11 = 53 passed，覆盖私有正文/metadata 分离、binding/scope/hash/foreign-owner 拒绝、restore 错误分类、hardlink/store 祖先替换与 exact-byte restore；master 全量 563 passed/0 failed/0 ignored，strict clippy/lib-bin build/fmt/diff 通过；V7.1 仍不代表 task/process 恢复 |
| V7.2 controller opt-in snapshot 接线 | 已验证 bounded Markdown controller 的 opt-in capture/显式 restore 边界 | `tests/controller_snapshot_integration.rs` 16 passed；另有 `workspace_snapshot` 42 项、`loop_controller` 37 项与 `workspace_edit_audit_gates` 14 项 focused 回归；覆盖 capture 前失败、公开 snapshot root、未配置 snapshot store 的兼容行为、非法 hash 不落事件、pre-mutation audit 失败不注册 binding、verification failure 不自动 restore、Paused/terminal restore 拒绝、authorized restore 的 PolicyGate/ApprovalStore/SideEffectLedger 链、snapshot preflight failure 不消费 Once approval、以及 post-restore audit failure 的 typed `WorkspaceSnapshotAuditUncertain` 语义；默认路径不自动 capture，controller 不再暴露 `approved: bool` restore API，restore 不构成 rollback 或完整恢复 |
| V8 SideEffectLedger | 已验证记录与基础派生解释切片 | `tests/loop_controller.rs`、`tests/dev_vertical_slice.rs`、`tests/audit_store.rs`、`tests/workspace_edit_audit_gates.rs`；ledger 缺失或 mutation 前 append 故障阻止 controller Markdown 写入。disposition/status 是 metadata 派生解释，不改变 JSONL schema、不构成执行授权或完整恢复/补偿状态机 |
| V9 GoalAlignmentCheck | 已验证切片 | `tests/drift_guard.rs`；结果写入完整 loop 仍待接线 |
| V10 DriftGuard | 已验证切片 | `tests/drift_guard.rs` |
| V11 Human Steering Mode | 已验证最小运行时切片 | `tests/user_command_inbox.rs`、`tests/loop_controller.rs`、`tests/state_store.rs`；已覆盖 Inbox 幂等、pause/resume/cancel 安全边界和状态重建，takeover、后台 worker、控制面仍未实现 |
| V12 Baseline Tool Recipes | 部分验证 | registry/recipe 边界已有；只读工具以 `tool_call_requested`、bounded workspace-edit 以 `model_action_started` 的 EventLog sequence 派生运行内 `tool_call_id`，同一请求生命周期复用同一 ID，mutation 与 verification identity 分离，同一 step 重复调用产生不同 ID；identity 字段非法或超长时在 policy/checkpoint/request 事件前 fail-closed；`tool_call_failed` 已记录该 ID、failure category 和最小 LessonCandidate；CLI 已通过 `tool-call page` 验证 metadata-only 分页、脱敏字段边界和 1024 limit 上限；Markdown link-check recipe 已注册并接入受控执行（`tests/tool_policy.rs`、`tests/tool_executor.rs`）；recipe candidate、shell quoting 仍未实现 |
| V13 Model Routing | 已验证本地路由、冻结请求、non-stream adapter 与库审计接线切片 | `tests/model_registry.rs`、`tests/model_router.rs`、`tests/model_role_fixtures.rs`、`tests/loop_model_routing.rs`、`tests/model_call_controller_audit.rs`、`tests/model_call_store.rs`、`tests/model_contracts.rs`、`tests/resolved_prompt.rs`、provider/Anthropic 与 localhost HTTP transport 测试；覆盖 binding/approval fail-closed、wire body hash、成功 resolved 调用的 usage/finish_reason 与成功 store、resolved adapter-error 的独立 `ModelCallFailureRecord`/failure JSONL、未知 response facts 为 None、原始 provider error 不落 failure 日志及失败审计错误；routed fixture fallback 有新 Attempt/真实事件/总预算和失败终态。不覆盖所有失败入口、live provider、resolved-provider fallback 或跨进程恢复；fixture 数值不是实测能力/成本/性能，脱敏不是通用 DLP |
| V13.1 Resolved failure evidence | 已验证安全投影切片 | 两协议 fake transport 验证决策失败的精确 wire hash、逐字段 usage（含完整/缺失/部分/零）与受控 finish reason；OpenAI actionless 和 Markdown 后置门禁保留证据、零工具/原文不变；transport/wire 失败无证据；恶意正文/identity/未知 finish reason 不入错误或持久日志；失败审计写入错误 fail-closed、旧幂等/冲突和 one-shot 限制不变。独立 focused 101项、全量62套/733项、严格 clippy、lib/bin build、fmt/diff 通过。不含 live provider、token/成本预算或自动升级 |
| V18 StreamReporter | 已验证 CLI 接入切片 | `tests/stream_reporter.rs`、`tests/runtime_cli.rs`；支持 lifecycle/progress/disable/writer error/redaction，readonly/fixture 生命周期输出与事件和状态一致；ContinueObservation/RequestHuman 不伪报 loop completed；尚未接入 LoopController 内部工具/模型 streaming |
| V26 紧凑交互与路由边界 | 设计已冻结，代码验证待做 | `docs/AGENT_REFERENCE_ARCHITECTURE_REVIEW_2026_10_20.md`、`docs/SECURITY_MODEL_DISCUSSION.md`；待验证短状态/详情分离、窄宽终端格式、endpoint/auth_ref 不进模型输入、低风险路由不重复询问及性能档位有限升级 |
| V27 主动自适应路由 | 已验证 opt-in 单次升档、已报告token库门禁及配置/runner接线切片，完整运行链未完成 | `tests/config_provider_binding.rs`、`tests/model_call_controller_audit.rs`：默认关闭、显式同边界 factory、wire low→medium、相邻单次升档、新 Attempt/Route/PromptPack/Call、父证据和次数预算继承、两轮宿主质量验收、RequestHuman 不完成、安全/解析/动作错误不重调、预算与审计故障后不再发送、事件查询。`tests/reported_token_budget.rs`及`tests/reported_token_controller_boundary.rs`验证Option用量、checked累计/冲突/未知阻断、重复ID、store/progress修复后跨入口阻断、内存TaskRun恢复及实际child累计。`tests/reported_token_runner_config.rs`及两份provider runner测试验证缺省/null/0、类型拒绝不回显、prepare冻结、四路径零发送、两HTTP零resolver和正额度私有快照。`tests/provider_foreground_upgrade.rs`验证独立`prepare_upgrade()`纯观察入口、双重opt-in、可信宿主evaluator、实际两调wire/身份链、配置快照、同一transport及共享次数/token账本、失败和审计门禁；一次升档上限不是多次cooldown。CLI自动升档、统一采集、预留式硬上限、金额约束、跨进程恢复及live验收仍待完成 |
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

2026-11-09 bounded Markdown 门禁：恢复后基线全量 71 harness/893 项通过，旧中断 baseline.log 为空，不作证据。合成响应补齐 provider id/model 并加入合法写入对照后，`red-valid-response.log` 为 4 失败/1 通过，实际复现 tool-lock/独立 audit hardlink 前置门禁及工作区替换缺口；较早 `red.log` 的响应缺字段，不能作 post-response 行为 RED。初步 GREEN 是新 5 项通过、旧 runner 14 通过/1 失败（提前 prepare 拒绝导致旧 unwrap 失败），尚非全目标通过。扩展测试曾因 EventLog 不可 clone 编译失败，以及把既有 prepare 碰撞拒绝误 unwrap 而失败；均保留日志，不冒充产品回归或最终 GREEN。

最终 `green-final.log` 为 14+15 项通过；master 全量 `master-tests.log` 为 72 harness/907 passed/0 failed/0 ignored，严格 all-target clippy（`-D warnings`）、lib/bin build、fmt/diff 均通过，见 `master-{clippy,build,fmt}.log`/`master-static.exit`。14 项新增包含于 907 项，不重复相加。矩阵覆盖十目的地+optional state 的类型/别名、纯 API 快照无写入、check 消费后可用、scope 限制、双协议合法写入/替换拒绝及 usage 保留、progress callback 后零发送、resolver 两协议拒绝/零 token/历史身份零额外解析；错误分支是否复核还由实际源码审读证明，单靠最终原 Err 不能区分该控制流。规格终审和独立质量安全终审均通过，零剩余 Critical/Important/Minor；master 直接审读实际 diff 并运行验证，不以代理自报替代验收。

证据目录 `/temp/codex/steerbox-validation/markdown-runtime-gates-20261108/`，jobs=1、外置 `foreground-upgrade-20261102/master-target`；不使用真实配置/凭据。契约见 `BOUNDED_MARKDOWN_RUNTIME_GATES.md`，特别保留失败审计非零副作用与 runtime by-path 非原子保护限制；本片不接 CLI/live，提交态及集成复验另记持久台账。

2026-11-08 离线只读预检：隔离基线全量 70 个 harness/884 项通过；新增命令接线前真实行为 RED 为 1 失败/7 filtered（合法调用未识别），不是编译故障。最终 `cargo test --locked --offline --test readonly_preflight_cli --test provider_readonly_cli -- --test-threads=1` 为 9+8 项通过。规格审核发现存储 hardlink 测试被授权 README 的 nlink 门禁提前拦截，已改用非 workspace/non-runtime 的独立保护文件并复审通过，最终定向证据为 `green-reviewed.log`/exit 0；较早 `green-final.log` 不作为修正后证据。master 全量 71 个 harness/893 项通过、0 失败/忽略；严格 all-targets clippy（`-D warnings`）、lib/bin build、fmt/diff 均通过。日志目录 `/temp/codex/steerbox-validation/readonly-preflight-20261108/` 含 `baseline-tests.log`、`red.log`、`master-{tests,clippy,build,fmt}.log` 和 exit 收据；单 Cargo writer、jobs=1、外置 `foreground-upgrade-20261102/master-target`。新增 9 项已包含于 893 项，不重复相加。覆盖双协议、缺失/无效合成 env、loopback 无连接、八目的地 directory/symlink/hardlink、授权文件别名、runtime/workspace 冲突、缺失父目录、非法 scope/profile/参数和损坏历史；成功及拒绝路径均比对 fixture 目录项与文件字节。规格复审与独立质量安全终审均通过，零剩余 Critical/Important/Minor；master 直接审读 diff，调用链审核确认无 resolver/client/store-open/tool/Git 调用，快照与 listener 测试本身不等于证明所有副作用不存在。不宣称 live、可写性、历史身份冲突、跨进程恢复或 Phase 1 完成。提交态和本地集成复验另记持久台账。

2026-11-08 bounded只读provider循环：规格初审要求补StateStore/ExecutionQuery证据，补证后复核通过；独立质量安全终审及重启后最终增量复核通过，零阻断项。master在最终冻结源码与测试上重启后执行`cargo test --locked --offline --tests --quiet -- --test-threads=1`：69个harness、871项通过、0失败/忽略；严格all-targets clippy、lib/bin build及fmt/diff均退出0。项目外日志目录`/temp/codex/steerbox-validation/readonly-provider-loop-20261108/`包含`postcrash-{tests,clippy,build,fmt}.log`；复用项目外`foreground-upgrade-20261102/master-target`，jobs=1。旧`master-gated-tests.log`因WSL中断而截断含NUL，不作通过证据。19项新测试包含于871项，不重复相加；覆盖双协议两步、实际次数/token、action/verifier边界、七路径/symlink/hardlink/替换工作区、失败闭环与脱敏，并从实际事件检查完成/停止/失败的投影和身份链。`07-semantic-red.log`与`10-nonutf8-red.log`记录实际行为故障；早期编译/fixture失败不冒充行为RED。补证初次17/2失败是policy事件可选attempt字段断言过严，非运行缺陷，保留于`13-projection-evidence-initial-assertion.log`，补证后19项通过见`13-projection-evidence.log`，最终Unix测试门禁后的19项通过见`14-unix-test-gate.log`。非Unix实际行为未验证，无真实配置、密钥或live调用；只读PolicyGate允许路径实际运行，但不声称新runner诱发过PolicyDenied。CLI、state文件落盘、跨进程恢复、通用工具及完整Phase 1仍未完成；提交态复验另记持久台账，契约见`BOUNDED_READONLY_PROVIDER_LOOP_CONTRACT.md`。

2026-11-08 普通foreground失败收尾：原foreground基线13项通过；行为RED为12通过/10失败（缺终态事件），修复后相关完整targets共128项通过（lib7、foreground23、routing21、model-call审计72、token-boundary5）。独立规格和质量安全审核通过，无Critical/Important/Minor发现。master在冻结源码独立执行`cargo test --locked --offline --tests --quiet -- --test-threads=1`：68个harness、852项通过、0失败/忽略；严格all-targets clippy、lib/bin build、fmt/diff全部通过。日志目录`/temp/codex/steerbox-validation/foreground-failure-close-20261108/`含`baseline.log`、`red.log`、`regressions.log`和`master-{tests,clippy,build,fmt}.log`；`green.log`仅筛选运行，不当作全目标结果。使用项目外`foreground-upgrade-20261102/master-target`、jobs=1。新增13项包含于852项，定向结果不重复相加。覆盖双协议认证/429/非法决策、零token零发送/解析、进度/终态审计故障、三层状态和预算/证据保留、日志修复后不重发、非Running不覆盖；既有合法fallback仍通过。不包含构造期间初始进度错误、持久恢复、live或Phase 1完成；契约见`FOREGROUND_FAILURE_CLOSURE_CONTRACT.md`。提交态复验另记外部日志及持久台账。

2026-11-08 resolved routed候选失败收口：定向baseline为4通过；校正旧binding fixture的精确错误断言后，行为RED为5失败/3通过，修复后focused8项、完整model-call审计72项通过。独立规格和质量安全审核通过，无未解决发现。master独立执行`cargo test --locked --offline --tests --quiet -- --test-threads=1`：68个harness、839项通过、0失败/忽略；严格all-targets clippy、lib/bin build、fmt/diff均通过。日志位于`/temp/codex/steerbox-validation/routed-resolution-close-20261108/`：`baseline.log`、`red.log`（保留首次及校正后两次运行）、`green.log`、`master-{tests,clippy,build,fmt}.log`。复用项目外`foreground-upgrade-20261102/master-target`，jobs=1。新增4项包含于839项，不重复相加。覆盖四出口、原错误身份、三层Failed、预算与原失败证据保留、无额外child/send、审计故障后修复日志仍拒绝重发及合法fallback对照。不代表普通foreground失败收尾、其他binding/adaptation出口、持久恢复、live或Phase 1完成；契约见`RESOLVED_ROUTED_FAILURE_CLOSURE_CONTRACT.md`，提交态复验另记外部日志与台账。

2026-11-08 resolved模型白名单修复：真实行为RED为5失败/2通过（含既有upgrade和允许路径对照），修复后focused7项、整个model-call审计套件68项通过。独立规格和质量安全终审通过；质量建议的三处错误断言已收紧为`NoCompatibleModels`并完成增量复审。master在最终冻结源码独立执行`cargo test --locked --offline --tests --quiet -- --test-threads=1`，68个harness、835项通过、0失败/忽略；严格all-targets clippy、lib/bin build、fmt/diff均退出0。日志在`/temp/codex/steerbox-validation/resolved-allowlist-20261108/`：`implementer-behavior-red-final.log`、`implementer-focused-minor-green.log`、`implementer-audit-regression.log`、`master-final-{tests,clippy,build,fmt}.log`；复用项目外`foreground-upgrade-20261102/master-target`、jobs=1。新增6项包含于835项，不重复相加。旧中断的编译日志和空baseline日志不作运行证据。覆盖request/route两个方向的拒绝、resolver前门禁、fallback禁用且attempt预算耗尽时保留原错误、空/允许名单、失败收口审计故障后禁止重发；不代表其他resolver/binding错误收口或live验收完成。契约见`RESOLVED_MODEL_ALLOWLIST_CONTRACT.md`；提交态复验另见外部`committed-*`日志和持久台账。

2026-11-07 显式foreground单次升档接线：源码规格及质量安全终审通过；修复disabled-opt-in测试原先被无效配置提前拒绝的问题后，质量增量复核通过，无阻断项。master独立审读生产diff和新增断言，并在最终版本执行`cargo test --locked --offline --tests --quiet -- --test-threads=1`：68个harness、829项通过、0失败/忽略；严格all-targets clippy、lib/bin build、fmt和diff检查均退出0。项目外目录`/temp/codex/steerbox-validation/foreground-upgrade-20261102/`保留`master-final-{tests,clippy,build,fmt}.log`，独立`master-target/`、jobs=1。新增24项包含于829项，不重复相加。`implementer-api-red.log`为缺API的有效compile-RED；早期`implementer-red.log`是无效auth_ref夹具失败，旧中断`red.log`不是完整RED；不声称行为级先失败。修正测试门禁另保留`implementer-disabled-fixture-{red,green}.log`及24项GREEN。契约见`FOREGROUND_EFFECT_UPGRADE_CONTRACT.md`。仅合成配置、fake/unavailable transport/resolver；不代表CLI自动质量调度、live provider、完整恢复或Phase 1完成，提交态复验以交付提交及外部`committed-*`日志为准。

2026-11-01 单次升档切片：规格及质量源码审查通过后，主 Agent 在冻结实现上独立执行 `cargo test --locked --offline --tests --quiet`（62 套、751 项通过、0 失败/忽略）、`cargo clippy --locked --offline --all-targets -- -D warnings`、`cargo build --locked --offline --lib --bins`、`cargo fmt --all -- --check` 与 `git diff --check`，均通过。WSL2 重启前未收取的全量测试不作为证据；重跑日志及独立 target 保存在项目外 `/temp/codex/steerbox-validation/effect-upgrade-20261101/`，编译 jobs=1。测试使用合成配置/fake transport，没有真实凭据读取或 live LLM 调用；不代表完整 Phase 1、生产部署或服务端能力验收。

2026-11-01 已报告token门禁切片：独立规格与质量源码审核、边界回归增量审核通过；master在最终冻结代码及新增独立测试上执行`cargo test --locked --offline --all-targets -- --test-threads=1`，64套/774项通过、0失败/忽略；`cargo clippy --locked --offline --all-targets -- -D warnings`、`cargo build --locked --offline --lib --bins`、`cargo fmt --all -- --check`与`git diff --check`均通过。独立target为项目外持久目录`/temp/codex/steerbox-validation/reported-token-budget-20261101/master-target`，jobs=1；证据位于该target的父目录：`master-final-tests.log`、`master-final-clippy.log`、`master-final-build.log`。纯账本6项、controller audit 62项和独立boundary 5项包含于全量，不重复相加。无真实凭据或live调用，不代表Phase 1整体完成、单请求token硬上限或金额预算。

2026-11-01 已报告token配置/runner接线切片：master在冻结代码上独立执行`cargo test --locked --offline --tests --quiet`，783项通过、0失败/忽略（65个test harness，其中1个无测试）；严格`cargo clippy --locked --offline --all-targets -- -D warnings`、`cargo build --locked --offline --lib --bins`、`cargo fmt --all -- --check`和`git diff --check`全部退出0。目录为`/temp/codex/steerbox-validation/reported-token-config-20261101/`，master独立target为其`master-target/`、jobs=1，日志`master-tests.log`、`master-clippy.log`、`master-build.log`、`master-fmt.log`和`master-diff.log`。实现者focused为13+15+4=32项，包含于master全量，不重复相加；`config-red.log`、`runner-zero-red.log`、`markdown-zero-red.log`和`outcome-red.log`保留先失败证据，格式化后focused结果见`focused-final.log`。仅使用合成配置、fake transport和返回unavailable的计数resolver；未读取真实配置/凭据或调用live。每次runner仍新建TaskRun，不代表跨运行配额、同ID重跑保护、持久恢复、在途请求硬上限、金额预算或完整Phase 1。此前64套/774项保留为库切片历史，不能冒充新接线验收。

2026-11-02 provider历史身份门禁切片：旧代码合成回归实际观察到同身份二次发送计数由1增至2，随后才发生重复模型审计ID错误（`repeated-send-red.log`、`identity-red.log`）。冻结实现的独立规格及质量安全审核通过（blocking/Critical/Important/Minor均0）；主代理重新执行`cargo test --locked --offline --tests --quiet -- --test-threads=1`，66个harness、790项通过、0失败/忽略；`cargo clippy --locked --offline --all-targets -- -D warnings`、`cargo build --locked --offline --lib --bins`、`cargo fmt --all -- --check`及`git diff --check`全部退出0。前次失联执行未作为完整验收证据，本次执行会话38044已收取退出0。日志位于`/temp/codex/steerbox-validation/provider-identity-20261102/master-{tests,clippy,build,fmt,diff}.log`；复用主代理既有项目外`reported-token-config-20261101/master-target`依赖缓存，jobs=1，未使用实施者target。新增7项参数化回归及相关focused 69项均包含于全量，不重复相加。四runner的重复身份零新增send、业务文件字节不变、typed aggregate与成败模型历史匹配、坏历史固定错误、两HTTP零resolver及预算优先级有实际断言；协议/路径检查的顺序由源码审核确认，没有新增组合优先级测试。提交验收状态以Git记录及已提交树复验为准；不代表并发原子认领、完整恢复、live provider或Phase 1完成，详见`PROVIDER_RUN_IDENTITY_GUARD_CONTRACT.md`。

2026-11-02 loop-scoped步骤投影切片：`loop-step-projection-20261102/red.log`保留原始两项失败（第二loop的同名步骤被误判重复）；冻结实现独立规格及质量安全终审通过（blocking/Critical/Important/Minor均0）。主代理执行`cargo test --locked --offline --tests --quiet -- --test-threads=1`，67个harness、805项通过、0失败/忽略；严格all-targets clippy、lib/bin build、fmt/diff均退出0，执行会话57008已收取。证据在`/temp/codex/steerbox-validation/loop-step-projection-20261102/master-{tests,clippy,build,fmt,diff}.log`，复用主代理项目外`reported-token-config-20261101/master-target`、jobs=1。focused 54项、新增14项投影回归及1项双协议foreground连续运行回归均包含于全量，不重复相加。覆盖同名步骤独立归属/终态、精确API和execution查询、歧义与字段错误拒绝、新嵌套JSON往返及旧非空flat拒绝。提交验收以Git记录及已提交树复验为准；不改变provider/controller/call ID，不涉及真实凭据、live或完整TaskRun恢复，公开API及旧投影重建边界见`LOOP_SCOPED_STEP_PROJECTION_CONTRACT.md`。

## 验证用例

2026-10-19 provider runner 回归补充：`provider_foreground_runner` 8 项与 `provider_markdown_runner` 11 项通过。前者验证 OpenAI-compatible/Anthropic 注入式 runner 的现有审计链不回显 auth_ref；后者验证 provider runner 接入的 `tool-calls.jsonl` 中 mutation 与 verification 的 metadata-only 生命周期、identity 分离，以及 verification 失败的 `started/failed`。这些测试不替代 live provider、后台 worker、真实多步 coding loop 或完整 DLP 验收。

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

### V7.1 单文件 snapshot 与显式条件恢复

独立 `WorkspaceSnapshotStore` 的本地 Unix-only 验收入口：

```bash
CARGO_TARGET_DIR=/tmp/steerbox-phase1-target CARGO_BUILD_JOBS=2 \
  cargo test --locked --offline --test workspace_snapshot --test workspace_edit
```

master 提供的本次 focused 结果为 snapshot 42 项、workspace edit 11 项，合计 53 passed；修正新测试 strict clippy 警告后，master 的 `cargo clippy --locked --offline --lib --bins --tests -- -D warnings` 也已通过。随后 master 的 `cargo test --locked --offline` 退出 0，日志 `/tmp/steerbox-phase1-snapshot-full-20261014.log` 汇总为 563 passed、0 failed、0 ignored；53 段 test-result 包含 0 项 doctest 段，不称为 53 个测试 targets。lib/bin build、`cargo fmt --all -- --check` 和 `git diff --check` 也由 master 确认退出 0；构建/测试使用上述 `/tmp` target、jobs=2。这些回归结果不等于 controller snapshot 接线或完整恢复验收。

已覆盖的 focused 场景与范围：

- 既有 UTF-8 Markdown 文件 capture/reopen/显式 restore 保留 CRLF 原始字节；capture 不修改源文件，公共 metadata/Debug/错误不回显原文，重复 ID 不覆盖。
- approval false、project/goal/canonical workspace 不匹配、capture 后 scope 收紧、当前文件被人类修改导致 content hash mismatch 均拒绝，目标保持当前字节；approval/expected hash 来自 trusted caller，不证明 harness 曾验证该当前状态。
- traversal、非 Markdown/out-of-scope、缺失/非普通文件、文件/祖先/store symlink、hardlink、非法 ID、损坏/篡改 record 和 body/hash/size 不一致均拒绝；源码/当前目标/record 有界读取。
- 新增独立断言区分 `CurrentHashMismatch`/`BindingMismatch`/`InvalidRecord`，并验证当前目标 hash 相符但已 hardlink、store 祖先被替换为 symlink 时 restore 仍拒绝且不改当前正文；审批拒绝只在 controller 授权路径验证，不属于 snapshot storage primitive。
- 正文上限 1 MiB，JSON private record 上限 6 MiB + 64 KiB，测试包含边界正文、转义膨胀和超限 record。Unix 新 store/record 的 owner-only 权限、现有过宽权限拒绝及当前目标 permission mode 保留已有 focused 覆盖；foreign-owner store/record 拒绝由 root-only 测试覆盖，master 确认本次 effective UID=0，测试实际 chown 仅涉及 TempDir；非 root 运行该用例会提前返回，不能把那种结果当作 owner 场景的运行时证据。非 Unix 实现 fail-closed，未宣称 Windows ACL 已验证。
- snapshot `before_hash`/restore `expected_after_hash` 为 `SHA256(file bytes)`；旧 controller checkpoint 为 `SHA256(relative_path + NUL + file bytes)`，不可混用。body hash 是完整性而非认证，本地 owner/权限不是加密或同用户/root 隔离，不能抵御同用户同时修改 body/hash。

限制：private body 仅在 workspace 外本地存储，不自动注入 prompt/事件/trace/summary/model-call store。恢复只保留 Unix permission mode，不保留 inode/owner/ACL/xattrs；精确替换不是恶意并发 CAS、跨进程 writer 锁或完整 crash transaction，replacement 后 sync 失败可能已改变目标，须检查后重试，该错误阶段未以本次 focused 记录宣称独立故障注入通过。controller 仅在显式配置 snapshot store 时 capture，restore 仍由 trusted caller 显式调用；verification failure 不自动 rollback，metadata 查询仍只读，TaskRun/Attempt/预算/进程及外部副作用恢复未完成。

### V7.2 bounded Markdown controller 的 opt-in snapshot 接线

当前 controller 接线的 focused 验收入口：

```bash
CARGO_TARGET_DIR=/tmp/steerbox-phase1-target CARGO_BUILD_JOBS=2 \
  cargo test --locked --offline --test controller_snapshot_integration --quiet
```

当前现场结果为 `16 passed; 0 failed; 0 ignored`。16 项覆盖：

- 配置 `with_snapshot_store(...)` 后，workspace edit 在 mutation 前 capture，事件仅记录 snapshot metadata；显式 restore 可恢复原始正文
- capture 前置失败（包括公开 snapshot root）不会修改目标文件，也不启动 tool ledger
- 未配置 snapshot store 时保持既有 edit 行为，不产生 snapshot capture 事件
- verification failure 产生 `RestoreRequired`，保留坏内容，不自动调用 restore
- Paused 或 terminal controller 拒绝显式 restore
- `approved=false`、错误 expected current hash、未绑定 snapshot 等 restore 失败不覆盖目标文件
- failed restore 只写 metadata-only `workspace_snapshot_restore_failed` 事件，不写入文件正文
- 非法 expected hash 不写入 restore 请求事件；capture 事件或 checkpoint 失败时 binding 不注册；恢复后成功事件写入失败时返回固定 `side_effect_uncertain` 错误
- snapshot 正文保留在 workspace 外的 private record，不进入公共 EventLog、Trace、summary 或模型调用记录

显式 restore 还要求 trusted caller approval、当前 controller run 的 snapshot binding、snapshot integrity 和 expected current raw-bytes hash；人类在 agent edit 后修改目标文件会触发 hash mismatch 并拒绝覆盖。snapshot hash 是 `SHA256(file bytes)`，不能与 legacy checkpoint 的 `SHA256(relative_path + NUL + file bytes)` 混用。唯一 controller 授权入口的验收顺序为：`PolicyGate → ApprovalStore scope/status 校验（强制 Once）→ snapshot read-only preflight → ledger planned → restore request event → ledger started → Once approval consume → prepared snapshot replacement`。任一 mutation 前审计写入失败或 snapshot preflight failure 时，审批保持可重用且 workspace 不变；审批消费后发生 physical restore 失败不回滚审批；后置审计失败返回 typed `WorkspaceSnapshotAuditUncertain`。controller 不再暴露 `approved: bool` restore API；底层 `WorkspaceSnapshotStore` primitive 仅作为独立存储层能力保留，不是 controller 或 model action 授权路径。该 slice 不接受 model action，也不提供 verification failure 自动 rollback、TaskRun/Attempt 恢复、进程/跨进程恢复、并发 writer CAS、完整 crash transaction 或外部副作用补偿；replacement 后 sync failure 仍需检查现场。

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

ToolCall identity 回归需满足：同一次受控工具调用的 started/completed、started/failed 或 started/verification_failed 使用同一个 `tool_call_id`；只读工具以 `tool_call_requested`、workspace-edit mutation 以 `model_action_started` 的 sequence 派生 identity，verification 使用自己的只读请求 sequence；同一 `step_id` 重复调用即使 `tool_id` 相同，也必须因对应起始事件 sequence 不同而产生不同 ID；非法字符、控制字符和超长 identity 必须在 policy、checkpoint 或请求事件前拒绝。该检查不证明跨进程恢复、全局唯一性、通用写入/Bash 或完整 DLP。

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

- 跨任务历史表现驱动的在线自适应多模型路由质量
- 多智能体协作质量
- 向量记忆召回率
- 图记忆推理能力
- 自动安全响应闭环
- 插件市场兼容性
- 企业级多租户隔离
- OS 级 checkpoint / restore
- 交互式 TUI/Web control plane

这些能力只检查是否有接口占位或文档边界；第一阶段的静态 registry、规则式路由、prompt resolution 和调用审计仍属于必验项。
