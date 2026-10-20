# 受约束的主动自适应模型路由与质量策略

**日期**：2026-10-20
**状态**：Phase 1 设计增量；尚未实现

## 目标

steerBox 需要同时具备：

- **安全性**：模型不能通过自适应请求降低安全底线、扩大数据出口、改变 endpoint 或取得凭据；
- **灵活性**：多个 Provider、协议、模型能力、成本和延迟档位可以被统一调度；
- **主动性**：后台运行时不因低风险调节反复询问用户，能够根据当前调用的确定性质量信号自动升级、切换或停止；
- **可解释性**：每一次升级、降级、fallback 和停止都有原因、预算、证据和可查询记录。

本文不把模型自评当作安全授权，也不把开放式在线学习纳入 Phase 1。

## 术语与边界

```text
CandidateFilter       运行时硬过滤
StrategySelector      选择初始任务策略
QualityAssessment     评估当前调用质量
AdaptationController  在预算内决定继续、升级、切换或停止
RouteExecutor         创建 Attempt/RouteDecision 并执行已授权切换
ProviderAdapter       使用受保护 endpoint/credential 发请求
```

### 不可由模型改变的硬边界

- provider allowlist、endpoint 绑定和 `auth_ref` 绑定；
- 数据隐私等级、出站规则、工具权限和任务范围；
- 最大成本、最大延迟、最大并发、最大重试和最大 Attempt 数；
- 安全策略、审批要求、审计和停止条件。

模型只能提出候选 profile、效果级别或重试建议；最终决策由运行时重新校验。

### 可以自动调节的运行参数

- 效果级别：`fast`、`balanced`、`strong`、`xhigh`；
- 已授权候选模型之间的切换；
- 单次调用 timeout、验证频率和有限重试次数；
- 在预算和并发上限内的局部执行策略。

自动调节不能把已拒绝的模型重新加入候选，也不能通过提高效果级别绕过安全规则。

## 质量评估

`QualityAssessment` 必须优先使用宿主可验证信号，按以下顺序组合：

1. **确定性验证**：结构化输出解析、工具执行结果、测试/链接检查、目标条件和 verification contract；
2. **运行时信号**：超时、截断、网络错误、Provider 状态、延迟和 token 使用；
3. **任务结果信号**：artifact 是否生成、是否产生有效进展、是否重复同一动作；
4. **独立 evaluator**：可选的第二模型评估，只作为证据，不直接获得权限；
5. **模型自评**：仅作低权重参考，不能单独触发安全放宽或无限重试。

建议的质量状态：

```text
Verified       确定性验证通过
Acceptable     达到任务最低质量目标
NeedsUpgrade   未达目标但仍可在预算内升级
Retryable      Provider/瞬态故障，可创建新 Attempt
Blocked        触碰安全、预算、权限或停止边界
Failed         无法在允许范围内继续
```

## 自适应状态机

```text
initial level
  -> call
  -> assess
     -> Verified/Acceptable: continue or complete
     -> NeedsUpgrade: raise one level, if budget/cooldown allows
     -> Retryable: create child Attempt and select next allowed candidate
     -> Blocked: stop without retry
     -> Failed: record evidence and stop/escalate
```

必须满足：

- 每次升级最多跨一个效果级别；
- 同一调用不得无限重试；
- 同一质量信号触发升级后进入冷却，防止 fast/balanced/strong 震荡；
- 升级、切换和重试共享 TaskRun/Attempt 总预算，不清零已消耗额度；
- 模型切换创建新的 `Attempt`、`RouteDecision` 和 `ModelCallRecord`，保留原失败证据；
- 达到上限后进入 `waiting_human`、`blocked` 或 `failed`，不能继续自我加码；
- 成功后的降级只能在明确的稳定窗口后发生，Phase 1 默认不在同一任务中自动降级。

## 路由评分与历史信号

Phase 1 允许使用当前任务内的运行信号修正候选排序，例如：

- 当前 Provider 是否超时或返回可重试错误；
- 当前模型是否通过结构化输出和 verification；
- 实际延迟和 usage 是否超过预算预测；
- 当前 Attempt 是否出现无进展或重复动作。

这些信号只能在本次任务预算和允许候选集合内影响排序。Phase 1 不做跨项目、跨任务的在线权重训练，不把单次异常永久写成模型能力结论。

## 并行模型的范围

Phase 1 先实现**顺序自适应**：一次只执行一个模型，质量不足时按规则升级或切换。并行比较、quorum、取消竞赛请求和多模型聚合属于后续阶段，必须另行定义共享预算和取消语义。

## 审计要求

每个 `AdaptationDecision` 至少记录：

- task/loop/attempt 身份；
- 当前效果级别和目标效果级别；
- 质量状态与触发信号类别；
- 候选集合、被拒绝原因和最终选择；
- 预算剩余、重试次数、冷却状态；
- 关联的 `RouteDecision`、`ModelCallRecord`、verification 和 evidence 引用。

记录中不得包含 API key、密码、token、cookie、私钥、认证响应、完整 endpoint 或未授权模型输入正文。

## 验收范围

### Phase 1 必须完成

- 当前任务内的有限效果级别升级；
- 确定性质量信号驱动的受控 fallback；
- 新 Attempt、累计预算和失败证据保留；
- 安全底线不因自适应而改变；
- 自适应决策可审计、可解释、可回放；
- fake/localhost provider 覆盖成功、低质量、超时、不可重试、安全阻断和预算耗尽。

### Phase 2 再做

- 跨任务历史表现学习；
- 成本/质量/延迟在线模型；
- 并行 quorum 和多模型聚合；
- provider prompt cache 和跨任务复用；
- 根据长期反馈自动修改路由配置。

## 当前状态

截至 2026-10-20，steerBox 已实现并通过 focused 契约测试验证 `EffectLevel`、`QualityAssessment`、`AdaptationDecision`，以及把受约束决策绑定到真实 `RouteDecision` fallback 链的纯规划接口。当前仍未实现本文件定义的完整 `AdaptationController`、provider response/verification/tool result 的质量信号采集、自动 Attempt/RouteDecision 事件接线和 resolved-provider 自动 fallback；因此这仍是可复用的安全规划切片，不是完整模型连接型 Agent。
