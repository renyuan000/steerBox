# steerBox 自主风险判断与敏感数据边界设计

**状态**：设计草案，已获用户暂定方向批准，待评审

**日期**：2026-10-12

## 1. 目标

steerBox 面向长程、后台、无人值守任务。它必须在不反复打断用户的前提下，自动判断绝大多数动作是否安全，同时保护以下信息：

- 敏感个人信息；
- 敏感系统信息；
- 敏感配置文件；
- 软件帐号、权限和会话信息；
- API key、密码、Token、Cookie、私钥等认证材料；
- 其它项目机密或受约束数据。

安全策略的目标不是“看到敏感字样就停止”，而是让智能体在最小权限内继续工作：

1. 项目内代码、测试、文档和受控临时目录可以自动处理；
2. 敏感数据优先使用元数据、脱敏视图或本地受控能力；
3. 原始凭据不进入模型上下文、普通日志、事件 payload 或外部网络；
4. 高风险或副作用不确定的动作进入 `pending_human`，后台不循环询问，也不自动重试绕过策略；
5. 任务中不相关的安全动作仍可继续执行。

本设计仍属于共享 Harness 核心和软件开发 Agent 垂直切片，不把 Phase 1 扩展为完整 DLP、企业 IAM 或安全运营产品。

## 2. 非目标

- 不承诺自动发现所有隐含机密；
- 不把模型的自然语言判断当作授权依据；
- 不在 Phase 1 实现生产级密码库、完整 TUI/Web 控制面或所有第三方登录流程；
- 不允许 `auto` 覆盖绝对禁止规则；
- 不把平台事实分析器直接变成权限授予器。

## 3. 四层决策架构

```text
内置硬边界
    ↓
人类配置（全局用户 → 项目 → 任务/会话）
    ↓
平台环境事实（Windows/Linux/macOS）
    ↓
数据分类 + 动作分类 + 目标分类
    ↓
确定性 PolicyEvaluator
    ↓
低风险自动执行 / 脱敏执行 / 最小能力增量 / pending_human / block
```

模型只能提交动作请求，并可提供风险解释或分类候选；模型不能提升权限、修改策略或把自己的“安全判断”当成批准。

决策优先级固定为：

```text
绝对禁止
  > 人类显式禁止
  > 项目/任务策略
  > 已登记能力与 recipe
  > 内置安全默认值
  > 平台环境事实
  > 模型请求
```

人类策略默认只能收紧权限。只有非绝对禁区，才允许人类显式扩大范围；扩大范围必须有作用域、期限、来源和审计记录。

## 4. 数据分类

### 4.1 分类级别

| 级别 | 含义 | 例子 |
|---|---|---|
| `D0_PUBLIC` | 可公开 | 公开文档、许可证、普通帮助文本 |
| `D1_PROJECT` | 普通项目数据 | 源码、测试、普通配置、项目文档 |
| `D2_SENSITIVE` | 敏感个人/系统/配置 | 用户标识、内网地址、主机信息、客户数据、系统配置 |
| `D3_AUTH_MATERIAL` | 认证和会话材料 | API key、密码、Token、Cookie、私钥、SSH key、云凭据 |
| `D4_CRITICAL` | 高危或受监管数据 | 密码库、生产密钥、支付/身份/医疗/财务数据、核心企业机密 |

帐号信息进一步区分：

- `AccountIdentity`：用户名、邮箱、组织/租户 ID；
- `AccountAuthorization`：角色、权限、配额、计费状态；
- `AuthMaterial`：密码、Token、Cookie、私钥、刷新凭据；
- `SessionMaterial`：登录会话、设备绑定和浏览器会话；
- `AccountOperation`：登录、发布、推送、创建 Token、修改权限。

分类结果必须带 `source`、`confidence`、`evidence_refs` 和 `classifier_version`。任何分类器都不得把原始敏感值写入证据。

### 4.2 分类来源

分类按可信度排序：

