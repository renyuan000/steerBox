# Foreground 单次效果升档契约

日期：2026-11-07。本文定义显式 foreground 单次升档入口的行为；实现及验收状态以实际代码和 `PHASE_1_VALIDATION_PLAN.md` 为准，不能仅将契约视为已交付证据。

## 范围与入口

复用已有 `RuntimeConfig::build_effect_bound_candidate()` 和 controller 的单次升档流程，将其接到纯观察 foreground runner。新增 `ForegroundProviderRun::prepare_upgrade()` 返回独立的 `ForegroundUpgradeProviderRun`；调用新入口和 profile 的 `effect.automatic_upgrade=true` 必须同时满足。缺少 effect、未开启升级、非 OpenAI-compatible 协议均在 prepare 拒绝。

旧 one-shot 入口保持原行为，包括拒绝 automatic-upgrade profile；不将旧调用静默改成两次调用。Markdown、Anthropic 和 CLI 不接入本片自动升档。没有新增启发式质量自评、模型切换或额外 LLM evaluator 调用。

新类型提供 transport 注入和 credential resolver 两个执行入口；两者都要求受信任宿主传入质量 evaluator。连接信息与凭据不进入 evaluator 的模型响应上下文。新入口因既有 owned candidate 工厂而需要 `'static` transport/resolver；不增加旧接口约束，也不要求 resolver 实现 Clone。

## 冻结与权限边界

- prepare 重新验证公开配置，冻结所选 profile、effect mapping、初始/最高档位、reported-token 限额、prompt 和 route。准备后修改原配置不得改变首轮或延迟构造的 child；快照测试须同时覆盖 mapping/档位、endpoint/auth_ref/model 和限额变更。
- 两轮共享同一 transport/HTTP client；每次网络发送可由 transport 解析凭据，不承诺只解析一次。runner 不主动读取、打印或持久化凭据正文。
- 执行前复用可信、串行、同一日志集合的历史身份门禁。重复或不可安全读取的同一历史，在创建 controller 和发送请求前失败；resolver 调用次数为零。这不是并发原子认领或跨进程恢复。
- 首轮 `step-1` 保持既有命名。child 使用 controller 给出的精确 route 重新 resolve 新 prompt pack，不修改私有 metadata。两轮 model/provider/endpoint/auth_ref、mapping 和上限，以及 prompt/policy/tool/context/privacy/risk 边界不变。

## 次数与质量

同一 TaskRun 预置最多 2 steps、2 model calls、2 Attempts；额度在运行前确定，child 不补预算。即使最大档位高于下一档，也最多升相邻一档、调用两次，不发第三次。reported-token 账本仍由原 controller 管理：关闭、真实零、未知、耗尽和审计故障保持现有语义，不是单请求硬上限或金额预算。

仅成功解析、无工具动作、非 RequestHuman 的结果可交给可信宿主 evaluator。只有 LowQuality/NoProgress 能发起升级；安全、provider、解析、工具、verification 错误不能当作质量不足重调。除 RequestHuman 直接返回外，首轮或 child 都须明确质量接受；child 不合格则失败关闭。接受的 Continue 返回 Running，不自动追加调用；RequestHuman 保留原 controller 语义，不伪报 Completed。

先有 child 元数据、再被 token 发送门禁拒绝是允许状态；“零 child 发送”不能误写成“零 child 记录”。审计和进度故障也不得引发补发。

## 验收

直接 runner 测试需覆盖：low→medium 的实际 wire、不同 call/route/pack 身份、父子 Attempt 事件、累计次数与 token、StateStore/ExecutionQuery 重建；首轮接受、Continue、RequestHuman；child 低质量或证据不足且无第三调；provider/parser/action/security 拒绝；限额关闭、真实零、0 限额、unknown、首轮耗尽和两轮累计；重复/坏历史、审计和进度故障；准备前非法配置与准备后外部修改；非 Clone resolver 和旧入口回归。只用合成数据及 fake/unavailable transport/resolver。

## 非目标

不实现生产 CLI 自动质量选择、Markdown 升档、Anthropic native 效果、多模型 fallback、并发认领、在途请求抢占、token 预留硬上限、金额预算、持久 TaskRun 恢复或 live provider 验收。本升档片不调用 fallback；后续独立的 resolved Goal 模型白名单修复见 `RESOLVED_MODEL_ALLOWLIST_CONTRACT.md`。其他候选 resolver/binding 错误的失败收口仍是单独待办，不能因白名单修复而视为全部解决。
