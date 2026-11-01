# TaskRun 已报告 token 累计与后续发送门禁

日期：2026-11-01。状态：库切片已实现并通过独立验收；配置及 runner 接线增量见 `REPORTED_TOKEN_RUNNER_CONFIG_CONTRACT.md`，其验收记录单列于 `PHASE_1_VALIDATION_PLAN.md`。关联：`PHASE_1_TODO.md` P12.3、自适应模型策略及 resolved 失败证据契约。

## 目标与方案选择

先累计 provider 实际报告的 input/output token，再决定同一 TaskRun 能否继续发送。默认不启用，不改变既有请求、provider retry 白名单及八维次数预算。

三种方案的取舍：

| 方案 | 本片选择 |
|---|---|
| 根据缺失字段、total 或静态估价推算用量 | 不采用；不能把估算当事实 |
| 已报告用量累计，未知时阻断后续请求 | 本片采用；能复用已有安全证据，边界可测试 |
| 发送前预留、可信 tokenizer、输出 cap 的硬上限 | 后续独立切片；本片不承诺正在执行的单请求不超额 |

金额预算还需币种、整数计费单位、价格版本与缓存等语义；本片不引入金额预算或账单推算。

## 不变量

1. 状态归属于 TaskRun，独立于可缩放的 LoopBudget；同 controller 的普通 resolved、fallback、upgrade、continuation 共用，不能因新 Attempt 重建或清空。可选状态缺省为关闭；启用只允许在首个模型调用前，不提供中途增额、清零或解除阻断接口。
2. 逐调用记录 `model_call_id` 和 `input_tokens/output_tokens: Option<u64>`。Some(0)是实际报告零；None仍未知，不从total倒算。不存响应正文、连接信息或凭据。
3. 已知分量及其合计均 checked_add；未知、冲突或溢出后永久阻断本账本的后续发送。错误不回显外部正文。不能wrap或静默饱和成正常计数。
4. 同ID/同用量重复结算幂等；任何逐字段差异拒绝并阻断。已结算ID不能再次发请求；成功/失败共用ID空间，不能通过重复step ID少计。这里的结算幂等不授权重放请求。
5. 共同发送接缝在消耗本次step/model-call预算之前检查：有效、无阻断、未使用call ID且累计已知token小于限额。限额为零时首调也不发送。达到或超过限额不再发送，但已发生用量不能撤销。
6. adapter返回后，无论成功、解析失败或transport失败均进入统一结算。成功使用response usage，失败使用ResolvedCallEvidence；没有证据时作为未知。调用前本地拒绝不虚构调用或结算。
7. 已知事实在审计追加失败时也保留；审计/进度故障必须先阻断后续发送，不能只依靠upgrade外层封口。不开自动审计重试，不补造未落盘evidence。
8. 显式启用后，不能从未提供可信usage的non-resolved API绕过门禁。该路径应发送前拒绝，不将其伪装成已计量调用。
9. 状态序列化round-trip须保留累计和阻断；旧TaskRun缺字段默认关闭。对可信持久化状态做反序列化内部一致性校验，拒绝小计与逐调用记录不符；不承诺防止整段删除、旧快照回放或一致性伪造。本片不提供跨进程恢复协调或并发worker共享预算保证。

## 实施与检查点

- [x] `src/reported_token_budget.rs`、`tests/reported_token_budget.rs`：纯账本、checked累计、幂等/冲突、未知与持久化校验；新模块在`src/lib.rs`导出。
- [x] `src/domain.rs`：TaskRun独立可选状态，旧快照缺省关闭；不修改八维预算的缩放语义。
- [x] `src/loop_controller.rs`：显式启用、共同发送前门禁、成功/失败结算、审计/进度错误阻断、non-resolved路径前置拒绝。
- [x] `tests/model_call_controller_audit.rs`、`tests/reported_token_controller_boundary.rs`及纯账本测试：默认行为、限额0/边界/超过、部分/未知/双零、溢出、重复ID、成功/失败共同计入、升档/fallback/continuation不补额度；独立边界回归验证已知非零用量、高余量、Running状态下，普通resolved的store/progress故障解除后跨action入口零新增调用，以及TaskRun恢复继承和两次实际升档请求累计。EventLog故障封口由共同接缝源码审核覆盖，本片未新增独立故障注入。
- [x] 依`DEVELOPMENT_REVIEW_POLICY.md`完成规格及质量源码审核、独立边界测试增量审核和主Agent验收。最终全量64套/774项通过、0失败/忽略；严格clippy、lib/bin build、fmt/diff均通过。

功能基线：`0aae738`；审核约定基线：`53deb0a`。工作树：`/temp/codex/steerbox-worktrees/reported-token-budget-20261101`。编译/日志：`/temp/codex/steerbox-validation/reported-token-budget-20261101`，jobs=1、offline；master最终日志为`master-final-tests.log`、`master-final-clippy.log`和`master-final-build.log`。不读取真实配置或调用live LLM。

宿主在首次调用前显式调用`controller.enable_reported_token_budget(limit)`；通过`controller.task_run().reported_token_budget()`只读查看。后续顶层配置字段`reported_token_limit`及runner接线见独立契约；不新增CLI参数，不保证在途请求不超额，也不是金额预算或跨进程恢复协调。上方64套/774项是本库切片的历史验收，不代替后续配置接线验收。
