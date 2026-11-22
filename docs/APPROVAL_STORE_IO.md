# ApprovalStore 安全 I/O（A1）

2026-11-22，基线 `ee87518`。本片仅加固 metadata-only 审批历史及其真实
`.lock` 的路径与追加确认边界，不扩展审批权限，不开放 live 写 CLI。
实现、测试、独立审核与最终交付状态分别记录；当前验收结果以
`PHASE_1_VALIDATION_PLAN.md` 和外置 `approval-path-20261122/RECEIPT.md` 为准。

本轮验证限定于普通合成文件、已落盘的两项 symlink 回归及实际进程锁/重载。
私有 FD 测试覆盖 partial write、完整写入后报错、文件同步失败、父目录同步失败
与 parent rename+witness；运行结果与最终门禁分别记录，不以测试定义代替通过证据。
扩展特殊路径矩阵、FIFO 和真实降权权限验证被审批层拒绝，未实施、未验收；
共享 helper 的既有测试不能替代审批公开入口专项。本片不据此放开 live 写入或宣称完整 A1。

## 单次操作的边界

`submit`、`resolve`、`consume`、`latest` 和 `read_all` 先取得实例 Mutex，
再安全打开并固定审批数据的父目录，通过该 directory fd 派生原 basename 加
`.lock` 的真实锁文件。实际 OS `try_lock` 保持 fail-fast，不改成阻塞等待。
取得锁后完整重载历史，判断业务状态；需要写入时，同一个 pinned parent
负责打开追加目标。只有完整 JSON+LF、同一个打开文件的 `sync_data` 和该 parent
的目录同步均成功，才更新内存并确认新记录。锁持有到操作结束，不删除 sidecar。

源码复用共享路径守卫：逐组件拒绝 symlink 和父目录回退；数据与锁使用
NOFOLLOW/NONBLOCK，并验证实际 fd 为单链接 regular file。这些实现守卫不等于
审批入口已完成 FIFO、hardlink 或特殊路径全矩阵验收。锁名不做有损 UTF-8 转换。

`open` 仍立即加载；缺失时创建父目录及空数据文件，并同步实际文件和目录。
构造器不取得 OS 锁，也不构成原子历史快照；后续操作必须在实际锁内重新加载。
构造器记录初始化错误；后续公开操作仍尝试锁内重载，成功后清除旧错误。
安全打开、取锁或重载失败时，`latest` 仍折叠为 `None`，`read_all` 返回错误；
不能把 `latest == None` 当成“磁盘上肯定没有审批”。

## 保持的审批语义

- 同 ID、同 intent 的 `submit` 忽略新 envelope 时间戳，返回既有状态；不续期、
  不重新 Pending、不追加，也不补做旧记录的持久确认。
- replay 使用完整 request 相等，包括原时间戳；仍校验合法状态迁移与快照形状。
- 同 scope 重复批准返回原 Approved，不重设 `resolved_at`，也不重新检查过期；
  后续 consume 仍检查存量 grant 的过期时间。
- Once 只有成功追加 Consumed 后才返回 `true`；Task/Session 匹配成功可复用，
  不追加 Consumed。过期追加 Expired。匹配字段、首个匹配记录的选择顺序不变。
- controller 的写前审计、snapshot preflight 仍先于审批消费；审批不能绕过
  PolicyGate 的 hard block。

## 失败不是回滚许可

write、文件同步或父目录同步失败时返回错误，后续步骤短路；失败前可能已经留下
部分或完整新字节，不截断、不回滚、不自动重试。内存不得在追加成功前推进。
若完整 Consumed 已可读，即使上次未收到成功 ACK，下一次操作也必须锁内重载，
不能使用旧 Approved 再次授权；若尾部损坏，则拒绝该历史，不退回旧内存快照。
这只说明当前可见历史的安全判定，不证明未同步字节在 OS 掉电后仍存在。

重复 submit/resolve 返回历史状态仍不是 durable re-ACK。保留 blank、物理行号、
UTF-8 I/O 错误及完整无 LF 尾行可读的历史兼容性；无 LF 后追加可能拼接成坏行，
本片不修尾，也不把此类历史改写成新的格式。

## 权限变化与未覆盖范围

新目录/文件请求 `0700`/`0600`（umask 可收窄）；既有 owner、mode 不改，
不新增 UID 或 owner-only 拒绝政策。查询需要可读 data 与可读写 lock；
Once/Expired 等新增状态另需 data 可写。安全父目录遍历要求目录可读，
每次访问的目录链屏障增加 I/O；成本尚未测量，不承诺吞吐不变。
非 Unix 实际 I/O fail-closed，没有进行 Windows/macOS 等价能力验收。

advisory lock 不能防任意同 UID 写者删除/替换 sidecar；pinned parent 不等于
跨访问身份绑定或 fencing，也不固定 reload 与 append 之间 data basename 的 inode。
静态推断（高置信度，依据为两次独立 data open）：有目录写权限的非合作进程
仍可替换数据条目或制造双锁 inode。本片要求可信目录与遵守同锁协议的合作进程。
构造、锁、重载和追加也不是跨 store 事务。
本片不完成 State/workspace 替换端加固、TaskRun/Attempt/预算/授权持久恢复、
请求/工具/副作用/快照对账或自动任务续跑。Phase 1 仍未完成。
