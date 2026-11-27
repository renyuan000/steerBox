# Provider Protocol Matrix

状态：设计基线及受限实现更新（2026-11-27；Chat 已有单一第三方的限定 live 验收，不代表其它协议或 Phase 1 全部完成）

## 结论

steerBox 必须支持 OpenAI，但“OpenAI”不是单一 wire protocol：

1. `openai-compatible-chat`：当前已经实现的 `/chat/completions` 风格协议。
2. `openai-responses`：OpenAI 原生 Responses API；不能直接复用 `messages -> text` 解析器。

Anthropic Messages 已作为独立协议接入。协议差异必须由 provider-specific adapter 处理，不能只替换 URL 或 provider 名称。

## 分层支持矩阵

| 协议族 | 典型接口 | 当前状态 | 设计边界 |
|---|---|---|---|
| OpenAI Chat Completions 兼容 | `/v1/chat/completions` | 已有 adapter、fake/localhost 测试、bounded Markdown runner；单一第三方纯观察及只读工具 live 闭环通过 | usage 扩展仅在 wire 边界忽略，内部统计契约严格；基础 SSE 仅离线解析验收，HTTP token streaming 未验收 |
| OpenAI Responses | `/responses`（也可显式配置 `/v1` 或完整地址） | 已实现受限、同步、非流式 adapter；离线注入与 localhost 测试覆盖 | 独立 input/output/status 解析、`store:false`、固定 `reasoning.effort`；原生工具调用、SSE、续接未实现，不自动降级成 Chat |
| Anthropic Messages | `/v1/messages` | 已有独立 adapter、fake/injected 覆盖；显式协议允许自定义中转 provider 名称 | 独立 system、消息块、认证 header、响应解析和错误分类；live 尚未验收 |
| Google Gemini 原生 | `generateContent` / `streamGenerateContent` | 未实现 | 独立 `contents/parts`、工具调用、缓存和安全设置；不能仅依赖 OpenAI 兼容层 |
| Google Gemini OpenAI 兼容入口 | OpenAI 风格 endpoint | 未单独验收 | 可复用 OpenAI-compatible adapter，但必须做 endpoint/provider capability 声明和 fixture 验证 |
| AWS Bedrock Converse / InvokeModel | AWS provider APIs | 未实现 | 独立 SigV4/credential boundary、模型族 request schema 和响应映射；不能把云凭据放入 auth_ref 之外 |
| Cohere Chat / Mistral Chat 等 | 厂商原生 chat APIs | 未实现 | 优先通过 OpenAI-compatible 兼容入口；原生 adapter 只有在工具、缓存、流式或错误语义需要时新增 |
| 本地服务 | Ollama、vLLM、SGLang、llama.cpp 等 | 仅可作为 OpenAI-compatible 目标推断 | 必须以实际 endpoint fixture 验证，不因“兼容”字样假定 tool、stream 或 JSON schema 完全一致 |
| 实时/语音协议 | OpenAI Realtime、厂商 WebSocket/RTC | 未实现 | 独立 session、双向流、取消和音频安全边界，留到 Phase 2+ |
| Agent-to-Agent / tool protocol | MCP、A2A 等 | 非模型推理协议 | 作为能力/任务边界接入，不替代 provider adapter、PolicyGate 或审计链 |

## 多后端配置规则与本轮接入审核

2026-11-27 恢复复审修复：三协议 EOF 与 HTTP 成功 body 读取中断时保留已收到的
拒绝/工具/过滤证据，不能因后续截断而触发修复请求；transport 新增 `UnsafeResponse`。
非 2xx 不读取错误正文，保留 status；晚到的明确错误不被 deadline 覆盖。
`ResponseTooLarge/InvalidResponse/Redirect/Other` 现在映射不可恢复的 `ResponseParse`，
只有 `Network` 保留 `Transport` class；相应 routed fallback 行为也随之收紧。
没有新增 wire 协议、生产流式或 live 验收；支持范围仍以上表和真实调用证据为准。

