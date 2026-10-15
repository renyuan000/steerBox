# Tool Capability Display Contract

状态：设计基线（2026-10-16）

## 目标

工具配置和运行状态必须一眼可读，避免用户把“已配置”“默认启用”“当前已启动”“本次已调用”混为一谈。展示应接近 Claude 的工具状态卡片：短表格、低噪声、明确资源成本和使用场景。

## 推荐输出

```text
工具能力

┌────────────────────┬──────────┬──────────────┬────────────────────┐
│ 工具/能力           │ 默认状态 │ 运行资源     │ 使用场景           │
├────────────────────┼──────────┼──────────────┼────────────────────┤
│ read_file           │ ✅ 启用  │ 极低/按需     │ 读取受限工作区文件 │
│ git_status          │ ✅ 启用  │ 极低/按需     │ 查看仓库状态       │
│ edit_markdown       │ 🟡 受限  │ 极低/按需     │ 审批后修改 Markdown│
│ bash                │ ❌ 禁用  │ 高/按需       │ 尚未纳入 Phase 1    │
└────────────────────┴──────────┴──────────────┴────────────────────┘

运行说明

- ✅ 已配置/默认可用，不代表进程已经启动。
- ⏸️ 已配置但当前未启动；按需调用时才拉起。
- 🟡 受策略、路径、审批或预算约束。
- ❌ 禁用或尚未实现。

资源说明

- 不调用时：按需能力不应持有进程或连接。
- 调用时：显示预期内存、启动延迟和是否需要外部服务。
- 运行后：显示本次调用耗时、结果、验证状态和副作用摘要。
```

## 四种状态必须分开

| 状态 | 含义 | 是否授予执行权 |
|---|---|---|
| configured | 配置中存在 manifest/profile | 否 |
| enabled | 允许路由选择该能力 | 否；仍需 PolicyGate |
| running | 当前进程或子进程已启动 | 否；仍需每次调用授权 |
| invoked | 本次运行已经调用 | 仅表示事实，不代表成功 |

MCP server、浏览器、数据库连接和本地模型都应遵守这四态模型。特别是 `enabled` 不等于常驻启动：按需工具默认不启动，避免无意义的内存和连接占用。

## 与安全边界的关系

- 展示信息只能来自可信 manifest、运行时计数器和审计事件，不能让模型自报资源或权限。
- `resource usage` 是运维提示，不是安全批准条件；PolicyGate 仍以 side-effect、risk、path scope、credential scope 和 human steering mode 决策。
- `use case` 是说明文字，不是 capability grant。
- 外部 MCP/tool server 必须先有 provider/capability manifest，再进入 ToolRegistry；不能因为配置文件中出现 server 就自动获得工具权限。
- 资源消耗未知时显示 `未知/待测`，禁止编造具体 MB 数值。

## steerBox 分阶段落地

### 当前 Phase 1

- 为现有 ToolRegistry 暴露只读 capability manifest：tool id、版本、side-effect、risk、allowed modes、recipe、状态。
- CLI 和 StreamReporter 使用短表格/单行状态输出。
- 只显示已验证的资源事实；没有实测就显示 `未测`。
- 不引入 MCP 常驻进程，不把 `enabled` 解释为自动批准。

### Phase 2

- MCP server lifecycle：configured/enabled/running/invoked/failed。
- 按需启动和闲置回收，记录启动延迟、峰值内存、连接和错误分类。
- 工具 manifest 版本、能力声明和 PolicyGate 绑定。
- TUI/Web 使用同一份结构化状态，不另造一套权限语义。

## 对用户的响应风格

配置完成后先给摘要表，再给资源/运行说明，最后给限制和下一步。不要把几十行启动日志当作主要 UI；详细日志进入 EventLog 或 `execution show`，交互面只保留用户需要决策的信息。
