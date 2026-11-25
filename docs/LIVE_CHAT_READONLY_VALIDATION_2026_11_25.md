# 真实 Chat 接口与只读工具闭环验收

日期：2026-11-25（Asia/Shanghai）。代码基线 `4d8803a` 加本次 usage 解码修复。
仅使用用户已授权的第三方 Chat Completions 接口和模型
`deepseek-v4.1-flash-1017`；不代表其它协议、其它模型或 Phase 1 整体验收。

## 实际故障与修复

真实 steerBox HTTP transport 收到 HTTP 200；响应的 `usage` 包含对象类型的
`completion_tokens_details`。旧 wire response 复用了带 `deny_unknown_fields` 的
内部 `ProviderUsage`，完整 usage 解码失败，而三个基础计数字段的投影解码成功。
诊断只输出状态、字节数和允许字段的类型，不输出响应正文、认证头或密钥。

修复将上游 wire usage 与内部统计类型分开：上游未知扩展字段丢弃，不进入审计；
三个已知计数仍要求非负整数或缺失/null，内部 `ProviderUsage` 仍拒绝未知字段。
这不是取消决策/action 校验，也不把 reasoning 正文加入模型观察或日志。

普通及流式 fixture 回归均先实际复现 `ResponseParse`，修复后通过。
另检查三个已知字段各自的负数、小数、字符串和对象被拒绝，以及内部契约仍严格。
流式仅为离线解析器回归，实际 HTTP token streaming 未验收。

## 两次真实 CLI 验收

| 范围 | 单次纯观察 | 只读工具闭环 |
|---|---|---|
| 开始时间 | 17:38:20 | 20:04:26 |
| loop ID | `live-connect-20261125-173820-511e576b` | `live-connect-20261125-200426-63694c7f` |
| CLI | `loop provider` | `loop provider-readonly` |
| 实际模型调用 | 1 | 2 |
| 实际工具调用 | 0 | 2：`git_status`、`markdown_link_check` |
| 退出及终态 | exit 0，CompleteLoop / Completed | exit 0，CompleteLoop / Completed |
| 本地总耗时 | 3805 ms | 8841 ms |
| 新事件 / 成功模型记录 / 失败模型记录 | 7 / 1 / 0 | 22 / 2 / 0 |
| provider 报告 input/output tokens | 305 / 118 | 704 / 235；911 / 129 |

只读测试最多三轮，无自动重试；只授权项目外独立 Git 工作区中的合成 `README.md`。
实际记录两次 policy decision、两次 tool completion、一次 `model_action_verified`，
第一步模型审计 `tool_call_count=2` / `verification_status=passed`；下一模型步完成。
配置和合成文件字节未变。运行不允许文件修改、Bash、部署或其它外部副作用。

两次 probe 均在本地将单独凭据文件解析后注入子进程环境，只用于 HTTP 认证；
密钥不进入命令参数或模型输入。输出和本次新增的事件、成败模型日志逐项检查，
未发现该密钥的精确原文。此检查不是通用 DLP 或完整凭据生命周期安全证明。
凭据文件路径及正文不纳入仓库；测试不修改用户配置。

## 复现及证据

持久验证目录：`/temp/codex/steerbox-validation/live-chat-20261125/`。
其中 `probe.py` 保留带鉴权文件权限检查、输出抑制和日志增量检查的本机验证程序；
`workspace/README.md` 是合成文件。运行需要用户配置及授权的真实账号，会产生调用费用：

```text
python3 /temp/codex/steerbox-validation/live-chat-20261125/probe.py
python3 /temp/codex/steerbox-validation/live-chat-20261125/probe.py --readonly-probe
```

每次生成新身份，不复用旧 loop/task ID；同一模型成功不意味着 resolved fallback、
动态升档、金额预算、其它模型或 Responses/Anthropic 真实服务已通过。
原始模型正文不保存到诊断证据；持久业务审计仍在既有用户测试 runtime。

## 本地回归与审核

修复后全量 `cargo test --locked --offline --jobs 1 --quiet -- --test-threads=1`
实际退出 0：94 个 harness（含 0 项 doctest），1345 passed、0 failed/ignored。
严格 all-targets Clippy、lib/bin 构建、全仓 fmt 与 diff 检查通过。
Cargo 编译和正式回归均显式使用既有项目外 target：
`/temp/codex/steerbox-validation/foreground-upgrade-20261102/master-target`。
完整复验日志为上述持久目录中的 `full-test.log`、`clippy.log`、`build.log`。
有一次先前的测试参数核查误漏外置 target，复用了既有项目内缓存并只输出 help，
没有运行测试，不计入验收；未删除既有 target，以免误删原有构建产物。

主代理自审确认 production diff 仅分离 wire usage 并显式转换三个计数，
不改请求、认证、超时、重试、PolicyGate 或动作权限；新增四项测试，
不新增依赖。未进行本轮独立模型/人工 code review，不将自审称为独立审核。
回滚依据为本次 Git diff/提交，不需要另复制版本控制中的源文件。

## 尚未解除的门禁

- live Markdown mutation、通用文件/Bash 和真实开发任务验收仍独立待做。
- status/cancel、后台调度、跨进程恢复、对账及 fencing 未由本次测试证明。
- Codex 外部自动审批的 502 不属于这次 steerBox provider 返回；没有黑名单证据，
  不把审批服务故障记为模型安全拒绝，也不据此放宽 steerBox 的本地安全规则。
