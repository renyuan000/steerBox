# 恢复诊断 RDI2：工具与副作用日志观察

2026-11-22，基线 `73886fe`。本片增加显式文件的有界观察，不做跨 store 对账或恢复。
实现已落地，最终审核、验证及提交状态以 `PHASE_1_VALIDATION_PLAN.md` 和外置收据为准。
Phase 1 未完成；本片不解除 live provider/live mutation 门禁。

## 入口与兼容

```text
steerbox recovery inspect events.jsonl --tools tools.jsonl --side-effects effects.jsonl
steerbox recovery inspect events.jsonl --state state.json --side-effects effects.jsonl
```

三个选项均只接收显式路径，可任意顺序、每个至多一次。省略的辅助源为 `not_requested`，
不推导 sibling 路径、不加载 config、不初始化 runtime。全部参数在 I/O 前校验。
未使用新选项时仍走原 `inspect_files`，原 JSON 结构及 coverage 不变。
新库入口为 `inspect_files_with_audits(events, state, tools, side_effects, limits, audits)`。
仅在旧投影成功后才比较 State 的行为不变；两个辅助源始终独立检查，不因其他源失败跳过。

新报告保留 schema_version=1、mode=diagnostic_only；coverage 为
`events_state_projection_and_audit_observations_only`，增加 `tools`、`side_effects`。
每源仅有 status/read_bytes/sealed_prefix_bytes/sealed_records/status_counts/class_counts。
报告及 Debug 仅保存固定词汇、计数和位置，不保存 ID、路径、summary、时间或原始错误。
issues 合并所有来源；退出码沿用0无 finding、2完整输出了 finding、1参数/request/输出失败。

## 预算与封口

| 输入 | 默认/硬上限 |
| --- | --- |
| events | 16 MiB，10000条 |
| 可选 State | 2 MiB |
| 可选 tools / side-effects | 每源4 MiB、10000条 |
| JSONL 行 | 共用1 MiB，包含 LF |
| 新入口 aggregate | 所有请求源 cap 之和不超过26 MiB |

库调用只可降低非零上限。先用 checked_add 校验所有请求源 cap 总和，再做任何文件 I/O；
相同路径作为不同源传入仍分别计费。每源至多读一次，不补读、不重试。
这是文件输入字节预算，不是进程内存上限，亦未做性能实测。

仅解析 LF 封口的 UTF-8/类型正确记录；辅助源使用 ToolCallRecord/SideEffectRecord，
不套用 EventEnvelope schema/sequence。空白行不计记录，但推进物理位置及封口前缀。
正好达到记录上限后，仅尾随空白可完整；填满 byte cap 不证明 EOF，仍报 `limit_reached`。
真实 EOF 未封口报 `unterminated_record`；坏行停止该源，不跳过后继续，也不隐藏另一源。
missing、真实 EOF 的空文件、not_requested 三者不同；不完整源的前缀只表示已观察记录。

## 观察语义

status_counts 的八桶互斥：planned、started、completed、failed、blocked、uncertain、
verification_failed、unknown。trim/ASCII大小写规范化，side_effect_uncertain 归 uncertain。
class_counts 六桶：none、workspace_read、workspace_write、external_network、external_system、
unknown；复用既有 SideEffectClass 解析，包括 workspaceread/workspacewrite 等真实写入别名。

计数单位是记录，不是唯一动作。planned/started 仅计数，不产生缺终态推断；
completed-only 是合法观察。completed 后再出现 verification_failed，两条都计入，
不能把 completed 解释为动作验证成功。重复 ID 或跨 loop 同 ID 不归并、不判状态迁移。
failed/blocked/uncertain/verification_failed/unknown status、unknown class 分别产生固定
`*_observed` finding，每源/每code只保留首次位置，计数仍保留全部观察。
源 status=complete 仅表示该文件扫描完整；可同时有异常状态 finding，不代表运行成功。

## 零写入与未覆盖

复用 RDI1 的 no-create 安全读取与同 FD 前后元数据检查；不调用会创建锁或父目录的
store 查询。unstable/read-error 不确认前缀。无文件/目录/锁创建、截断、修复、授权消费、
模型请求、工具执行或恢复；应用零写入不承诺内核 atime 不变。
非 Unix 实际 I/O 继续 fail-closed，未做跨平台实测。

各文件独立读取不是跨 store 一致快照；无 counterpart 不能证明没有执行。
SideEffectRecord 缺少 tool_call/step/task-run 外键，不能靠拆 ID 或时间接近拼接。
本片未检查 model-call、checkpoint 或 snapshot，未持久化 TaskRun/Attempt/预算/
reservation/授权，也没有 fencing、repair、resume 或 OS 掉电恢复保证。
这些是后续门禁，不能从本片 exit0 推导恢复许可。