2026-11-27 完整性/恢复增量：三种 resolved 协议在 decision 解析前检查收尾；明确拒绝、
工具/过滤/未知状态不可恢复，明确 EOF 与输出上限可作为固定缺口；通用解析错误不重试。
公开 adapter 增加 `GenerationRejected` 安全错误变体，class 保持 response_parse；resolved
层转换成固定 decision 拒绝原因并保留已知 usage 与宿主输入 hash，不携带拒绝正文。
`prepare_recovering` 显式纯观察宿主入口最多两次调用、共享预算，无工具重放；既有入口
不新增隐式重试。合同与验收边界见 `PROVIDER_RESPONSE_RECOVERY.md`，并非协议能力全覆盖。

2026-11-27 中断防护增量：基础 Chat SSE 已缓冲解析要求一致身份、有效正文、finish 与
DONE 双终止确认；缺终止帧、reasoning-only 或终止后继续输出均拒绝，兼容合法 usage 尾块。
Chat/Responses 与 Messages 的 HTTP 响应体超时现在保留 `Timeout` 分类，心跳不延长
已有总期限。本段中断防护切片不包含语义进展 watchdog、自动重试或断流续接；
后续显式纯观察恢复见上段，生产 HTTP 仍是有界非流式。
缺陷证据、Unicode 回归范围及后续门禁见 `PROVIDER_INTERRUPTION_GUARDS.md`。

- 一个 `model_profiles` 可以同时配置 Chat、Responses、Messages；`provider` 是后端标识，
  `endpoint_protocol` 决定 wire 编解码及认证方式。显式 Messages 不再要求后端必须叫
  `anthropic`；工厂和 runner 均保留用户配置的后端标识。不按域名判断是否第三方中转。
- 省略协议字段时，现有 JSON 默认规则仍为：`provider=anthropic` 使用 Messages，
  其它名称使用 Chat。自定义中转必须显式选择 Responses/Messages，不能只改 provider 名称。
- `endpoint_mode=base_url`（默认）在配置地址后追加协议路径：`/chat/completions`、
  `/responses` 或 `/messages`；`full_url` 原样使用完整地址。不会自动补 `/v1`、探测协议
  或试错降级；常规第三方 base URL 无需额外设置 `full_url`。
- 当前运行入口按 `model_id` 选择 profile，因此该 ID 在一个配置内必须唯一。
  JSON 加载与三个公开适配器工厂现在都会拒绝重复 ID，包括直接修改公开结构后的重复项；
  在解析凭据或发送请求前返回 `ConfigError::DuplicateModelProfile`，不再静默取第一个后端。
  当前 `model_id` 同时作为上游模型名；同名模型跨中转的独立 profile ID/wire name 配置尚未接通，
  不可通过给上游模型随意改名来绕过此限制。
- 删除不再适用的公开错误项 `UnsupportedAnthropicProvider`；调用方应处理实际的
  `UnsupportedEndpointProtocol`。协议不匹配、能力不足、非法 effect/auth/endpoint 仍拒绝。
- 混合协议回归按与配置不同的顺序选取三个模型，分别核对实际请求地址、wire 结构、
  模型名、认证引用、调用数、usage 与审计后端身份，并检查认证引用/endpoint 不进入模型审计。
  见 `tests/multi_backend_binding.rs`、`tests/config_anthropic_binding.rs` 和
  `tests/config_provider_binding.rs`。本轮仅离线验证，没有新增真实服务调用。

尚未完成的相关能力：Responses/Anthropic live 验收、HTTP token streaming、统一原生工具
调用、通用 resolved-provider fallback、跨后端自动切换及并行多 LLM 调度。自动相邻档位升档
当前仅有 Chat 的显式宿主库入口，不代表其它协议已具备相同能力。CLI 内置凭据解析仍仅有
显式 opt-in 的环境变量方式；真实 Chat 测试的文件读取辅助程序不是已交付的文件凭据功能。
Chat/Responses 使用 Bearer，Messages 使用 `x-api-key`；自定义认证头/云签名不在本轮支持范围。
真实调用证据见 `LIVE_CHAT_READONLY_VALIDATION_2026_11_25.md`，不将混合配置测试当成并行调度验收。