1. 人类明确标记；
2. 内置路径/扩展名/字段规则；
3. 平台凭据存储事实；
4. 内容扫描和结构化解析；
5. 本地小模型或规则模型提出的候选分类；
6. 远程模型提出的候选分类。

模型提出的分类只能提高风险或触发脱敏，不能降低风险。

## 5. 动作与目标分类

### 5.1 动作

- `MetadataRead`：文件名、大小、时间、权限等元数据；
- `LocalRead`：本地读取；
- `LocalTransform`：搜索、解析、摘要、脱敏、校验；
- `WorkspaceWrite`：项目工作区或受控临时目录写入；
- `ToolExec`：测试、构建、格式化、脚本执行；
- `ExternalSend`：向模型、HTTP 服务、仓库或其它外部目标发送数据；
- `IrreversibleChange`：删除、覆盖、发布、权限修改、服务修改；
- `AccountOperation`：使用或改变软件帐号和认证状态。

### 5.2 目标

- `LocalTrustedTool`：本地、注册过的工具；
- `LocalModel`：本地模型；
- `RemoteModel`：远程模型或中转平台；
- `AllowlistedService`：人类登记的外部服务；
- `UnregisteredExternal`：未登记的网络目标；
- `AuditStore`：EventLog、TraceStore、ModelCallStore、Memory/Wiki；
- `UserSurface`：终端、TUI、Web 或控制面。

风险评估至少计算：

```text
数据敏感级别 + 动作副作用 + 目标可信度 + 外发性质
+ 可逆性 + 执行权限 + 路径/命令确定性
```

## 6. 默认决策矩阵

| 场景 | 前台 | 后台 |
|---|---|---|
| 项目内读源码、读测试 | 自动允许 | 自动允许 |
| 项目内改源码、测试、文档 | 自动允许，checkpoint + verification | 自动允许，checkpoint + verification |
| 受控 `/tmp/steerbox/<project>/` 临时文件 | 自动允许 | 自动允许 |
| 普通测试、格式化、固定安全 recipe | 自动允许 | 自动允许 |
| 敏感文件元数据或脱敏视图 | 自动允许 | 自动允许 |
| 敏感文件原文交给本地受控工具 | 最小能力增量 | 已有任务授权则自动，否则 `pending_human` |
| 敏感文件原文发送远程模型 | 明确审批 | 阻断或 `pending_human` |
| 使用已登记帐号能力 | 受范围和期限限制 | 受范围和期限限制，凭据不暴露给模型 |
| 未登记外部网络 | 询问或阻断 | 阻断 |
| 删除、发布、改系统服务/权限 | 询问 | `pending_human`，不反复询问 |
| 密码库、私钥、生产凭据外发 | 永久阻断 | 永久阻断 |

`/tmp` 不是天然安全区。路径允许只代表路径层级允许，文件内容仍需分类；同样，项目工作区内的 `.env`、`kubeconfig`、`*.pem`、`*.key`、浏览器 profile 和凭据文件仍按敏感数据处理。

## 7. 自主风险判断

### 7.1 三段式判断

每个动作执行前必须经过：

1. **确定性预检**：解析命令、路径、目标、数据标签和副作用；
2. **风险分类**：规则引擎先判断；不确定时可调用本地小模型辅助分类；
3. **策略决定**：PolicyEvaluator 根据硬规则、授权快照、环境事实和分类结果决定。

本地小模型可以帮助识别“这段文本是否像帐号配置”“这个命令是否可能删除文件”，但不能降低 `D3/D4` 风险，也不能覆盖 hard deny。

### 7.2 置信度与降级

每个判断返回：

```text
decision
risk_level
confidence
matched_rules
evidence_refs
required_capabilities
redaction_plan
policy_revision
```

降级规则：

- `confidence >= high` 且命中明确低风险 recipe：自动执行；
- 中等置信度：只允许元数据、脱敏视图或本地可逆操作；
- 低置信度：不猜测，进入 `pending_human` 或 `block`；
- 检测器失败、环境事实缺失、策略快照无效：不能回退到全权限。

### 7.3 后台行为

后台模式绝不进入“重复询问循环”：

