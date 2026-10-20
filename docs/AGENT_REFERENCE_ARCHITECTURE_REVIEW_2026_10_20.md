# AI Agent 参考架构与交互设计审核

**日期**：2026-10-20
**状态**：设计审核；不是源码完整审计，也不是实现完成声明

## 目的与证据等级

本文把用户指定的四个参考入口与 steerBox 的设计目标对齐，重点关注：

- 多模型接入、路由和 fallback；
- 长任务历史、执行过程和恢复可见性；
- 流式输出、TUI/Web 交互和低打扰后台运行；
- 凭据、endpoint 和模型输入之间的边界。

证据分三类：

1. **已核对入口**：本次只核对目录和文件名，确认仓库路径存在；没有把文件名当成行为证明。
2. **用户观察/已有 steerBox 文档**：例如 Claude 输出紧凑、DeepSeek Harness 的历史/流程查看、OpenCode 的鼠标可交互 Web 界面。
3. **设计推断**：根据入口形态提出可借鉴的接口，不代表参考项目一定采用了相同实现。

本次确认存在的参考入口：

| 项目 | 本次核对到的入口 | 可研究方向 | 当前证据边界 |
|---|---|---|---|
| Codex | `codex-rs/tui`、TUI suite tests、`codex-rs/agent-*` | 终端交互、键盘可达性、会话/agent 边界 | 仅核对目录与文件名，未做完整行为审计 |
| OpenCode | `packages/web`、`packages/ui`、UI hooks 和选择/滚动相关模块 | Web 鼠标交互、自动滚动、文件/差异选择 | 仅核对目录与文件名，未做完整行为审计 |
| DeepSeek Harness | `apps/cli`、`apps/web`、`snapshots/session`、`snapshots/web`、`dsh-client-ui-ux` | 执行历史、会话生命周期、子 agent、Web 体验 | 仅核对目录与文件名，未做完整行为审计 |
| Claude Code reference | `src/src/cli`、structured/remote IO、auto mode handlers | 紧凑输出、流式/结构化 IO、自动模式交互 | 该目录仅作合法合规架构研究，不复制或传播受限内容 |

## 采纳的设计结论

### 1. 路由是“AI 辅助、运行时裁决”

模型可以提出策略建议，例如选择 `fast`、`balanced`、`reasoning`，或在质量不足时请求升级到 `xhigh`。但下列内容始终由运行时控制：

- endpoint 和 provider allowlist；
- `auth_ref` 与 endpoint 的固定绑定；
- 数据出口、预算、并发、重试和 fallback 上限；
- 是否允许把某个数据等级发送到某个 provider。

建议的稳定边界：

```text
CandidateFilter      运行时硬过滤
StrategySelector     AI/规则提出质量档位或候选 profile
RouteExecutor        运行时校验、切换、fallback、审计
ProviderAdapter      使用受保护 endpoint/credential 发请求
```

AI 不能通过修改配置、prompt 或工具参数把 key 发送到任意 URL。

### 2. 凭据保护是模型输入边界，不是 OS 文件位置一刀切

- API key、密码、token、cookie、私钥和认证响应不进入 prompt、上下文、工具返回、trace、event 或 memory。
- endpoint 只作为运行时传输参数，不作为模型输入。
- 普通软件配置、日志和性能数据可以在授权范围内读取和分析。
- 本地工具如果必须使用凭据，只向模型返回成功/失败、状态码或脱敏摘要，不返回凭据正文。
- 读取控制和外发控制分开：允许本地使用不等于允许发送给远程模型。

### 3. 交互输出采用“紧凑摘要 + 渐进展开”

用户对 Claude 的观察对应一个明确目标：减少空白和冗余，不牺牲审计信息。steerBox 的交互 adapter 应优先：

- 首屏给出一行状态、当前动作、耗时/预算和下一步；
- 工具调用和模型思考显示为短行事件，详细参数按需展开；
- 将长 JSON、trace 和历史放入可查询的详情视图，而不是默认纵向刷屏；
- 终端支持键盘优先；Web adapter 再提供鼠标选择、滚动、过滤和折叠；
- CLI、TUI、Web 共用同一个结构化 `ProgressEvent`/execution projection，不让 UI 重新解释事实；
- 后台运行默认不因低风险路由、性能升级或只读动作反复询问用户。

这部分是交互目标，不表示当前 Phase 1 已有 TUI/Web。

## 不采纳或暂不承诺的内容

- 不把参考项目的目录存在误报为源码行为或性能结论；
- 不复制疑似泄露内容、系统提示、凭据或受限正文；
- 不把“AI 自我约束”当作唯一安全边界；
- 不在 Phase 1 直接实现完整 Web 控制面、鼠标交互和多项目后台调度；
- 不因追求 UI 丰富度引入每 token 的同步策略检查或频繁审批，避免像当前用户观察到的 Codex ask/auto 体验一样反复打断。

## 分阶段落地

### Phase 1 必须冻结的接口

- provider-neutral `RouteDecision`、`ModelCallRecord` 和 `ProgressEvent`；
- 运行时硬过滤与 endpoint/auth 绑定校验；
- 性能档位的有限升级（例如 `balanced -> xhigh`），不改变安全底线；
- metadata-only 的执行历史和可查询 projection；
- 无真实密钥的 fake/localhost contract tests。

### Phase 1 之后的交互工作

1. 先做 CLI 的紧凑流式摘要和详情查询；
2. 再做 TUI：键盘导航、折叠详情、固定状态栏、宽度自适应；
3. 最后做 Web adapter：鼠标选择、过滤、历史时间线和运行中接管；
4. 以首字节延迟、事件到达延迟、默认输出行数、滚动可用性和后台零打扰运行作为验收指标。

## 审核结论

- **采纳**：Claude 的紧凑、及时反馈目标；OpenCode 的鼠标可交互方向；DeepSeek Harness 的历史/执行流程可见性；Codex 的 TUI 测试和结构化 agent 边界作为研究入口。
- **保持 steerBox 自身优势**：EventLog、RouteExplanation、预算、PolicyGate、checkpoint 和审计链不能为追求短输出而隐藏或绕过。
- **当前状态**：本文完成的是设计收敛与证据分层；四个参考项目尚未完成完整源码级比较，steerBox 也尚未实现完整 TUI/Web 或真实多模型自动路由。
