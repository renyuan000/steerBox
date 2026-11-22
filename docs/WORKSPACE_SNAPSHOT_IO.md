# Snapshot SS1：限定安全磁盘 I/O

2026-11-22，基线 `4586e5f`。本片只加固现有 `WorkspaceSnapshotStore`，
不新增恢复入口、授权、JSON schema、ID 或 hash 合同。验收状态见
`PHASE_1_VALIDATION_PLAN.md` 和外置 `snapshot-io-20261122/RECEIPT.md`。

## 为什么修改

旧实现先检查/canonicalize 路径，再按路径打开记录、删除失败记录和同步目录。
仅在末级加 `NOFOLLOW` 不能固定祖先目录。普通目录 rename/替身 fixture 已复现：
source 读取到替身正文，capture 写入替身 root，失败清理未删除原 root 的本次记录。
最初五项行为回归在旧实现上为0通过/5失败；不是实际攻击或 OS 掉电实验。

SS1 将操作绑定到安全逐组件打开的真实目录 FD。没有新增依赖；不以复制整个
workspace 或增加后台线程代替目录绑定，也不修改 controller 审批和事件语义。

## 操作边界

| 操作 | 合同 |
| --- | --- |
| `open(root)` | 只保存路径，零磁盘 I/O。 |
| store root | 绝对路径、无静态 symlink、workspace 外；安全打开现存 containing parent，再相对进入 store leaf。 |
| capture 建目录 | 仅缺失末级 root 可 `mkdirat(0700)`；缺失祖先返回错误，不递归创建。本次 mkdir 成功后同步 containing-parent FD。失败可留下 root，不回滚。 |
| root 私有性 | 从实际 store FD 取 metadata；effective UID 相同且 `mode & 0o7077 == 0`。不要求 GID 相同，不自动 chmod/chown。 |
| record 创建 | 同一 root FD 下 `O_CREAT|O_EXCL|O_NOFOLLOW`，请求0600；创建成功即把 FD 交回 snapshot，后续 regular/single-link/private 校验失败也进入清理路径。 |
| capture 成功 | 完整 JSON write → 同一 record FD `sync_all` → pinned root FD `sync_all` → 返回 metadata。 |
| capture 失败 | 仅在本次独占创建成功后，尽力 unlink 本 basename、同步同一 root FD；保留最初错误，root 不回滚。冲突不会删除旧记录。 |
| record 读取 | 同一 pinned root FD 相对打开，无创建；实际 FD regular/single-link/private 检查后有界读取。 |
| source/current 读取 | 静态检查保留错误分类；实际打开逐组件 no-follow，固定父 FD，再打开末级 FD；保留 single-link，不套用 editor W1 的 hardlink 放行规则。 |

共享 helper 的 `candidate` 只负责安全打开，调用方必须在读取/写入正文前校验
实际 FD。snapshot 自行校验以保留 `NotRegularFile`/`NotPrivate`，不将其吞成通用 Io。
其他 store 继续使用既有校验接口；旧 helper 方法行为未改。

## 兼容与错误顺序

- capture：身份/workspace/path preflight → root 校验/可选末级创建 → source 完整读取、
  UTF-8/限额 → 编码限额 → record 独占创建。坏 source 优先于重复 ID 冲突；
  非法 UTF-8 或超限可能留下空 root，但不能留下 snapshot record。
- restore：expected hash 格式最先验证，之后身份/root/record integrity/binding，再查
  current 的内容 hash。store/record 缺失不创建；静态 symlink 仍为 `UnsafePath`。
  动态替换后的安全打开失败为脱敏 `Io { kind }`，不回显路径、正文或原始 OS 文本。
- source1MiB，record `6*1MiB+64KiB`；metadata 限额和 `take(limit+1)` 双检查。
  原 CRLF/Unicode/JSON escaping、private 模式、goal/workspace/path/hash 绑定不变。
- replacement 仍调用既有 editor，失败映射 `ReplacementFailed`；controller 的
  Once approval、preflight、ledger/checkpoint 门禁不变。
- 非 Unix 继续 `UnsupportedPlatform`；新增 helper 非 Unix 返回 Unsupported，
  不增加 by-path fallback。本轮没有非 Unix 实测。

## 明确不保证

固定目录 FD 不等于同 UID/root 隔离：目录内容仍须可信、无未协调的同名 entry 替换。
另一个 writer 在同一个 pinned root 内替换 basename 时，best-effort unlink 不提供
inode 条件删除/CAS/fencing；清理失败可能残留，原始持久化错误不被清理错误覆盖。

capture 仍直接独占创建最终 `<id>.json`，不是原子发布；进程中途退出可留下不完整
记录。部分/完整 write 或 sync 报错不是可盲目重试许可。读取没有前后 metadata
稳定性比较，不证明读取期间没有原地修改，也不证明 capture 与实际编辑前版本一致。

目录需可读才能安全打开；没有降低权限以迁就配置。本片未做性能测量，不声称
增加目录打开/同步没有成本。测试证明普通本地文件路径与故障传播，不证明文件系统、
磁盘、OS/WSL2 掉电恢复或所有故障矩阵。

snapshot binding 仍在 controller 内存中，不能跨重启重建。没有跨 store 对账事务、
TaskRun/Attempt/预算/授权恢复、自动 rollback/resume 或孤儿清理。Phase 1 未完成，
live provider/live mutation 验收仍须独立获得授权并执行。

## 可复现检查

使用外置 `CARGO_TARGET_DIR`、`CARGO_BUILD_JOBS=1`，串行执行：

```bash
cargo test --locked --offline --lib
cargo test --locked --offline --test workspace_snapshot --test controller_snapshot_integration
cargo test --locked --offline --all-targets
cargo clippy --locked --offline --all-targets -- -D warnings
cargo build --locked --offline --lib --bins
cargo fmt --all -- --check
git diff --check
```

新增 helper 测试首次为缺符号 compile RED，不能称为行为失败；snapshot 的五项
普通文件回归取得真实行为 RED。完整写入报错、cleanup 失败保留原错误及固定 root
record 读取另有测试。成功见证观察实际 write/file-sync 的 raw FD/dev/inode、held-root
目录身份，并真实执行两个同步；四类错误的正常持久化阶段严格短路。
这不是 open-file-description 全生命周期或内核真实 sync 故障证明；cleanup 的同步
不在正常阶段轨迹中。未新增 metadata/read 阶段故障矩阵。
不重复未获准 A1 特殊路径/FIFO/真实降权专项。