- 低风险动作直接执行；
- 可安全降级的动作改为脱敏、摘要或本地处理；
- 高风险动作生成稳定的 `approval_request_id`，状态置为 `pending_human`；
- LoopController 继续其它不相关的安全动作；
- 相同请求不得重复创建、重复执行或通过重试绕过策略；
- 用户稍后批准时，只能恢复该请求绑定的最小能力，不扩大到整个任务。

## 8. 帐号与凭据能力

真实帐号信息必须通过 opaque 引用使用：

```text
model request
  -> account_alias + operation + target
  -> PolicyEvaluator
  -> AccountCapability / SecretBroker
  -> local provider/tool
  -> redacted result + reference_id
```

普通运行时只接触：

- `auth_ref` 或 `account_alias`；
- scope（global/project/environment/task）；
- operation；
- 允许目标；
- 期限；
- 结果状态和不可逆副作用摘要。

原始凭据只能在受控 provider/tool 边界短暂读取，不进入 prompt、EventLog、Trace、memory、wiki 或错误信息。

## 9. 审批与自动授权

审批不是全局布尔值。请求必须绑定：

```text
request_id
session/task/goal
permission_kind
tool_id
path_or_target_pattern
data_class
operation
recipe_hash
policy_revision
expires_at
```

支持三种结果：

- `once`：只放行当前调用；
- `scoped`：在当前任务/项目和 TTL 内复用；
- `reject`：拒绝当前请求并关闭相关重复请求。

不允许提供对 destructive command、动态路径或未解析 shell 的宽泛永久授权。

## 10. 审计与脱敏

EventLog、TraceStore、ModelCallStore、MemoryCandidate 和用户输出统一执行：

- 结构化敏感字段拒绝或替换；
- 文本模式脱敏；
- 路径、帐号和目标使用稳定 hash 或 alias；
- 记录命中规则、风险等级、策略版本和结果；
- 不记录原始凭据、原始敏感文本、完整命令中的秘密参数。

审计目标是回答“为什么允许/拒绝”，而不是保存被保护的数据正文。

## 11. 平台事实分析器

Windows、Linux、macOS 分别实现 `EnvironmentAnalyzer`，只生成事实：工作区、可写根、受保护根、当前身份、网络状态、凭据存储、沙箱和容器边界。分析器不能直接扩大权限。

环境事实缺失或解析失败时，策略必须 fail-closed；不能把 `unknown` 当成 `full_access`。

## 12. Phase 1 最小实现边界

实现顺序建议为：

1. `DataClass`、`ActionClass`、`DataFlowTarget` 和 `RiskEvidence`；
2. `PolicySnapshot` 与固定优先级合并；
3. 路径/命令/目标结构化预检；
4. 脱敏视图和统一审计过滤器；
5. `AccountCapability`/`auth_ref` 边界；
6. `pending_human`、幂等 approval request 和后台继续执行；
7. Windows/Linux/macOS analyzer trait 及当前平台实现；
8. 端到端测试：正常代码编辑不询问、`/tmp` 受控写入不询问、敏感原文不外发、高风险动作只挂起一次。

真实远程 LLM 验证仍需单独配置测试帐号，并且必须在测试前通知用户；本设计不读取或保存任何真实密钥。

## 13. 研究与业界依据

本设计不是把某个模型的提示词当成安全边界，而是吸收以下共识：

- AgentDojo（arXiv:2406.13352）将工具返回的不可信内容、间接提示注入和工具动作放在动态环境中评估；这支持“工具输出是数据，不是授权指令”的设计原则。
- Fides（arXiv:2505.23643）将机密性/完整性标签、动态信息流和确定性策略结合，并通过选择性隐藏扩大安全可执行任务范围；这支持 `DataClass`、来源/溯源和脱敏视图。
- CaMeL（arXiv:2503.18813）把控制流与数据流分离，并在工具调用边界使用 capability；这支持 `AccountCapability` 和 `auth_ref`。
- OWASP LLM06:2025 的 Agent 安全指导强调最小权限、权限控制、提示注入和过度自主性；这支持将审批、沙箱、网络、帐号能力和数据流分层。

