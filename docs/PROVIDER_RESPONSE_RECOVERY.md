# Provider 结果校验与有界恢复

日期：2026-11-27。用户已批准分层校验、缺口标注、定向补全的方向。
本合同先限定纯观察宿主入口：无工具、无写入、无外部动作重放；不将可用性优先解释为绕过安全门禁。

## 方案审核

选择：复用现有一次 wire 解析、完成状态与严格 decision schema；只在异常时生成固定缺口码，
用同一 controller/预算/审计链创建 child Retry attempt。正常调用不多发请求，不引入审核 LLM、
全文语义扫描或后台轮询。保留已有单次入口；恢复由可信宿主显式启用，不嵌入 transport 隐藏重试。
替代方案：每轮双模型审核会增加延迟/token；transport 内无限重试会隐藏调用和预算，均不采用。

边界规则：

- Chat 的 `stop`、Messages 的 `end_turn/stop_sequence` 仅确认生成收尾，不证明任务已完成。
  `length/max_tokens` 即使正文是合法 JSON 也拒绝作为完整 decision；空/未知终止原因不猜成功。
- 明确拒绝/过滤、认证/权限、非法请求、绑定错误、动作越界、审计/进度失败及预算异常不重试。
- 仅 timeout/network/upstream 暂态失败可重发纯推理请求；JSON syntax 缺口和输出截断
  可用固定、无正文的提示要求重新生成完整无动作 decision。不是拼接残缺 JSON 或伪造缺失字段。
- wire JSON 的明确 EOF 仅在已收到部分中没有结构化停止信号时记为 termination_unconfirmed；通用 ResponseParse 不在恢复白名单，
  避免把未知协议、工具块或拒绝误当成连接中断。明确拒绝/工具/过滤信号优先于输出截断；
  已解码响应的已知 usage 和宿主输入 hash 保留，未完整解码的用量保持未知。
- decision 的 EnvelopeInvalid/EmptySummary 可能先于动作错误返回，故都不自动修复；
  保留既有首失败分类，无须新增一次全文解析。含糊结构错误宁可准确停止，不盲目重发。
- 最多两次模型调用（一次修复），共享 steps/model_calls/attempts 与可选 reported-token 账本。
  两次失败后必须退出而非伪装完成。未知 usage 不能按零记账；若启用硬 token 门禁，则继续受其阻止。
- 成功返回 continue_observation/request_human 时停止恢复，不自行强改 complete_loop；CompleteLoop
  仍走现有完成验证。纯观察验收不等于代码产物或真实副作用的完成证明。
  两种非完成决策沿用既有纯观察运行器的 Running 状态；本片不新增持久等待人工或后台续跑状态。
- 总期限由可信宿主指定，有界且包含请求时间与恢复等待；每次请求 timeout 不超过剩余期限。
  暂态错误仅做一次短退避，不忙轮询。已发请求可能计费，重发不是免费或 exactly-once 保证。
- 新请求只携带原始授权上下文与固定缺口码，不重发失败正文、reasoning、credential 或 endpoint。
  同一原始观察最多派生一份修复说明，避免逐轮累积上下文。

## 2026-11-27 复审修复：保留已收到的停止证据

- 正常 typed wire 解码不增加第二次解析；仅 EOF 或 HTTP 200 body 读取失败时，
  对已收到部分进行有界结构探测。探针不构造响应树、不搜索模型正文、不补造缺失字段；
  拒绝、过滤、工具和未知终止信号跨重复字段单调保留，后续 EOF 不可将它们降级为可恢复缺口。
- HTTP body 读取失败时，先保留已收到的停止证据，再分类 timeout/network；UTF-8 分裂尾部
  不清除此前完整信号。新增 transport `UnsafeResponse` 映射 `UnsafeTermination`，不重试。
- 非 2xx（redirect 仍直接拒绝）保留 HTTP status，不读取错误正文；截断或超大错误页
  不能把 401/403 改写成 network。超过共享 deadline 仍拒收成功结果，但不覆盖明确错误或非 2xx。
- 只有 `Network` 映射可恢复的 `Transport` class；`ResponseTooLarge`、`InvalidResponse`、
  `Redirect`、`Other` 改为不可恢复的 `ResponseParse`。这也收紧基于这些 class 的 routed fallback，
  不再把未知失败当成换后端的授权。认证/权限/429 等状态继续按既有独立分类处理。

新增真实 RED/GREEN 覆盖拒绝后 EOF、错误页中断、成功 body 中断和明确传输失败；
后者旧实现的 `ResponseTooLarge` 实际发送两次，修复后三协议五类错误均仅发送一次，
只有一条失败记录，且没有 recovery/completed 事件。另补超期证据与分裂 UTF-8 单元测试。
本轮仍只做离线/localhost 验证，不构成真实 provider 或生产吞吐验收。

