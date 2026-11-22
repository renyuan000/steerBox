# 崩溃恢复：当前证据与实施边界

核查日期：2026-11-22。ToolCallRecordStore T1 基线 `4fe81d9`，限定切片已通过独立审核与全量验证；
ModelCallStore M1 增量基线 `0a1ca50`，限定切片已通过独立审核与全量验证；
EventLog 安全 I/O 增量基线 `76f31dc`；
三 store 安全 I/O 增量基线 `bc17025`，限定切片已验收；
历史审计文件同步增量基线 `904606a`，EventLog 尾部门禁基线 `5923dad`，
私有 transcript 库增量基线 `7998508`。
本文的“当前”不是完整 Phase 1 验收，也不把设计目标标为已实现。

## 现在能恢复什么

下表仅列已验证能力；EventLog 与三 store 安全 I/O 的范围及兼容性变化见下方。

| 对象 | 已实现 | 不代表什么 |
|---|---|---|
| EventLog | 五个实际 I/O 入口逐组件 NOFOLLOW、dirfd-relative 打开，同 fd 验证 regular/nlink=1；完整追加后文件与父目录均同步，再推进序号与 ACK；重开完整历史后接续序号 | 每次访问独立 pin，不是生命周期身份绑定；没有跨进程单写者锁、事务或完整自动恢复；MAX 序号仍有写后溢出边界 |
| ModelCallStore | 成败 JSONL 与两实际锁安全打开；持锁完成历史校验、判重和文件/父目录同步后 ACK；相同记录 retry 同步原历史 fd、不改字节 | 查询可创建 RW sidecar；不防锁替换，无跨文件事务、坏尾修复或完整恢复 |
| ToolCallRecordStore | tool JSONL/真实锁同 parent 安全打开，实际 fd 检查 regular/nlink/UID/mode；持锁至 JSON+LF、文件/parent 同步后 ACK | constructor 预检非 anchored 身份保证；不判重、不防锁替换，无尾修/事务/完整恢复 |
| StateStore | 从合法事件重建 loop/step 投影；临时文件同步、rename、Unix 父目录同步 | 不包含完整 TaskRun/Attempt、模型上下文、预算和租约 |
| checkpoint / side-effect / trace | 共享 JSONL 追加/历史读取逐组件 NOFOLLOW、dirfd-relative 打开，同 fd 验证 regular/nlink=1；目录链屏障、完整追加后文件同步再父目录同步；可查询及标记 `uncertain` | 每次访问独立 pin，不是生命周期身份绑定；无跨 store 事务、并发写者协调或完整恢复，不承诺抗 OS 掉电 |
| workspace snapshot | 可选单文件原文快照；文件及目录同步；显式授权 restore | controller 的 snapshot binding 仍在内存，重启后不能据此直接 resume |
| provider 身份门禁 | 拒绝顺序复用已有 run 身份 | 不证明旧请求没发送，也不证明旧动作没执行 |

源码依据：`src/event_log.rs`、`src/model_call_store.rs`、`src/tool_call_store.rs`、`src/state_store.rs`、`src/audit_store.rs`、
`src/workspace_snapshot.rs`、`src/loop_controller.rs`、`src/provider_runner.rs`。
CLI 的 execution 查询和运行中 inbox Resume 不是进程重启后的恢复入口。

## ToolCallRecordStore T1 安全 I/O（2026-11-22，限定切片已验收）

基线 `4fe81d9`，仅 tool JSONL 与其一个真实 `.lock`。复用逐组件 NOFOLLOW、
dirfd-relative 打开，同次操作在一个 pinned parent 派生锁与数据；实际 fd 验证
regular/nlink=1 后再验证当前 UID、owner-only。完整 JSON+LF → 同 fd `sync_data`
→ 最终 pinned parent 同步 → ACK，全程持真实 RW advisory lock，错误返回后释放。
constructor 保留 metadata-only 即时诊断，不要求打开已有 data，也不是 anchored
metadata 身份保证；实际操作重新固定目录并校验 fd，不能用预检查替代实际安全门禁。
构造时准备并同步完整目录链；后续操作不重做整链，仅最终 parent 屏障。新建目录/文件
请求 0700/0600，受 umask 收窄，不 chmod 既有对象。非 Unix 实际 I/O fail-closed。