这些研究只能提供威胁模型、控制原则和评估方法，不能证明 steerBox 已经安全。Phase 1 仍需用本地 fixture 和后续真实 provider 测试验证：

1. 正常代码编辑不会产生无意义审批；
2. 敏感内容不会进入模型、日志或外发通道；
3. 工具输出中的指令不会自动改变权限；
4. 高风险请求只生成一次可恢复的 pending 状态；
5. 脱敏或能力代理不会破坏正常开发任务。

## 14. 研究原则的实用性评估

| 来源/原则 | 实用性 | Phase 1 采用方式 | 暂不采用 |
|---|---|---|---|
| AgentDojo：工具输出可能包含间接提示注入 | 高 | 把工具输出视为不可信数据；权限只能由本地策略产生；增加注入回归 fixture | 不把 benchmark 当成生产安全证明 |
| Fides：数据标签、信息流和选择性隐藏 | 中高 | 采用粗粒度 `DataClass`、来源、目标和脱敏计划 | 暂不做全量动态 taint tracking |
| CaMeL：控制流与数据流分离、capability 使用 | 高 | `AccountCapability`、`auth_ref`、模型只请求能力，工具读取真实凭据 | 暂不引入完整安全解释器或重写所有工具 |
| OWASP/NIST：最小权限、身份授权、过度代理、提示注入 | 高 | 作为 PolicyGate、沙箱、网络和帐号能力的验收标准 | 不把分类清单当作完整合规认证 |
| 新的工具拦截/隐私代理原型 | 中 | 借鉴“模型上下文外拦截器”边界 | 不在 Phase 1 依赖研究原型作为基础设施 |

结论：论文中的“信息流控制”和“capability”适合直接转成 steerBox 的边界；完整 taint tracking、通用安全解释器和研究型拦截器不适合在 Phase 1 一次性实现。最小可用实现应先保护模型、工具、外部网络和审计存储之间的边界。

## 15. 脱敏服务与帐号使用方案

### 15.1 Phase 1：本地进程内脱敏边界

Phase 1 不把原文发送给远程“脱敏 LLM”，也不依赖网络脱敏服务。首选本地、确定性的脱敏组件，在以下边界统一执行：

- tool input；
- tool output；
- model request；
- model response；
- EventLog/Trace/ModelCallStore；
- memory/wiki candidate；
- 用户终端和控制面输出。

处理顺序：

```text
结构化字段过滤
  → 路径/扩展名/凭据格式规则
  → 本地内容分类器（可选）
  → 替换为 typed placeholder/opaque alias
  → 外发前最终 egress guard
```

例如：

```text
[REDACTED:AUTH_TOKEN]
[REDACTED:PRIVATE_KEY]
[ACCOUNT_ALIAS:github-work]
[PATH_ALIAS:workspace-config-1]
```

脱敏必须可重复、可审计，但不能通过可逆编码把原文藏在日志里。需要保留跨步骤关联时使用不可逆 hash 或 task-scoped alias。

### 15.2 何时使用独立本地脱敏进程

独立本地 sidecar/daemon 不是 Phase 1 默认依赖，但在以下情况有价值：

- 第三方插件或不可信工具会接触工具输出；
- 需要隔离 Rust 主进程的解析崩溃或规则更新；
- 需要组织级统一脱敏策略；
- 希望使用 OS 权限把脱敏器限制为只读输入、只写脱敏输出。

即使使用 sidecar，也必须本机运行、受 Unix socket/Named Pipe ACL 保护，不能把原文发送给远程模型来“帮忙脱敏”。

### 15.3 远程登录和帐号能力

用户的判断是正确的：远程登录所需的用户名、密码、Cookie、Token 和私钥通常不应发送给 LLM。更好的闭环是：

```text
LLM 返回意图/方法/参数模板
  → PolicyGate 校验目标、动作、数据类别和帐号能力
  → AccountCapability 使用 auth_ref 调用本地 broker/provider
  → 真实凭据只在受控执行边界出现
  → LLM 只收到状态、脱敏结果和 reference_id
```