复审验收基线 `49528a1`：全量 97 个 harness / 1,394 项通过，0 失败、0 忽略；
恢复集成套件 19 项、deadline 单元测试 3 项。严格 all-targets Clippy、实际 lib/bin build、
fmt/diff 均通过。独立只读质量复审发现并复核关闭上述错误白名单问题，主代理核对 diff
并执行全部测试。复现命令沿用下方外置 `RECOVERY_TARGET`；最终新日志位于同一验证目录：
`full-test-stop-evidence-final.log`、`clippy-stop-evidence.log`、`build-stop-evidence.log`。
未新增依赖；未读取真实凭据，未调用真实模型；源码可按本轮 Git diff 精确回滚，无须另复制备份。

本片不靠全文语义扫描猜测自然语言拒绝，结构化协议信号与 schema 是可验证边界；
缺少这些信号的自然语言正文仍可能落入一次 schema 修复。修复不扩大权限、不提供工具，
不对“任何中转异常均不中断”或数学意义的最优 CPU/token/完成率作保证。未进行性能基准，
本轮只验证正常请求不增加模型调用、异常最多一次额外调用、无忙轮询和无新增依赖。

## 实施计划与验证点

1. `tests/provider_response_recovery.rs`：先用已有入口复现合法 JSON+length/max_tokens
   被误接纳为 complete_loop；命令 `cargo test --locked --offline --jobs 1 --test provider_response_recovery`
   使用项目外 target。预期修复前失败，修复后没有成功调用记录/loop_completed。
2. `src/provider_recovery.rs`：定义固定缺口码、协议收尾检查与错误白名单；保留用量证据。
   `src/provider_model_adapter.rs` 与 `src/anthropic_adapter.rs` 在 resolved decision 解码前调用门禁。
   明确拒绝字段在 wire 边界识别；缺口与正常工具调用终止不可混为恢复权限。
3. `src/provider_runner/recovery.rs`：新增显式 recovering preparation/transport/resolver 宿主入口；
   复用 `ForegroundProviderRun` 绑定和同一 controller，不另起 run 清空预算。
   `src/loop_controller.rs` 只增加 crate-private 纯观察 child Retry 入口，调用统一审计/完成路径。
4. 故障注入：timeout→成功、schema 缺口→补全、连续失败有界退出、拒绝/权限不重试、
   request_human/continue 不误报完成、未知 usage/耗尽不绕过、审计失败不继续发送、合法动作不执行。
   核对每次请求各有 failure/success 记录、Retry 父链及固定缺口事件，无原始失败正文泄漏。
5. 全量测试、严格 all-targets Clippy、实际 lib/bin build、fmt/diff，自审后精确提交；
   不修改其它未提交文档，不读取真实 credential，不进行付费 live 测试。

## 最终验收（2026-11-27）

基线 `63913ab`。三位子代理分别承担协议边界修复、只读规格验收和只读质量复核；
主代理核对实际 diff 并统一运行最终验证。规格验收通过，质量复核无未解决的
Critical/Important/Minor；不是仅以代理报告代替编译和测试。

- 全量离线测试：97 个 harness（含空 binary/doctest harness），1,384 项通过，
  0 失败、0 忽略；恢复集成套件 17 项，deadline 单元测试 2 项。
- `cargo clippy --locked --offline --all-targets --jobs 1 --target-dir "$RECOVERY_TARGET" -- -D warnings` 通过。
- `cargo build --locked --offline --jobs 1 --lib --bins --target-dir "$RECOVERY_TARGET"` 通过。
- `cargo fmt --all -- --check` 与 `git diff --check` 通过。

其中 `RECOVERY_TARGET=/temp/codex/steerbox-validation/foreground-upgrade-20261102/master-target`；
全量复现命令为
`cargo test --locked --offline --jobs 1 --target-dir "$RECOVERY_TARGET" -- --test-threads=1`。
最终日志在 `/temp/codex/steerbox-validation/provider-recovery-20261127/`：
`full-test-final.log`、`clippy-verified.log`、`build-verified.log`。
构建/测试产物在项目外；未读取真实凭据，未进行真实或付费模型调用。

用例覆盖三协议暂态错误/EOF/输出上限、拒绝/工具/动作不重试、保留已知用量及输入 hash、
未知用量硬门禁、第二次调用剩余期限递减及超期成功拒收；Responses 重复 type/reason
不能隐藏拒绝、工具或过滤状态。普通路径无额外模型调用，没有新增依赖。
上述是故障注入与回归证据，不是生产性能基准或所有中转故障均可恢复的证明。

## 未来边界

生产 incremental streaming、长期调度延迟重试、跨供应商自动切换、工具执行后对账和任务级
产物覆盖检查不由本切片实现。不承诺任意中转永久故障时仍能完成任务，也不把终止误称为成功。
协议来源：OpenAI 官方 SDK `openai-python/src/openai/types/chat/chat_completion.py` 的 Choice，
Anthropic 官方 SDK `anthropic-sdk-python/src/anthropic/types/stop_reason.py`；本轮读取核对终止枚举。