本轮验证（2026-11-25）：全量离线测试 95 个 harness（含空 doctest）/1351 项通过，
0 失败/跳过；严格 all-targets Clippy、实际 lib/bin 构建、fmt/diff 检查通过。
构建显式使用项目外 `--target-dir /temp/codex/steerbox-validation/foreground-upgrade-20261102/master-target`；
全量测试、Clippy 和构建日志在 `/temp/codex/steerbox-validation/multi-backend-20261125/`。
两类缺陷均实际先失败再修复通过；审核为主代理源码/diff 自审，不是独立模型审核。
本轮回滚依据为精确 Git diff/提交，不另复制受版本控制的源文件。

## steerBox 的统一抽象

统一层只承诺以下语义：

- `ResolvedPromptPack`：已绑定的 provider、endpoint、model、prompt contract 和消息摘要。
- `ResolvedModelAdapter`：一次 resolved invocation，返回严格的 decision/action envelope 和 secret-free usage/error metadata。
- `ProviderTransport`：Chat/Responses 共享的 bearer HTTP transport 边界；wire 编解码由显式协议分别选择。
- 独立 transport/adapter：协议有独立认证、消息结构、工具调用、流式事件或错误分类时使用。
- `ModelCallStore`：只保存 binding、hash、usage、finish reason、错误分类和 lineage，不保存 credential 或原始正文。

统一层不应抹平：

- tool call / function call / content block / item 的结构差异；
- streaming event、断点续接、取消和后台任务语义；
- prompt cache、上下文缓存和 provider-specific system/developer 规则；
- credential header、签名、租户和区域选择；
- provider 错误类别、重试条件和计费单位。

## 实施优先级

### Phase 1

- 保持已通过的单一第三方 Chat 纯观察/只读工具 live 验收；真实写入及其它后端单独验收。
- 保持 Anthropic Messages 独立 adapter；当前 fake/injected bounded Markdown 覆盖不等于 live 验收。
- 保持显式 endpoint protocol 分派；进一步补足逐能力声明和验证，不在运行时用 provider 名称或 URL 猜协议。
- 增补受限 `openai-responses`：无原生工具、非流式、无续接，复用已有 decision/action、只读与 Markdown controller 门禁；固定 effect 可用，自动 effect upgrade 仍仅限 Chat。
- 暂不实现 Gemini 原生、Bedrock、Realtime 或跨协议 fallback。Responses live 验收单独记录，不以离线通过代替。

### Phase 2

1. 扩展已完成的受限 `openai-responses`：另立 SSE、原生工具调用、推理 item/continuation 合同及验收；不得将其视为当前已经支持。
2. Gemini 原生 adapter：独立 contents/parts、工具和安全设置映射。
3. Bedrock adapter：独立 credential/signing boundary 与模型族 schema。
4. Provider capability matrix：声明 `chat`, `responses`, `tool_call`, `stream`, `cache`, `resume`, `vision`, `audio` 等能力；路由只能选择声明并通过 fixture 验证的能力。
5. Realtime/语音协议和 A2A/MCP runtime：另立 Phase 2+ 设计，不塞入当前同步 model-call contract。

## 明确禁止

- 不把 OpenAI Responses API 的 item/tool/status 直接序列化成 `messages`。
- 不把 Anthropic、Gemini、Bedrock 的认证方式伪装成 OpenAI bearer header。
- 不因为一个中转平台声称“OpenAI 兼容”就自动允许工具、流式、缓存或 JSON schema。
- 不把 fake transport、localhost fixture 或 adapter 编译通过当成 live provider 验收。
- 不在 provider adapter 内读取、打印、持久化真实 credential；只传递 opaque `auth_ref`。