执行约束：

- 不把密码放进命令行参数；
- 不把 Token 放进普通环境变量、EventLog 或错误字符串；
- 优先使用 OS secret store、受保护 stdin、secret FD 或 provider SDK 的 credential callback；
- 用户名、邮箱、组织 ID 默认按 D2 处理，可在策略允许时使用 alias；
- MFA、验证码、设备确认和权限提升不能由 Agent 绕过；
- 登录失败只返回分类后的失败原因，不返回认证材料或完整服务响应；
- 登录动作仍要记录目标、帐号 alias、policy revision、结果和副作用，不记录凭据正文。

这样 LLM 可以提出“登录 GitHub、读取某仓库、创建分支”的方法，真实帐号仍由 steerBox 的能力层使用，不需要让 LLM 知道密码。

## 16. 长期安全领域扩展与分期

共享 Harness 必须长期支持软件开发、持续运维、安全评估/漏洞研究和安全运营，但这些方向不能共享同一个“默认允许”策略。相同工具在不同领域、目标和授权下可能有完全不同的风险。

### 16.1 共享 Harness 层

所有领域共用：

- Goal/TaskRun/Attempt/Loop 生命周期；
- EventLog、StateStore、Trace、Evidence 和 SideEffectLedger；
- DataClass、DataFlow、Capability 和 PolicyEvaluator；
- checkpoint、lease、heartbeat、budget、pause/cancel/takeover；
- provider/model/prompt 审计和 egress guard；
- 统一的 pending、恢复、幂等和紧急停止语义。

### 16.2 领域能力包

**软件开发包**：工作区读写、测试、构建、发布和代码审查。默认目标是项目工作区，普通编辑和验证可以自动执行。

**持续运维包**：服务巡检、容量、性能、部署、恢复和生产变更。只读观察可以长期自动运行；生产变更、服务重启、凭据轮换和批量隔离需要 `ChangeAuthorization` 与回滚计划。

**安全评估/漏洞研究包**：代码/依赖分析、攻击面发现、授权扫描、隔离 PoC 和修复复测。主动动作必须绑定 `AuthorizationContract`、目标/排除范围、限速、时间窗、环境边界和停止条件。

**安全运营包**：信号归一化、调查、威胁假设、事件关联、受控处置和效果验证。低风险标记、聚合和报告可以自动运行；阻断、删除、生产隔离、外部通知和 hack-back 永远不由默认策略自动放行。

### 16.3 长期运行必备约束

长时间运行不代表永久授权。每个常驻任务必须具备：

- 授权 TTL 和过期后的自动停机/降级；
- lease、heartbeat、预算、背压和死循环检测；
- 模型、Provider、工具、规则或目标发生变化时的重新评估；
- 检查点恢复时重新校验授权和环境事实；
- 外部网络、敏感数据出口和能力调用的持续审计；
- 紧急停止和人工 takeover；
- 不确定副作用不得自动重放。

### 16.4 分期路线

1. **Phase 1**：共享 Harness + 软件开发垂直切片；为其它领域冻结契约和边界，不实现主动渗透或完整安全运营。
2. **安全平面切片**：DataClass、Capability、AuthorizationContract、脱敏、auth_ref、pending_human、Durable Worker 和本地恢复。
3. **安全评估切片**：先做被动分析、资产/依赖/配置审计和隔离靶场 fixture，再加入授权范围内的低风险主动验证。
4. **长期运维切片**：先做只读巡检、证据归档和低风险 playbook，再加入带回滚的中风险变更。
5. **安全运营切片**：信号、Finding、Incident、ResponseAction、效果验证和人工接管闭环。
6. **受控攻防研究**：在独立授权、隔离环境、限速、证据和停止条件齐备后，才增加更强的漏洞复现和攻防协作能力。

每个阶段都必须有独立的能力清单、授权契约、数据出口规则和验收测试，不能因为共享 Harness 已经存在，就默认获得其它领域的主动权限。