append 明确由 O_RDWR 改为最小 O_WRONLY：owner-only 0200 data 可追加但不可查询，
0400 可查询但不可追加，0000 仍可 metadata-only 构造；lock 始终要求 RW。
不扫旧历史、不判重，同 ID 的生命周期及完全相同记录均保留；同步失败后的显式再次
append 也会新增一行，不提供 ModelCallStore 的 identical retry 合同。
cap 在方法 I/O 前拒绝，zero 先安全锁后跳过 data；缺失 data 不创建业务 JSONL，
查询可创建 sidecar，构造后缺失 parent 不重建。保留 offset 前解析、物理行号、UTF-8
错误、空白行及页满早停。新增 I/O 错误仅保存阶段和 ErrorKind，不扩成通用 DLP。

不防 sidecar unlink/替换、不提供 timeout、生命周期绑定或跨 store 事务。
失败可能留下部分/完整字节，不自动重试或回滚，也不表示工具尚未执行或已撤销。
不做严格 LF 门禁或补尾：可解析的无 LF 尾行仍可读，后续追加可能拼接损坏，未修复。
同步/锁成本未测，无 Windows/macOS 实机验收，不新增 provider/live 写 CLI 或完整恢复。
首轮定向为真实 seam 5 项、既有 store 14 项、路径 11 项和 runtime 4 项通过；
补充锁 owner/mode 后路径 13 项通过。省略目录同步的 mutant 被 4 项失败拦截，
精确恢复后通过。最终全量 78 个 harness、1079 项通过，0 失败/跳过；严格 all-targets
Clippy、实际 lib/bin build、fmt/diff 通过，独立规格/质量审核无剩余阻断。
日志与提交/合入复验见 `/temp/codex/steerbox-validation/tool-store-path-20261122/RECEIPT.md`。

## ModelCallStore M1 安全 I/O（2026-11-22，限定切片已验收）

范围为成功 JSONL、failure JSONL 及二者的真实 advisory lock；覆盖构造、两类追加、
全量/分页查询与 run identity 扫描。逐组件 NOFOLLOW、dirfd-relative 打开，在同 fd
验证 regular/nlink=1；新目录 0700、新数据和锁 0600，不强制迁移既有普通文件权限。
`open` 只准备父目录，不打开数据或锁；保留目录 leaf 在实际读写时才报错的行为。
每次 `open` 的父目录准备同步完整遍历链，包括既有目录；后续操作不重做整链屏障，
而是在一次 pinned parent 内派生锁与数据 sibling，持锁完成扫描、判重和文件/父目录屏障。
实际新追加为完整 JSON+LF → 同 fd `sync_data` → pinned parent 同步 → ACK。
相同记录 retry 完整校验历史后，同步原 history read fd 和 pinned parent 才 ACK；
不重新打开数据、不新增记录、不改字节。任一屏障失败均报错，可能已留下部分或完整字节。

查询仍需可读写锁，可创建 sidecar；缺失业务 JSONL 返回空/false，不创建业务记录。
零 limit 先获取安全锁，再跳过数据打开；不是 EventLog 的无 I/O 短路。
分页仍解析并校验 offset 前的非空记录，取满 limit 后停止；identity 命中后仍扫描
两份成败历史。保留 UTF-8、物理 JSON/contract 行号和 `.lines()` 的无 LF 尾行语义。
已有只读数据可查询并做 identical retry，但锁仍要求 RW，新记录追加仍可能被权限拒绝。
非 Unix 实际 I/O fail-closed，不宣称 Windows/macOS 实机验证；同步及锁等待成本未测。

锁只在单次、单文件操作内有效，无 timeout；不防 sidecar 被 unlink/替换后形成不同锁，
不提供生命周期身份绑定、跨成败/store 事务或任意同 UID 恶意修改防护。
retry 屏障不是尾行修复，也不是自动安全重试；不补 LF、不截断、不恢复坏尾。
完整无 LF 尾 JSON 后再追加不同记录仍会拼接损坏；本片仅用特征测试记录，未修复。
本片不新增 provider 请求、正文采集、live 写 CLI、恢复检查器或 TaskRun/Attempt/预算续跑。
最终库 67 项（含共享 helper 新增 6 项、模型新增 12 项），路径专项 21 个入口（含 3 个子进程
probe），已有模型/审计/身份回归及全量 77 个 harness、1061 项通过，0 失败、0 跳过；严格
all-targets Clippy、lib/bin build、fmt/diff 通过。retry 目录屏障 mutant 两项失败，锁 NOFOLLOW
mutant 路径矩阵失败，精确恢复后通过；独立规格/质量与文档审核无剩余阻断。
最终日志、提交及合入复验见 `/temp/codex/steerbox-validation/model-store-path-20261122/RECEIPT.md`。
不外推为 Phase 1 完成或 OS/VM 掉电恢复验收。

