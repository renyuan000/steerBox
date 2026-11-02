# Loop-scoped Step 投影契约

## 目标

StateStore 是可从 EventLog 重建的投影，不是第二套事实源。不同 loop 可以各自使用 `step-1`；投影必须同时保留两者，不能因为裸步骤名相同而使整份执行历史查询失败。

本片只改变步骤投影身份、直接查询 API 和投影 JSON。provider、controller 的 step/event/model-call ID、历史身份门禁、动作执行、预算和凭据边界不变，不以改名隐藏历史重放问题。

## 步骤身份与状态迁移

- 步骤身份为 `(loop_run_id, step_id)`；内部索引为 loop 到 step 的嵌套 map。
- `StateStore::step(loop_run_id, step_id)` 精确查询；不保留只给裸 step ID 的歧义 API。`steps()` 返回所有 loop 下的步骤，不丢失同名步骤，迭代顺序为先 loop ID、再 step ID。
- 不同 loop 同名步骤可以开始和独立完成/失败；同 loop 重复 `step_started` 仍拒绝，已存在的未知 loop、非 Running loop 启动限制不变。
- terminal 有明确 loop 身份时，只查该 loop 的步骤；未知 loop、未知步骤、重复 terminal 均拒绝，不回退到其他 loop 的同名步骤。
- terminal 没有 loop 身份时，保留既有无 loop 字段事件的唯一匹配语义：只存在一个同名步骤才可应用；零匹配拒绝，多个匹配返回固定原因的 `InvalidTransition`，不能按 map 顺序或“哪个还在运行”猜归属。
- 顶层字段与 payload 的冲突仍用既有字段解析器拒绝；状态、summary、时间和 sequence 更新属于精确匹配的步骤及其所属 loop。

## 投影持久化边界

新 steps 布局为 `steps[loop_run_id][step_id] = StepState`，可持久化并重新加载两个同名步骤，不能用 tuple key 草率编码成 JSON object。

旧的非空扁平 `steps[step_id] = StepState` 不做静默兼容迁移；`load()` 拒绝该布局。需要时由宿主读取原 EventLog，执行 `StateStore::rebuild()` 并显式持久化新投影；本片不新增自动文件发现、自动覆盖或 CLI 恢复命令。空 steps map 可按新布局正常解码。事件和模型审计日志不迁移、不删除，也不重新执行模型或工具。

本片不增加 StateStore schema-version，不承诺新旧二进制投影文件互读；EventLog 事实源保持原格式。已有原子写入实现不在本片重构。

## 验证与非目标

先以两个 loop 各自产生同名步骤的合成事件复现，然后验证步骤独立终态、精确查询、ExecutionQuery 的 list/show/page、同 loop 重复拒绝、错误 loop 和缺少 loop 的歧义拒绝，以及新布局 round-trip 和旧非空扁平布局拒绝。最终实际命令和结果记录在 `PHASE_1_VALIDATION_PLAN.md`，不能把本契约视为已运行测试。

不解决 controller 事件 ID 全局唯一、跨进程执行认领、持久 TaskRun/Attempt/预算恢复、日志不完整、并发写入或 live provider 验收；也不改变旧步骤 terminal 在 loop 已结束时的其它既有行为。
