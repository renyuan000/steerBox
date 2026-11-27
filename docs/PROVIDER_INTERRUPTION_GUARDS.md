# Provider 中断防护与验收边界

状态：2026-11-27，受限解析及 HTTP 超时修复；不是生产流式接入或自动恢复交付。

后续同日纯观察有界恢复切片见 `PROVIDER_RESPONSE_RECOVERY.md`。本文件保留 `63913ab`
中断防护验收的历史范围，下文“没有新增重试”仅指该切片，不覆盖后续显式宿主库 API。

## 证据边界

用户报告 OpenCode 经第三方中转调用 DeepSeek 时，偶发只有思考/行动、无最终结果。
缺少失败请求追踪、终止帧及安装版本，不能确认中转断流、客户端解析、会话绑定或中文
项目路径是根因。本轮已读取以下 OpenCode 官方仓库报告，后续 API 复查返回 403，未取得更新：

| 证据 | 读取时状态 | 参考机制，不是现场定论 |
|---|---|---|
| PR [64492](https://github.com/anomalyco/opencode/pull/64492)，2026-11-17 创建 | open，未合并 | 原始字节重置 chunkTimeout 可能让保活掩盖无进展；提议按语义事件计时 |
| Issue [59565](https://github.com/anomalyco/opencode/issues/59565)，2026-10-27 创建 | open | 报告网关截断后误判完成、无产物/通知及续接挂起 |
| Issue [62319](https://github.com/anomalyco/opencode/issues/62319)，2026-11-06 创建 | open，2026-11-26 更新 | 报告另一个 hosted provider 的 DeepSeek 停滞和静默重试，不是同一个 big-model 后端 |

本地 `/home/renyuan/opencode.git` 的 `0cf99cf5f9`（2026-05-24 快照）中，
`packages/opencode/src/provider/provider.ts` 的 `wrapSSE` 按每次原始 `reader.read()`
计时，字节到达后重启计时，EOF 直接关闭。它不是用户安装版本的证明。
`packages/opencode/src/session/llm/request.ts` 存在 `x-session-affinity` 注入；
这个 header 的存在不能证明上游会话绑定导致故障。

## 本轮实现

`src/provider_adapter.rs` 的基础、已缓冲 Chat SSE 解析新增成功确认：

- 要求非空且一致的 request ID/model、单 choice、非空 finish reason 和 `[DONE]`。
- 必须有非空白正文；reasoning-only 加终止事件不能作为可用答案。
- 缺任一终止标志返回 `ProviderAdapterError::IncompleteStream`，归一化审计仍是
  `response_parse`，错误不携带原始正文。失败时不返回局部事件。
- finish 后拒绝新 choice，DONE 后拒绝数据，不能吞掉后置错误；允许 finish 后、DONE 前的
  usage-only 尾块和保活注释。usage 不算正文进展。

这是**已缓冲、逐行 SSE 子集**的收尾验证，不是完整多行 SSE framing、实时 token 输出或
原生工具调用。finish reason 只是协议终止原因，动作 schema、策略、输出截断和任务完成
仍须由后续门禁判断，不能把协议完成等同任务完成。

`src/http_transport.rs` 的共享 Chat/Responses transport 和独立 Messages transport
仍只支持非流式。已有总 timeout 包含响应体，心跳不会延长总期限。本轮修复响应体超时
误归 `Network`：reqwest blocking Read 可能包装成 IO `Other`，现在保留其内部 timeout
分类，不输出底层错误、URL、header 或 credential。

没有新增重试、fallback 或权限请求。runner 回归证明 malformed HTTP 200 正文记录失败，
不生成成功模型记录或 `loop_completed`；真实心跳超时由 transport 回归直接验证。
两者未组合验收，不宣称完整端到端超时恢复。

## 验证与回滚

`tests/provider_interruption.rs` 七项测试覆盖缺终止帧、reasoning-only、终止顺序、身份变化、
usage 尾块、中文正文、两种 transport 心跳超时、Unicode 审计目录。仅用合成数据和
localhost，无真实模型请求或凭据读取。

- RED：原实现 1 通过/6 失败；GREEN：相关六个套件合计 54 项通过。
- 本轮全量 96 个 harness（含空 doctest）/1358 项通过，0 失败/跳过；严格 all-targets
  Clippy、实际 lib/bin 构建、fmt/diff 检查通过，具体命令输出保存在外置日志。
- Unicode 回归仅证明中文/空格**审计目录**可用，不证明完整工作区工具链兼容或用户故障原因。
- 主代理源码/diff 自审，不是独立模型审核；回滚依据为精确 Git diff/提交，无需另复制源文件。

```bash
cargo test --locked --offline --jobs 1 --test provider_interruption \
  --target-dir /temp/codex/steerbox-validation/foreground-upgrade-20261102/master-target
```

本轮证据：`/temp/codex/steerbox-validation/provider-interruption-20261127/` 下的
`red.log`、`green.log`、`full-test.log`、`clippy.log`、`build.log`。构建产物均在项目外。

## 后续流式门禁（尚未交付）

1. 分开统计总期限、字节空闲期限、模型有效进展期限；注释、ping、usage-only 不延长
   有效进展期限。持续推理也必须受总期限和预算约束，不能永久续命。
2. 区分 receiving、protocol-completed、failed/cancelled；记录字节数、有效事件数、
   最后进展时间和 terminal-seen 等 secret-free 元数据。EOF 不能代替协议完成。
3. 局部输出不能执行未经完整验证的动作；reasoning 不能当最终答案。
4. 重试/换路由必须显式、有界、共享预算并记录父子 attempt。请求可能已经计费；
   工具/外部副作用须先对账，不因没有最终回答就盲目重放。写入结合 snapshot、审批和
   幂等/fencing 合同，不能靠再次问模型解决不确定状态。
5. incremental transport、语义 watchdog、续接和跨后端自动恢复另行实现和验收；
   当前保持 bounded non-streaming，不顺带扩大安全边界，不宣称 Phase 1 完成。
