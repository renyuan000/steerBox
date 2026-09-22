# Shared Harness 安全模型讨论稿

## 文档定位

本文讨论共享 Harness 自身的安全模型，不替代安全评估智能体或安全运营智能体的领域设计。它回答的是：无论当前执行的是编码、运维、评估还是运营任务，Agent、模型、工具、记忆和外部 Provider 之间如何建立可信边界。

本文是设计讨论稿。未标为已实现的内容均不能当作当前代码能力。

## 信任边界

```text
用户 / 控制面
  -> GoalContract / Approval / PolicyGate
  -> Harness Runtime
       -> Model Provider（外部可见性边界）
       -> Tool / MCP / Plugin（能力和供应链边界）
       -> Workspace / Asset / Data Source（数据与副作用边界）
       -> EventLog / State / Evidence / Memory（事实与长期知识边界）
```

默认假设：模型可能犯错，外部数据可能包含 Prompt Injection，Provider 可能有留存或法律可见性，工具和插件可能被污染，长期记忆可能被投毒。安全设计不能只依赖系统提示或模型自律。

## 核心控制

### 数据分级与 Provider 出口

所有发送给模型或外部工具的数据先经过分类和最小化。至少区分公开、内部、敏感、凭据/密钥、受监管或不可外发数据。凭据不进入 prompt、trace、event、checkpoint 或 memory；通过受控 `auth_ref` 在 adapter 边界解析。

Provider、Endpoint、Model、Prompt 和 Agent Role 分层配置。路由策略要能根据数据等级、任务风险、地域、留存、延迟、成本和能力约束拒绝或改走本地/私有模型。

### 不可信数据与可信控制流

网页、邮件、日志、Issue、漏洞情报、工具输出、样本说明和用户转发内容均可能是不可信数据。它们可以作为待分析对象，不能直接改变 GoalContract、权限、授权、审批状态或停止条件。

### Capability 与最小权限

工具能力应带有作用域、资源范围、网络范围、side effect、预算和有效期。安全评估的主动能力必须绑定 `AuthorizationContract`；安全运营的响应能力必须绑定 `ResponsePolicy`。高风险能力不能通过模型自述或普通聊天隐式获得。

### 记忆与知识治理

事件事实、当前状态、检查点、短期上下文、长期记忆、代码索引和冷归档分层存储。长期记忆写入先经过来源、置信度、作用域、冲突和投毒检查；跨项目共享必须显式授权。记忆不是事实源，不能替代 EventLog、Evidence 或确定性权限查询。

### 审计与恢复

策略判断、模型路由、数据出口、工具调用、人工审批、响应动作和版本变化都必须进入可追溯记录。运行时要支持暂停、取消、紧急停止、检查点恢复、幂等和不可恢复副作用标记。

## 风险到控制映射

| 风险 | 主要控制 |
|---|---|
| Provider 隐式上传或留存 | egress gateway、数据分类、Provider allowlist、出口审计、本地/私有模型 |
| Prompt Injection | 不可信数据标签、控制流/数据流分离、工具结果校验、审批 |
| 记忆投毒 | provenance、隔离区、冲突检测、晋升审批、可撤销引用 |
| MCP/插件污染 | manifest、签名/来源、默认禁用、沙盒、最小权限和回归测试 |
| 模型越权执行 | capability token、PolicyGate、目标范围、预算、SideEffectLedger |
| 长期任务失控 | 心跳、租约、预算、超时、暂停、取消、人工接管和紧急停止 |
| 多模型降级风险 | 能力兼容检查、显式 RouteDecision、重新验证、高风险禁止静默 fallback |
| 审计证据缺失 | EventLog、TraceStore、Evidence、哈希、访问审计和归档策略 |

## 与四个专业方向的边界

- 软件开发：重点是代码、文件、构建、测试和交付副作用；
- 持续运维：重点是服务、资源、变更、恢复和生产影响；
- 安全评估与漏洞研究：重点是授权、目标范围、隔离验证和 PoC 副作用；
- 安全运营：重点是安全信号、调查、事件、处置和防护效果验证。

共享 Harness 提供统一控制，但不把四类领域的权限、证据分类和审批策略合并成一个默认策略。

## 待完成的设计门禁

在实现通用安全模型前，需要用具体 schema 和测试冻结：数据分类、出口决策、`AuthorizationContract`、`ResponsePolicy`、工具 manifest、记忆 provenance、事件保留、紧急停止、租约过期、恢复和模型切换语义。当前这些是设计任务，不是已实现安全保证。