## EventLog 安全 I/O（2026-11-22，限定切片）

覆盖 `open`、`append`、`read_all`、`contains_run_identity`、`read_page`，
复用 crate-private 安全目录 fd 原语，而非复制路径检查或 unsafe 打开逻辑。
完整路径先验证；逐组件 NOFOLLOW、dirfd-relative 打开；同 fd 验证 regular/nlink=1。
拒绝 symlink、hardlink、device、FIFO、`..`、NUL 和非法 basename，缺失查询不创建节点。
`open` 允许已有只读历史；缺失才创建，并在文件和父目录同步后经同一 parent 读取扫描。
`open` 与 append 的父目录准备均同步完整遍历链，包括既有目录；成本尚未测量。
追加顺序为完整 JSON+LF → 同文件 `sync_data` → pinned parent 同步 → 更新序号 → ACK。
写入或同步失败可能已有字节留下，内存序号不推进，不自动回滚或盲重试。

保留原解析差异：只有 `open` 拒绝未封口尾行；三个查询仍接受可解析的无 LF 尾行。
身份查询即使命中也扫描全部历史；分页 offset 按非空行计数，页前不解析 JSON，
但不跳过 UTF-8 错误，取满 limit 即停止取行/解析；不承诺 BufReader 完全不预读。
所有错误保留物理行号/类别。零 limit 在任何 I/O 前返回空；非 Unix 的实际 I/O
返回 `Unsupported`，不使用较弱路径回退。新文件 0600、新目录 0700，
不强制改动既有普通历史权限。Linux 验证不等于 Windows/macOS 实机验证。
新 I/O 错误仅含阶段与 ErrorKind；目录叶节点保留 `IsADirectory`。
共享 audit 读取目录的错误 kind 文本也由 `InvalidInput` 变为 `IsADirectory`，拒绝行为不变。

本片不增加每次 append 的全历史复验、生命周期身份固定、多写者锁、事务、sticky 失败
或安全重试。已有 MAX 序号边界仅加特征测试：历史最大为 MAX−1 时，追加可能写出并同步
MAX 记录后返回 `SequenceOverflow`，重新 open 也拒绝；未前移溢出检查，不称为已修复。
不放开 live 写 CLI，也不构成 TaskRun/Attempt、预算、授权或 OS/VM 掉电恢复验收。

## 三 store 安全 I/O（2026-11-22，限定切片已验收）

本片仅覆盖 CheckpointStore、SideEffectLedger、TraceStore 共用的 JSONL 追加与历史读取：
dirfd-relative 逐组件 NOFOLLOW 打开，在同一 fd 验证 regular file 与 nlink=1；
每次追加在写入前同步遍历的目录链（包括既有目录），补齐此前失败可能遗留的建项屏障；
完整追加后须同文件 `sync_data` 再最终父目录同步才返回成功。最终父目录写前、写后均同步。
缺失读取不创建文件或目录；拒绝 symlink、hardlink、device、FIFO、`..` 和无 basename 路径，
非 Unix 的共享追加/读取返回 `Unsupported`，包括缺失历史读取，不使用较弱路径回退；
构造函数签名不变。新目录 0700、新文件 0600，不强制迁移既有普通文件权限。
Linux 本轮已验收，不能外推为 Windows/macOS 实机验证通过。目录屏障次数随深度增长，
性能成本尚未测量，不对每个流式 token 使用此审计接口。

