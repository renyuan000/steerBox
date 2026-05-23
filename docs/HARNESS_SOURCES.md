# steerBox Harness Sources

## 文档目的

本文件保留 `steerBox` 当前最重要的三篇官方 harness 参考来源，方便后续学习、回查和继续提炼。

为避免版权问题，这里不复制整篇原文，只保留：

- 原文标题
- 官方链接
- 发布时间
- 与 `steerBox` 相关的核心主题
- 少量可回查的短摘录或关键词

## 来源 1

- 标题：`Effective harnesses for long-running agents`
- 来源：Anthropic Engineering
- 发布时间：`November 26, 2025`
- 链接：https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents

与 `steerBox` 最相关的主题：

- 长任务不能只靠单个上下文窗口
- initializer / coding agent 分工
- 结构化 handoff
- 一次只做一个 feature
- 浏览器驱动的端到端验证

短摘录：

> “a different prompt for the very first context window”

回查关键词：

- initializer agent
- progress.md
- feature list
- one feature at a time
- browser verification

已反映到：

- [HARNESS_PRINCIPLES.md](./HARNESS_PRINCIPLES.md)
- [ARCHITECTURE_DIRECTION.md](./ARCHITECTURE_DIRECTION.md)
- [ARCHITECTURE_QUESTIONS.md](./ARCHITECTURE_QUESTIONS.md)

## 来源 2

- 标题：`Harness design for long-running application development`
- 来源：Anthropic Engineering
- 发布时间：`March 24, 2026`
- 链接：https://www.anthropic.com/engineering/harness-design-long-running-apps

与 `steerBox` 最相关的主题：

- generator / evaluator 分工
- sprint contract
- planner 对高层意图的扩展
- context reset 与 compaction 的边界
- evaluator 是否值得常开

短摘录：

> “generator-evaluator loop maps naturally”

回查关键词：

- planner
- sprint contract
- generator
- evaluator
- context reset
- compaction

已反映到：

- [HARNESS_PRINCIPLES.md](./HARNESS_PRINCIPLES.md)
- [ARCHITECTURE_QUESTIONS.md](./ARCHITECTURE_QUESTIONS.md)

## 来源 3

- 标题：`Harness engineering: leveraging Codex in an agent-first world`
- 来源：OpenAI Engineering
- 发布时间：`February 11, 2026`
- 链接：https://openai.com/index/harness-engineering/

与 `steerBox` 最相关的主题：

- humans steer, agents execute
- repo as system of record
- AGENTS.md 只做目录入口
- strict boundaries and predictable structure
- logs / metrics / traces 进入 agent 可见范围
- mechanical enforcement

短摘录：

> “Humans steer. Agents execute.”

回查关键词：

- AGENTS.md
- docs
- feedback loops
- strict boundaries
- predictable structure
- metrics
- traces
- garbage collection

已反映到：

- [HARNESS_PRINCIPLES.md](./HARNESS_PRINCIPLES.md)
- [ARCHITECTURE_DIRECTION.md](./ARCHITECTURE_DIRECTION.md)
- [ARCHITECTURE_QUESTIONS.md](./ARCHITECTURE_QUESTIONS.md)

## 当前使用建议

后续继续学习这三篇原文时，建议顺序：

1. 先读本文件，明确每篇文章在 `steerBox` 中最有价值的主题
2. 再读 [HARNESS_PRINCIPLES.md](./HARNESS_PRINCIPLES.md)，看提炼后的原则
3. 最后回到官方原文做细读，避免把二次总结误当成原文
