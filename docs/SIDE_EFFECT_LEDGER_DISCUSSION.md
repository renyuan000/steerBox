# Side Effect Ledger Discussion Draft

## 文档目的

本文件用于定义 `steerBox` 中外部副作用记录、恢复、重放、补偿和审计的设计方向。

本文件是讨论稿，不是最终实现规格。

## 为什么需要 SideEffectLedger

agent restore 最大风险之一，不是本地状态恢复失败，而是外部动作已经发生。

例如：

- 已经调用安全响应接口
- 已经修改远程配置
- 已经提交代码或推送
- 已经删除资源
- 已经封禁账号/IP
- 已经发出告警或工单
- 已经调用外部扫描器或攻击模拟工具

如果 restore 后 agent 再次生成“类似但不完全相同”的动作，可能造成重复执行或语义回滚风险。

## 核心原则

- 外部副作用必须先记录，再执行或同步记录执行结果
- 不可逆动作 restore 后默认不自动重放
- 所有副作用必须有 idempotency / replay / compensation 策略
- 高风险副作用应绑定 checkpoint 和 approval
- side-effect ledger 是 audit 和 recovery 的共同基础

## SideEffectEvent 字段草案

```text
side_effect_id
goal_id
loop_id
trace_id
action_type
tool_name
target_system
request_fingerprint
request_summary
response_fingerprint
response_summary
idempotency_key
side_effect_type
risk_level
reversible
replay_policy
compensation_policy
approval_id
checkpoint_id_before
checkpoint_id_after
status
created_at
completed_at
```

## side_effect_type

```text
read_only_external
idempotent_write
reversible_write
irreversible_write
security_response
deployment_or_publish
credential_or_permission_change
network_scan_or_attack_simulation
notification_or_ticket
unknown
```

## replay_policy

```text
safe_replay
no_replay
replay_only_with_same_idempotency_key
manual_approval_required
fork_only
manual_reconcile
```

## compensation_policy

```text
none
automatic_rollback_available
manual_compensation_required
compensation_not_possible
unknown
```

## 执行前检查

高风险外部动作执行前必须检查：

- 是否在 GoalContract scope 内
- 是否有 checkpoint
- 是否有 approval
- 是否有 idempotency key
- 是否有 compensation plan
- 是否记录 target_system
- 是否记录 request fingerprint
- restore 后是否允许重放

## restore 后处理

restore 后必须执行 side-effect reconciliation：

```text
restore checkpoint
  -> read side-effect ledger after checkpoint
  -> classify side effects
  -> decide replay/no_replay/fork/manual_reconcile
  -> block unsafe automatic replay
  -> write reconciliation event
```

## 与安全智能体的关系

安全智能体尤其需要 SideEffectLedger。

高风险动作包括：

- 自动封禁
- 自动隔离资产
- 修改防火墙/ACL
- 触发扫描
- 触发攻击模拟
- 删除恶意文件
- 重启服务
- 修改安全策略

这些动作默认不应在 restore 后自动重放。

## 与软件开发智能体的关系

开发智能体也需要 SideEffectLedger。

高风险动作包括：

- git push
- 创建 PR
- 修改 CI/CD 配置
- 发布版本
- 删除分支
- 修改全局配置
- 修改依赖锁文件并发布

## 第一阶段建议

第一阶段先实现文档/schema 级设计：

- `SideEffectEvent`
- `ReplayPolicy`
- `CompensationPolicy`
- `SideEffectReconciliationEvent`

实现时，所有外部 tool plugin 都必须声明：

- side_effect_type
- replay_policy
- compensation_policy
- idempotency support

## 当前结论

没有 SideEffectLedger 的 checkpoint/restore 对真实长期 agent 不够安全。

`steerBox` 应把 side effect ledger 作为 checkpoint、policy gate、audit 和 recovery 的共同基础设施。