每次访问独立 pin，不提供 store 生命周期的目录/文件身份绑定，也不承诺抵御任意同 UID
恶意进程。文件或父目录同步失败时可能已有字节写出，不代表追加完全未发生；
sticky 失败状态、坏尾恢复、安全重试、并发写者协调及跨 store 原子性不在本片。
该历史切片不包含 EventLog（其后续范围见上节）；model/tool、真实锁文件及
State/workspace 替换端仍须独立验收；
`RunSummary::write_json/read_json` 的普通路径 JSON I/O 也不在本片，不能据此称整个
`audit_store.rs` 或全部 audit 错误均已加固/脱敏。
本片不放开 live 写 CLI，不恢复 TaskRun/Attempt、预算或授权。

## 审计追加的文件同步屏障（2026-11-12，历史切片）

`CheckpointStore`、`SideEffectLedger`、`TraceStore` 共用的 `append_jsonl` 在完整记录及
换行写入后，对同一打开文件执行 `sync_data`；写入或同步失败均返回错误，新写入/同步
错误只保留阶段和 ErrorKind，不带底层错误正文。controller 原有写前 checkpoint、trace、
ledger 门禁因此会阻止该次实际编辑。trace/ledger 前可能已有 `model_action_started`，
它只是意图，不证明编辑发生；写后审计失败则不能宣称文件未变或已经回滚。

失败追加可能留下部分或完整字节；helper 没有 sticky 状态、自动重试或坏尾保护，调用者
不得盲重试。首次创建目录项、可信路径/文件类型、历史读取、并发写者及跨 store 原子性
仍待加固。本片不开放 live 写 CLI，也不恢复 TaskRun/预算/授权。每条 trace 同样新增阻塞
同步，尚未测量延迟；不能宣称没有性能影响，也不在每个流式 token 上调用此审计接口。

## 私有正文日志的新增边界

`transcript_store` 已增加独立存储库：获准正文可逐条写入私有 JSONL，默认首次 dirty 后
1s 或累计 64KiB 同步；关键消息及完整模型请求/响应成功同步后才确认，显式 close 同步并
回收后台线程。它只同步自己的文件；首次创建还同步固定的父目录，不执行全系统 sync。
序号水位区分 written 与 durable；部分写入或同步失败后拒绝继续追加。

这不是实际聊天/provider 的自动记录，也没有恢复读取/续接；整流敏感检查、受保护引用、
轮转和 opt-in 接线仍待实现。默认 1s 是调度目标，不是故障下最大损失承诺；同 UID 恶意
篡改、WSL/VHDX 损坏以及跨 store 原子性也未解决。详见 `PRIVATE_TRANSCRIPT_STORE.md`。

## EventLog 尾部门禁（2026-11-09，历史切片）

旧 `EventLog::open` 使用去除换行的 `.lines()` 读历史，完整 JSON 即使缺少最后换行也会被接受。
下一条事件直接追加时没有前置分隔符，会把两条 JSON 连到同一行。

现在 `open` 保留读取到的换行；可解码为 UTF-8 的非空最后一行若没有 `\n`，包括只有空格的
未封口行，返回 `EventLogError::UnterminatedRecord { line }`。这个错误只含行号，不含原始正文；
非法 UTF-8 在读取阶段返回 `Io` 错误，不做有损解码。
已有异常记录不被删除、不补换行、不被自动跳过，正常 JSON 但缺换行的导入文件同样拒绝；
外部穷尽匹配公开错误枚举的调用方需处理新增变体。空文件、已封口空行和 CRLF 保持可用。
已封口的非法 JSON 中间行仍报错，不允许借“尾部恢复”跳过内部损坏。

这只是 **重开时拒绝继续扩大损坏**，不是日志修复、坏尾隔离、事务、路径安全或自动 resume。
已打开对象之后的外部并发修改、失败追加后的原对象重试不由这次门禁保护；仍不得多写者使用。
`read_all/read_page` 没有因此变成独立的坏尾诊断或完整前缀恢复 API。

## 进程退出与 OS 崩溃不能混为一谈

- `tests/event_log_recovery.rs` 的强杀回归在子进程完成 append、报告成功后发送 SIGKILL，
  确认非正常退出，再由父进程重开历史并从下一序号追加。它不依赖正常析构。
- 这证明受控的 **进程死亡后重开**，不是杀内核、WSL VM 掉电、磁盘掉电或 VHDX 损坏测试。
- 文件同步与目录条目的持久性是不同屏障。三 audit store 的共享 JSONL 追加和后续
  EventLog 五入口片、ModelCallStore M1 和工具 T1 已增加并测试目录屏障，
  其余 runtime 路径仍须分别验收，
  多 store 之间没有统一提交事务。
