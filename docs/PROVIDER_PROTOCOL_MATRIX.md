# Provider Protocol Matrix

状态：设计基线（2026-10-16）

## 结论

steerBox 必须支持 OpenAI，但“OpenAI”不是单一 wire protocol：

1. `openai-compatible-chat`：当前已经实现的 `/chat/completions` 风格协议。
2. `openai-responses`：OpenAI 原生 Responses API；不能直接复用 `messages -> text` 解析器。

Anthropic Messages 已作为独立协议接入。协议差异必须由 provider-specific adapter 处理，不能只替换 URL 或 provider 名称。

## 分层支持矩阵

| 协议族 | 典型接口 | 当前状态 | 设计边界 |
|---|---|---|---|
| OpenAI Chat Completions 兼容 | `/v1/chat/completions` | 已有 adapter、fake/localhost 测试、bounded Markdown runner | 统一 `messages`、非流式/基础 SSE；不同厂商的扩展字段不能静默假设兼容 |
| OpenAI Responses | `/v1/responses` | 未实现 | 独立 request/response/item/tool-call/continuation contract；不能降级伪装成 Chat Completions |
| Anthropic Messages | `/v1/messages` | 已有独立 adapter、fake/injected 覆盖 | 独立 system、消息块、认证 header、响应解析和错误分类 |
| Google Gemini 原生 | `generateContent` / `streamGenerateContent` | 未实现 | 独立 `contents/parts`、工具调用、缓存和安全设置；不能仅依赖 OpenAI 兼容层 |
| Google Gemini OpenAI 兼容入口 | OpenAI 风格 endpoint | 未单独验收 | 可复用 OpenAI-compatible adapter，但必须做 endpoint/provider capability 声明和 fixture 验证 |
| AWS Bedrock Converse / InvokeModel | AWS provider APIs | 未实现 | 独立 SigV4/credential boundary、模型族 request schema 和响应映射；不能把云凭据放入 auth_ref 之外 |
| Cohere Chat / Mistral Chat 等 | 厂商原生 chat APIs | 未实现 | 优先通过 OpenAI-compatible 兼容入口；原生 adapter 只有在工具、缓存、流式或错误语义需要时新增 |
| 本地服务 | Ollama、vLLM、SGLang、llama.cpp 等 | 仅可作为 OpenAI-compatible 目标推断 | 必须以实际 endpoint fixture 验证，不因“兼容”字样假定 tool、stream 或 JSON schema 完全一致 |
| 实时/语音协议 | OpenAI Realtime、厂商 WebSocket/RTC | 未实现 | 独立 session、双向流、取消和音频安全边界，留到 Phase 2+ |
| Agent-to-Agent / tool protocol | MCP、A2A 等 | 非模型推理协议 | 作为能力/任务边界接入，不替代 provider adapter、PolicyGate 或审计链 |

## steerBox 的统一抽象

统一层只承诺以下语义：

- `ResolvedPromptPack`：已绑定的 provider、endpoint、model、prompt contract 和消息摘要。
- `ResolvedModelAdapter`：一次 resolved invocation，返回严格的 decision/action envelope 和 secret-free usage/error metadata。
- `ProviderTransport`：OpenAI-compatible HTTP transport 边界。
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

- 保持并完成 `openai-compatible-chat` 的真实 credential/live provider 验收前置条件。
- 保持 Anthropic Messages 独立 adapter；当前 fake/injected bounded Markdown 覆盖不等于 live 验收。
- 为 endpoint protocol 增加显式 capability，禁止用 provider 名称或 URL 猜协议。
- 暂不实现 Responses、Gemini 原生、Bedrock、Realtime 或跨协议 fallback。

### Phase 2

1. `openai-responses` adapter：先做无工具、非流式严格 response item 映射，再做工具调用和续接。
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
