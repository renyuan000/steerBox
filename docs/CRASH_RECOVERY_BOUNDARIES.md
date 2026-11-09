# 崩溃恢复：当前证据与实施边界

核查日期：2026-11-09。EventLog 尾部门禁基线 `5923dad`，私有 transcript 库增量基线 `7998508`。
本文的“当前”不是完整 Phase 1 验收，也不把设计目标标为已实现。

## 现在能恢复什么

| 对象 | 已实现 | 不代表什么 |
|---|---|---|
| EventLog | 事件与换行一起 `write_all`，随后 `sync_data`；重开完整历史后接续序号 | 没有跨进程单写者锁、事务或完整自动恢复 |
| StateStore | 从合法事件重建 loop/step 投影；临时文件同步、rename、Unix 父目录同步 | 不包含完整 TaskRun/Attempt、模型上下文、预算和租约 |
| checkpoint / side-effect / trace | 可查询记录；可标记 `uncertain` 并禁止自动执行 | `append_jsonl` 尚无同步屏障，不能承诺抗 OS 掉电 |
| workspace snapshot | 可选单文件原文快照；文件及目录同步；显式授权 restore | controller 的 snapshot binding 仍在内存，重启后不能据此直接 resume |
| provider 身份门禁 | 拒绝顺序复用已有 run 身份 | 不证明旧请求没发送，也不证明旧动作没执行 |

源码依据：`src/event_log.rs`、`src/state_store.rs`、`src/audit_store.rs`、
`src/workspace_snapshot.rs`、`src/loop_controller.rs`、`src/provider_runner.rs`。
CLI 的 execution 查询和运行中 inbox Resume 不是进程重启后的恢复入口。

## 私有正文日志的新增边界

`transcript_store` 已增加独立存储库：获准正文可逐条写入私有 JSONL，默认首次 dirty 后
1s 或累计 64KiB 同步；关键消息及完整模型请求/响应成功同步后才确认，显式 close 同步并
回收后台线程。它只同步自己的文件；首次创建还同步固定的父目录，不执行全系统 sync。
序号水位区分 written 与 durable；部分写入或同步失败后拒绝继续追加。

这不是实际聊天/provider 的自动记录，也没有恢复读取/续接；整流敏感检查、受保护引用、
轮转和 opt-in 接线仍待实现。默认 1s 是调度目标，不是故障下最大损失承诺；同 UID 恶意
篡改、WSL/VHDX 损坏以及跨 store 原子性也未解决。详见 `PRIVATE_TRANSCRIPT_STORE.md`。

## 本轮修复：未封口尾行不得继续追加

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
- 文件同步与目录条目的持久性是不同屏障。Event/model/tool 的文件同步不等于它们的新建目录项
  已同步；checkpoint/ledger 当前甚至缺少文件同步。多 store 之间也没有统一提交事务。
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
   及真实锁文件，不能遗漏失败追加和历史读取。处理首次创建目录条目同步，并明确同步失败
   的不确定性。State/实际 workspace edit 的替换端也要独立验收。
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
事务批量提交、RPO/延迟及故障注入指标须先测试再决定；本轮不引入额外同步或性能改动。

## 验证入口与证据

`cargo test --locked --offline --test event_log_recovery --test state_store --test audit_store -- --test-threads=1`

证据目录：`/temp/codex/steerbox-validation/event-tail-guard-20261109/`。
旧实现 RED 是 3 failed / 5 passed；初次 focused GREEN 是 event 8、state 6、audit 14 项通过。
采纳独立审核建议收紧 typed 错误与行号并补 UTF-8 后，最终 focused 为 event 10、state 6、audit 14 项通过；
master 冻结源码全量 72 个 harness、916 passed / 0 failed / 0 ignored；严格 all-targets clippy、
lib/bin build、fmt/diff 检查通过。提交态和合入后的实际复验、提交 ID 以持久交付台账为准。
未实测 OS 掉电、实际 LLM 或完整跨进程 controller 恢复。

同步语义参考：Linux `fsync(2)` 本机手册，以及 Rust `std::fs::File::sync_all/sync_data` 契约。