- `sync_*` 失败时可能已有部分数据写出；不能把错误解释为“完全没发生”。同一磁盘上的日志
  也不是该磁盘损坏后的备份。WSL 数据盘损坏等故障需要另一故障域的备份与恢复演练。

## 不能盲目重试的窗口

controller 在写前记录 checkpoint，执行文件编辑后再记录完成事件和副作用结果。
若文件已替换但完成记录未落盘，历史可能只留下 `started`。同理，在途 LLM 请求可能已经
到达 provider 或产生费用，而本地结果尚未记录。**缺少 completed 不等于没有执行。**

后续恢复器必须先对账，而不是给旧工作换新 run ID 再执行：

| 恢复时证据 | 后续设计要求，尚未实现 |
|---|---|
| 只有状态投影缺失/落后，权威事件完整 | 校验身份与事件边界后重建派生投影，不重做工具 |
| 写入 started、没有可靠完成记录 | 比较当前文件 hash、写前 hash、预期写后 hash及快照引用 |
| 能证明预期写后结果且验证通过 | 补明确的恢复对账记录，不再执行同一写入 |
| 当前文件与写前/预期写后均不符 | 标记冲突/uncertain，阻断该动作，不能自动覆盖别人的修改 |
| 在途外部请求或非幂等副作用 | 查询外部结果/使用已持久化的幂等键；不能证明时保持 uncertain |
| 原文坏尾/内部损坏或身份不一致 | 保留证据并阻断，修复应是独立、可审计的操作 |

后台不能询问时，不应以自动放行为替代：只搁置无法安全判定的分支，记录原因；
只有依赖关系、授权和预算允许的独立工作才可以继续。当前尚未实现这个恢复调度机制。

## 接下来的交付顺序

1. **实际 runtime I/O 加固与关键审计同步**：固定可信目录 fd、dirfd-relative 打开及
   打开后文件身份/类型校验；覆盖 event、model 成败记录、tool、checkpoint/trace/ledger
   及真实锁文件，不能遗漏失败追加和历史读取。共享审计 helper 的文件同步已补齐；
   三 store 与 EventLog 的安全打开、历史读取及目录同步已逐片实现并测试，不能替代其余路径验收。
   ModelCallStore 成败记录及两锁的 M1 已通过独立审核与全量验证；其余门禁不因此解除。
   ToolCallRecordStore T1 已通过独立审核与全量验证，不能代替剩余路径验收。
   State/实际 workspace edit 的替换端也要独立验收。
   仅末级 NOFOLLOW 或 metadata 预检不等于父路径安全，更不等于抵御任意同 UID 恶意进程。
2. **只读恢复检查器**：识别完整前缀/坏尾、stale state、未闭合请求/工具/副作用、缺失快照；
   不解析凭据、不联网、不重发模型请求、不重做工具、不消耗 approval、不自动截断。
3. **持久恢复输入与受控续跑**：外化 TaskRun/Attempt、预算/请求 reservation、目标/计划修订、
   授权有效期、snapshot binding 和单写者租约；校验后才能从已验证边界续跑，不能重置预算。
4. **supervisor 及故障演练**：只在合法恢复判定后重启；使用 fencing/租约防双跑，不只判断 PID。
   在写前意图、编辑前、rename 后完成审计前、完成后投影前等位置注入失败；OS 掉电演练只在
   明确隔离的可丢弃 VM 中做，不以实际用户 WSL/主机为测试对象。

沿用现有架构逐步加固；现在直接盲目 replay 风险更高，立刻整体迁移 SQLite/WAL 也不能自动
解决外部副作用和幂等问题。本轮不做这两种行为变更。历史 storage 讨论中的 SQLite 分层仍是
设计候选，不是当前 JSONL/JSON 实现已经迁移。

性能要求：动作意图、关键审计和 checkpoint 的持久屏障应在动作放行前完成；详细调试 trace
可另设批量/丢弃策略，但不能把尚未同步的记录标为 durable。不要对每个流式 token 强制 fsync。
事务批量提交、RPO/延迟及故障注入指标须先测试再决定；2026-11-12 片增加逐条审计文件同步，
2026-11-22 片增加限定路径安全打开及目录同步，未测量其性能成本。
没有实现可放宽 trace 持久性的模式或批量提交调优。

