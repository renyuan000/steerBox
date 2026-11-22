# 零写入恢复检查器 RDI1

2026-11-22，基线 `75cdb2e`。本片只回答显式事件文件与 loop/step State 投影是否可比较，
不回答任务是否可恢复，不提供 repair/resume。Phase 1 仍未完成。

## 使用与退出码

```text
steerbox recovery inspect /absolute/events.jsonl
steerbox recovery inspect /absolute/events.jsonl --state /absolute/state.json
```

也支持安全的相对路径。缺参、未知/重复/额外参数在 I/O 前拒绝。
没有 config/output/repair/resume 参数。stdout 为单个 JSON+LF，输出失败退出1；
部分输出不能当成功报告。有 finding 的成功报告退出2，不打印 usage。

| 退出码 | 含义 |
| --- | --- |
| 0 | 本片覆盖内无 finding；不代表完整恢复或安全续跑 |
| 2 | 发现异常或检查不完整；不自动修复 |
| 1 | 参数、request 或输出失败 |

## 固定报告合同

schema_version=1、mode=diagnostic_only、coverage=events_state_projection_only。
events 含 status/read_bytes/sealed_prefix_bytes/sealed_records/last_sequence_no/ignored_event_count；
projection 含 status/loop_count/step_count；state 含 status；issues 每项仅
source/code/line/byte_offset。line 从1、offset 从0，指异常记录起点或检查停止位置；
例如满cap且末行已封口时指向下一待确认位置，未必实际存在记录。不适用时为 null。
报告及其 Debug 不含路径、事件/状态正文、ID、payload 或原始错误。

| 字段 | 状态词汇 |
| --- | --- |
| events.status | complete, missing, unsafe_path, unreadable, unstable, limit_reached, unterminated_record, invalid_utf8, invalid_json, unsupported_schema, invalid_sequence |
| projection.status | not_rebuilt, rebuilt, invalid |
| state.status | not_requested, not_compared, missing, matches, mismatch, invalid, unsafe_path, unreadable, unstable, limit_reached |

issue.source 为 events/projection/state；code 与异常状态同名，projection 使用 invalid_projection。
`has_findings()` 等价于 issues 非空。无效 limits 为固定 request error，不接触文件。

## 有界读取与确认规则

库 API 为 `inspect_files(events, state, InspectLimits)`；默认 events16MiB、state2MiB、
line1MiB（含LF）、event_records10000。调用方只能缩小非零上限，不能放大。
两文件最多实际读取18MiB，不是合计16MiB。`take(cap)` 内完成读取；
填满cap但没观察到EOF即limit_reached，文件恰好等于cap也不例外，不以metadata长度代替EOF。
不先无界 read_line/read_until 再判限。

只解析 LF封口行；空白LF行推进物理位置及前缀，不计事件。完整封口行依次校验行上限、
UTF-8、JSON/事件结构、schema1、sequence Some(1),Some(2)…。
下一个非空记录才触发记录上限，恰好达到上限加末尾空白可完整。
未封口尾部未见EOF先limit，已见EOF先unterminated；不跳过内部损坏。
这是诊断的保守 coverage，不修改 EventLog 原有读写兼容合同。

只有完整受支持历史才能 `StateStore::rebuild`，失败只报 invalid_projection。
只投影既有九类 loop/step 事件；其余schema1事件计ignored，不能据此称完整事件对账。
State 仅在完整重建后才打开、反序列化并比较；mismatch 不等于 stale。

## 安全与未覆盖边界

只用 `AuditParent::open(false)/open_read`：逐组件 NOFOLLOW、实际fd regular/nlink=1，
没有mkdir、sync、sidecar、锁或写入。使用前后同fd的dev/ino/mode（含类型）/nlink/len/
uid/gid/mtime+ns/ctime+ns检测变化，排除atime；unstable/read-error 不扫描前缀、不重建。
不是一致性快照、lease/fencing或对任意同UID攻击的完整防护。
字节限额报告中的前缀只表示本次观察到的封口记录，不是 durable 或可续跑边界。
非Unix实际安全I/O继续fail-closed，未做跨平台实测；应用零写入不承诺kernel atime不变。

不加载config、不访问provider/凭据、不消费approval、不修尾、不重放工具或副作用。
请求/工具/副作用/快照对账、跨store一致性、持久TaskRun/Attempt/预算/授权、
single-writer fencing及受控续跑仍是后续任务。不能把本工具的exit0用于放开live写。

验证与提交状态见 `PHASE_1_VALIDATION_PLAN.md` 及
`/temp/codex/steerbox-validation/recovery-inspect-20261122/RECEIPT.md`。
