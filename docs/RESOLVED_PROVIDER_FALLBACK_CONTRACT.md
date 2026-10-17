# Resolved Provider Fallback Contract

状态：Phase 1 实现契约（2026-10-17）

## 目标

为已经解析出的 provider/model 调用增加有界 fallback，同时保持 prompt、认证引用、Attempt lineage、预算和审计事实可追溯。此能力只处理 provider invocation 失败，不把 workspace mutation 或工具执行重放纳入 fallback。

## 允许的范围

一次 fallback 只有在以下条件全部成立时才允许：

- primary 调用已经完成 request binding 校验；
- 失败发生在 provider invocation 阶段；
- 错误类别是 `rate_limit`、`timeout`、`transport` 或 `upstream`；
- primary 没有返回 action；
- 没有执行工具、workspace mutation、verification、checkpoint 或 side-effect ledger；
- fallback candidate 在允许模型、provider capability、route risk 和预算范围内；
- fallback chain 有明确的最大尝试数，并共享原 TaskRun/LoopBudget。

以下错误不得 fallback：认证失败、权限失败、invalid request、binding mismatch、响应解析失败、审计写入失败、action 执行失败、verification 失败和副作用状态不确定。

## 每个 candidate 的重新解析要求

fallback 不是只替换 `model_id`。每个 candidate 必须重新解析并验证：

1. provider protocol/adapter；
2. endpoint；
3. opaque `auth_ref`；
4. model id 与 capability binding；
5. `ResolvedPromptPack` 的 route/task/provider/endpoint/model 绑定；
6. candidate 自己的请求 hash 与审计元数据。

credential 正文不得进入 prompt、事件、trace、checkpoint、ModelCallStore、错误摘要或 fallback event。

## Attempt 与事件语义

primary failure 必须先写入独立 failure JSONL，不能被 child 成功覆盖。随后：

- primary Attempt 结束为 `Failed`；
- 新建 `AttemptReason::Fallback` child；
- child 保存 `parent_attempt_id`；
- child 绑定新的 route decision id 和 step id；
- 写入 `model_route_fallback`，记录 from/to route、尝试序号和脱敏错误类别；
- child 成功后只发布 child 的 completion facts；
- 所有 candidate 失败时以失败收口，不伪造 completion 或 verification。

fallback 不清零 step、model-call、no-progress、verification 或 TaskRun 总预算。

## 禁止的第一版扩展

第一版不实现：

- workspace-edit fallback；
- 已发生写入后的自动重放；
- 并行 provider 调用；
- 健康检查或动态 evaluator 路由；
- 跨 provider prompt cache 迁移；
- 真实 live provider 验收；
- 把 OpenAI Responses、Gemini、Bedrock 等原生协议伪装成当前 Chat Completions 抽象。

## 最小验收矩阵

必须覆盖：

| 场景 | 预期 |
|---|---|
| primary retryable failure，candidate success | child success，parent failed，lineage 与 fallback event 完整 |
| primary authentication/permission/invalid request | 不 fallback |
| primary response parse/binding mismatch | 不 fallback |
| action/verification/checkpoint/ledger failure | 不 fallback |
| fallback chain 或预算耗尽 | 有界失败，不清零预算 |
| candidate 越界或 binding mismatch | 发送前拒绝 |
| primary failure audit 写入失败 | fail-closed，不创建伪 child |
| OpenAI-compatible ↔ Anthropic candidate | 分别构造 provider-specific wire request |

当前实现状态：契约已冻结；代码实现和真实 provider 验收仍待完成。