## 验证入口与证据

2026-11-22 EventLog 片（基线 `76f31dc`）：路径 RED 为 8 passed / 12 failed，
identity RED 为 3 passed / 1 failed；第一版接线后定向 209 passed，
全量 1022 passed / 0 failed / 0 ignored，严格 all-targets Clippy、lib/bin build、fmt/diff 通过。
定点跳过追加后的目录同步时，相关测试 1 passed / 2 failed；精确恢复后 3 passed，
源码哈希与此前规格审核快照一致。降权子进程验证只读历史可打开和查询、追加被拒绝。
证据目录 `/temp/codex/steerbox-validation/event-path-20261122/`：
`identity-red.log`、`paths-red.log`、`focused-green.log`、`full-test.log`、
`directory-sync-mutant-red.log`、`directory-sync-restored-green.log`、
`clippy.log`、`build.log` 及 `RECEIPT.md`；审核和提交/合入状态以收据为准。
该目录 `baseline.log` 实际含 helper 增量，991 passed，不冒称纯基线结果。

以下为先前三 store 安全 I/O 片的验收：

2026-11-22 主仓 `bc17025` baseline 补跑：959 passed / 0 failed / 0 ignored，退出码 0；
证据为 `/temp/codex/steerbox-validation/runtime-recovery-20261122/baseline-dev-tests.log`
及同目录 `baseline-dev-tests.exit`。这是新片实现前的 baseline，不是安全 I/O 新片专项、
严格 Clippy/build/fmt 或最终交付验收。

新片专项：模块 13 项，集成 audit 14 + durability 3 + paths 20 + controller gate 17 项通过；
路径 RED 为 7 passed / 9 failed，随后 GREEN 为 20 passed（新增边界测试也包含在内）。
旧 `/dev/null` 集成用例现准确命名为设备拒绝；真实文件同步失败保留注入单测，不能再以
设备用例冒充 sync-failure 阶段证据。另有目录同步失败、短路及建目录失败重试屏障单测。
最终全量 987 passed / 0 failed / 0 ignored，严格 all-targets Clippy、build、fmt/diff 通过；
两名独立生产/规格 reviewer 无剩余阻断问题，目录同步成本和 RunSummary 排除范围已补入。
证据同目录 `audit-unit-green.log`、`audit-integration-green.log`、`path-red.log`、
`full-test.log/.exit`、`clippy.log/.exit`、`build.log/.exit`、`fmt.log` 及 `RECEIPT.md`。

2026-11-12 审计同步专项：`cargo test --locked --offline --lib audit_store::tests -- --test-threads=1`
为 5 passed（含 4 条新增同步单测）；`cargo test --locked --offline --test audit_store_durability
--test audit_store --test workspace_edit_audit_gates -- --test-threads=1` 为 3 + 14 + 17 passed。
覆盖真实 `/dev/null` 写成功/同步失败、三个 store 错误传播、写前阻止编辑及自建子进程
ACK 后 SIGKILL/读取。SIGKILL 不证明 OS 掉电；设备测试不代表生产允许特殊文件。
证据目录：`/temp/codex/steerbox-validation/audit-sync-20261112/`。最终全量、独立审核、
提交态及合入后复验见该目录交付收据，不把未运行门禁计为通过。

以下为 2026-11-09 EventLog 尾部门禁的历史验收：

`cargo test --locked --offline --test event_log_recovery --test state_store --test audit_store -- --test-threads=1`

证据目录：`/temp/codex/steerbox-validation/event-tail-guard-20261109/`。
旧实现 RED 是 3 failed / 5 passed；初次 focused GREEN 是 event 8、state 6、audit 14 项通过。
采纳独立审核建议收紧 typed 错误与行号并补 UTF-8 后，最终 focused 为 event 10、state 6、audit 14 项通过；
master 冻结源码全量 72 个 harness、916 passed / 0 failed / 0 ignored；严格 all-targets clippy、
lib/bin build、fmt/diff 检查通过。提交态和合入后的实际复验、提交 ID 以持久交付台账为准。
未实测 OS 掉电、实际 LLM 或完整跨进程 controller 恢复。

同步语义参考：Linux `fsync(2)` 本机手册，以及 Rust `std::fs::File::sync_all/sync_data` 契约。
