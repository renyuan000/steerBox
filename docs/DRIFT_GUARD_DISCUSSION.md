# Drift Guard Discussion Draft

## 文档目的

本文件用于定义 `steerBox` 如何防止自主 loop 中的 agent 跑偏、扩大范围、过度重构或破坏已经成型的项目结构。

本文件是讨论稿，不是最终实现规格。

## 问题定义

自主 agent 最危险的行为之一是：

> 用看似合理的理由，把本来已经成型的代码、架构、文档或安全策略改得面目全非。

典型表现：

- scope creep
- over-refactor
- architecture drift
- style churn
- deleting edge cases
- weakening security boundaries
- changing behavior to satisfy tests only
- replacing project-specific design with generic pattern
- modifying stable files without explicit need

## DriftGuard 核心目标

- 保持 GoalContract 范围
- 保护稳定 artifacts
- 阻止无授权架构漂移
- 阻止过大 diff
- 阻止行为静默变化
- 阻止安全边界弱化
- 在跑偏早期暂停 loop

## Drift 类型

### 1. Scope Drift

表现：

- 修改不在 scope 的文件
- 解决未要求的问题
- 把“顺手优化”加入任务

处理：

- block 或请求人工审批
- 写入 drift event
- 要求更新 GoalContract 才能继续

### 2. Architecture Drift

表现：

- 替换原架构模式
- 改模块边界
- 改插件机制
- 改数据流方向

处理：

- 触发 architecture invariant check
- 需要 ADR 或人工审批

### 3. Quality Drift

表现：

- 测试通过但代码质量下降
- 删除边界条件
- 弱化错误处理
- 增加隐式依赖

处理：

- evaluator / review gate
- regression test
- diff explanation

### 4. Security Drift

表现：

- 降低权限检查
- 删除审计
- 放宽策略
- 绕过 sandbox
- 弱化验证

处理：

- block by default
- security evaluator
- human approval

### 5. Style / Churn Drift

表现：

- 大量无关格式改动
- 重命名无关变量
- 大范围移动代码
- 把项目风格替换成模型偏好风格

处理：

- diff size gate
- style churn detection
- require explicit justification

## DriftGuard 输入

```text
goal_contract
action_plan
diff_summary
affected_files
stable_artifacts
architecture_invariants
security_invariants
verification_result
loop_history
tool_history
```

## DriftGuard 输出

```text
decision: allow | warn | require_checkpoint | require_evaluator | require_human_approval | block
reason
risk_level
drift_type
required_action
checkpoint_required
```

## 固定检查

每轮 loop 检查：

- 是否仍在 goal scope 内
- 是否触碰 out_of_scope
- 是否修改 stable artifacts
- 是否需要 checkpoint
- 是否已有验证证据
- 是否连续多轮无验证
- 是否出现重复失败但继续推进

## diff 检查

建议指标：

- changed_files_count
- changed_lines_count
- deleted_lines_count
- renamed_files_count
- touched_core_files_count
- touched_security_files_count
- out_of_scope_files_count
- behavior_change_declared
- tests_updated

## stable artifacts

稳定 artifacts 可以包括：

- 已发布模块
- 用户标记的稳定 checkpoint
- 核心配置
- 安全策略
- 公共 API
- 数据库 schema
- 协议定义
- CI/CD 配置
- 文档总纲

## 处理策略

### allow

低风险、scope 内、小 diff、有验证。

### warn

轻微异常，但可继续。

### require_checkpoint

可能影响恢复，需要先 checkpoint。

### require_evaluator

质量或行为可能有变化，需要独立验证。

### require_human_approval

风险较高或触碰稳定 artifact。

### block

明确越 scope、破坏安全边界、无授权删除或不可逆高风险动作。

## 与 CheckpointPolicy 的关系

DriftGuard 不是替代 checkpoint，而是触发 checkpoint 或阻止 restore 后继续跑偏。

典型联动：

```text
DriftGuard detects high-risk diff
  -> CheckpointGate requires hard checkpoint
  -> EvaluatorGate runs review
  -> HumanSteeringGate requests approval
```

## 与 PromptProfile 的关系

不同模型跑偏模式不同，因此 DriftGuard 应与 ModelProfile 关联。

例如：

- 容易 over-refactor 的模型：降低 diff limit
- 工具格式不稳定的模型：提高 ToolGate 严格度
- 长上下文注意力弱的模型：缩小 task scope
- 强模型但自信过度：强制 evaluator 或 review gate

## 第一阶段建议

先实现文档/schema 级设计：

- `DriftGuardPolicy`
- `DriftEvent`
- `ArchitectureInvariant`
- `StableArtifact`
- `DiffRiskSummary`

第一阶段检查可以先是静态和人工可读：

- changed files
- changed lines
- out-of-scope paths
- stable artifacts touched
- security files touched
- behavior change declared or not

## 当前结论

DriftGuard 是自主 loop 的必要安全层。

没有 DriftGuard，agent 即使有 checkpoint，也可能不断把项目往错误方向推进；checkpoint 只能恢复，不能提前判断“这一步是不是不该做”。
