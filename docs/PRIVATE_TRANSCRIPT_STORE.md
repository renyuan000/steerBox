# 私有对话/模型正文日志：库基础

日期：2026-11-09。实现入口 `src/transcript_store.rs`。本片不改变实际 provider、controller
或 CLI 行为，不会自动抓取用户现有聊天、凭据或请求。

## 已实现契约

| 内容 | 行为 |
|---|---|
| 新会话 | `TranscriptStore::create(root, workspace, session, policy)`；不 reopen、不覆盖历史 |
| 逐条写入 | 完整 JSONL 直接写文件；保留获准 UTF-8 正文，JSON 空白不重新格式化 |
| chunks | 首次 dirty 起默认 1s、累计默认 64KiB，任一达到即同步；后续追加不推迟期限 |
| 关键边界 | UserMessage、AssistantComplete、ModelRequest、ModelResponseComplete 同步成功才 ACK |
| 持久化凭据 | `written_sequence` 与 `durable_sequence` 分离；仅同步成功推进 durable |
| 故障/限额 | 部分写入/同步失败 sticky；参数/容量拒绝不毒化、不消耗序号 |
| 正常结束 | 显式 `close` 同步并 join；Drop 仅尽力，不能代替成功收据 |

限额：正文 64KiB/条，文件默认 64MiB（最大 1GiB）；同步间隔 1ms–60s，阈值
1B–4MiB。大正文分片需要调用方先做整流安全检查，不应直接切块绕过门禁。
没有无界正文队列；每个活动 store 有一个条件变量等待的线程，空闲时不定期 fsync。
大量并发会话的线程/吞吐开销尚未基准测试，不能据此宣称性能提升比例。

## 私有文件与敏感边界

Unix：root/workspace 是已存在的绝对目录；拒绝歧义路径、任一级符号链接与重叠目录树。
逐级 `openat` 固定目录 fd，root 必须由有效 UID 所有且无 group/other 或特殊权限；最终
`O_EXCL|O_NOFOLLOW` 创建 `0600` 普通单链接文件，首次同步文件及固定父目录。
非 Unix 直接拒绝，不创建文件。权限由运行环境保证；不修改目录权限。

后续写入持有文件 fd，原路径替换不会重定向写入；**不防任意同 UID 进程修改 inode、
删除、改权限或重挂载**。创建后校验/同步/线程创建失败可能留下新文件；不得按路径
盲删或覆盖重试。日志是本地明文私有文件，不是加密备份。

调用方只能交入已获准正文。已有本地 Redactor 检查已知敏感模式；完整 JSON 还检查
解码值、重复键及递归 transport 字段。去除开头空白后以 `{`、`[` 或 `"` 开始却不能
安全解析的正文（含深层嵌套、数值溢出、格式错误和未闭合 chunk）直接拒绝，不回退文本检查；
这可能拒绝普通代码/引号片段，调用方不能将该库当任意原始流存档器。
已识别敏感数据整条拒绝，不静默改写。公共错误
不带正文/路径，Entry 的 Debug 不带正文或标识值；原 metadata-only ModelCallStore 不变。

这不是任意秘密识别器，尤其不能保证识别跨 chunk 的秘密；因此**尚未启用原始流自动采集**。
结构化正文中的 headers/auth/auth_ref/endpoint/base_url/credential 等传输字段被拒绝，
即使同名字段用于普通业务也可能被拒绝；本片宁可显式拒绝，不把配置误当获准正文保存。
Endpoint、API key、认证头、resolver 返回值或未经处理的上游错误均不属于正文输入。

## 证据与未完成项

证据目录：`/temp/codex/steerbox-validation/transcript-store-20261109/`。
writer 行为 RED：3 passed / 13 failed；公共 API 最初因未实现编译失败，两者分开记录。
初次 GREEN writer 16 项；初次公共专项 16 项通过，包括并发序号、Unicode/JSON 空白、
重复键、Unicode 转义回归、容量、路径与受控 ACK 后 SIGKILL。
Unicode 转义回归先复现失败，再修复后通过。
独立质量审核另发现结构化解析失败回退的绕过，`structured-red.log` 已实际复现；
补深层/溢出/格式错误回归并修复，最终结果以重新验证日志与交付收据为准。
修复态 `reviewed-public.log` 为 17 passed，`reviewed-tests.log` 为 73 harnesses / 949 passed /
0 failed / 0 ignored，`reviewed-tests.exit=0`；严格 all-targets clippy、lib/bin build、fmt/diff
重新通过（`reviewed-final-checks.exit=0`）。独立规格与质量复核均无剩余阻断。

```sh
cargo test --locked --offline --lib transcript_store::writer::tests -- --test-threads=1
cargo test --locked --offline --test transcript_store -- --test-threads=1
```

提交态和合入后复验：最终交付收据记录，本段不将未运行检查标为已通过。
SIGKILL 测试仅杀自建子进程，不证明 OS/VM/磁盘掉电。
阻塞 I/O/线程调度可能超过 1s；provider 已执行却本地未记录的窗口仍不能盲重试。

后续：整流敏感处理、受保护 reader/引用、轮转/去重、明确 opt-in 的真实入口接线。
2026-11-12 共享 audit 追加已补齐 checkpoint/ledger/trace 的同文件同步屏障，见
`CRASH_RECOVERY_BOUNDARIES.md`；目录项/路径安全及跨 store 原子性仍待完成。
现有 runtime 安全 I/O、持久 TaskRun/预算/授权与对账恢复仍独立待办；
本片不授权 live 写 CLI、不代表 Phase 1 完成。
