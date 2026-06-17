# steerBox Tracks and Supporting Capabilities

## 文档目的

本文件用于说明 `steerBox` 中“主线”和“支撑能力”的关系，避免后续文档和实现把业务方向、工程机制、研究方向混在一起。

本文件是架构讨论稿，不是最终代码目录设计，也不代表当前仓库已经具备对应能力。

## 总体组织原则

`steerBox` 保持一个项目，但内部应分清三类内容：

1. 主线场景
2. 通用 harness 核心
3. 横向支撑能力

推荐理解方式：

```text
steerBox
  -> 主线场景：软件开发智能体、安全智能体
  -> 通用核心：长时运行、状态、权限、审计、tracing、checkpoint、人类可掌舵运行模式
  -> 横向能力：goal、loop、efficiency、multi-model routing、memory、tool/skill/MCP、code logic index、auto-evolution
```

## 主线场景

主线场景回答的是：`steerBox` 要训练和支撑哪类真实智能体。

当前保留两条主线。

### 1. 软件开发智能体

目标是构建能够长期推进真实软件开发任务的智能体。

关注点包括：

- 长时间自动化开发
- 代码阅读、修改和组织
- 构建、测试、lint、review、refactor
- 开发工作流控制
- 失败后恢复与继续
- 代码改动的影响面分析
- 最佳代码实践沉淀
- 高质量、低跑偏、可验证交付

### 2. 安全智能体

目标是构建能够长期运行、持续分析和响应安全信号的智能体。

关注点包括：

- 长时间自动化安全运维
- 长时间自动化安全日志分析
- 安全事件发现与响应
- 自动化攻防分析
- 漏洞发掘与分析
- 安全开发与安全审查
- 高风险动作审批、审计、追踪和回滚

## 通用 Harness 核心

通用 harness 核心回答的是：不同主线共享什么运行底座。

这部分能力应尽量不绑定具体领域。

核心能力包括：

- agent / subagent 生命周期管理
- minimal supervisor
- 任务状态管理
- 统一事件日志
- checkpoint / restore
- 工具调用控制
- capability 与权限边界
- policy gate
- human-steerable operation modes
- tracing 与审计
- durable memory
- evaluator 与验证闭环
- side-effect ledger

设计原则：

- agent 是可替换执行体，状态属于 harness
- 关键状态不能只存在上下文窗口或进程内存里
- 所有高影响动作必须有 trace、审计和可恢复边界
- 开发方向和安全方向共享 harness，但不共享领域策略

## 横向支撑能力

横向支撑能力回答的是：用什么工程能力让主线智能体越来越稳、越来越快、越来越省、越来越可靠。

这些能力不是第三条业务主线，而是支撑两条主线的工程层。

### 1. Goal Engineering

把自然语言目标转成可执行、可验证、可约束的 `GoalContract`。

它应定义：

- 做什么
- 不做什么
- 成功标准
- 验证方法
- 允许工具
- 禁止工具
- 风险等级
- 停止条件
- 人类介入点
- checkpoint 策略

这部分单独沉淀在：

- [Goal and Loop Engineering Discussion](./GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md)

### 2. Loop Engineering

把智能体执行过程设计成可观测、可调度、可中止、可恢复的循环。

一个基本循环是：

```text
observe -> plan -> act -> record -> evaluate -> update state/memory -> checkpoint -> continue/stop
```

但真实系统还需要：

- policy gate
- tool gate
- budget gate
- risk gate
- verification gate
- recovery gate

这部分单独沉淀在：

- [Goal and Loop Engineering Discussion](./GOAL_AND_LOOP_ENGINEERING_DISCUSSION.md)

### 3. AI Agent Efficiency Engineering

效率工程不是单纯省 token，而是在最短时间内、以最快速度、最高质量、最佳代码实践、最小 token 花费、最小系统资源占用，达成运行目标。

其中多模型路由是重要组成部分：不同模型应根据费用、特长、可访问性、工作效率、问题难度和架构需求分配不同任务，以发挥各自优势并降低总体成本。

这部分单独沉淀在：

- [AI Agent Efficiency Engineering](./AI_AGENT_EFFICIENCY_ENGINEERING.md)

### 4. Agent Auto-Evolution Engineering

目标不是让 agent 无约束地自我修改，而是在可控、可审计、可回滚的机制下持续改进 harness。

可进化对象包括：

- prompt fragments
- project instructions
- skills
- tools
- MCP registry
- memory policy
- retrieval policy
- evaluator rules
- loop policy
- checkpoint policy
- code navigation indexer

每次进化都应有证据、预测、验证和回滚。

### 5. Skill / Tool / MCP Auto-Evolution Engineering

工具、skill、MCP 是智能体真实能力的主要来源之一。

它们应被作为可观测组件管理：

- capability
- 输入输出 schema
- 权限范围
- side effect 类型
- 成功率
- 平均耗时
- token 成本
- 失败样例
- 替代工具
- 回滚版本

原则：

- 自动新增工具默认禁用
- 自动提升权限必须人工审批
- MCP server 不应被视为天然可信边界
- tool poisoning 和 prompt injection 必须作为一等威胁建模

### 6. Dream Learning Engineering

“梦境学习工程”建议定义为：在非前台执行时间，对历史经验进行离线整理、压缩、去重、冲突检测、索引和策略候选生成。

它不是直接训练模型，也不是自动放权。

关键分层：

- raw experience
- episode summary
- pattern memory
- failure memory
- tool memory
- codebase memory
- policy candidates
- accepted policy

可用技术包括：

- SQLite / JSONL
- 向量检索
- 关键词检索
- 图谱关系
- hierarchical summaries
- recency / importance / confidence scoring
- contradiction detection
- forgetting / decay policy

### 7. Code Logic Organization Engineering

这是软件开发智能体的核心支撑能力。

目标是解决普通 IDE、LSP、grep 不容易完整表达的复杂逻辑关系，例如：

- 宏
- 代码生成
- 反射
- 动态注册
- 配置驱动逻辑
- 回调链
- 跨语言 FFI
- build tags / 编译条件
- 协议字段映射
- 测试与代码关系
- 日志错误与代码路径关系

建议后续建设 `CodeLogicIndex`，帮助 agent 更快定位代码、更准确理解影响面、更少误改。

## 文档拆分原则

后续建议按下面方式组织文档：

```text
README.md
  -> 项目入口，只写定位和链接

README_CN.md / README_EN.md
  -> 双语入口

docs/ARCHITECTURE_DIRECTION.md
  -> 项目总方向，两条主线，边界原则

docs/TRACKS_AND_CAPABILITIES.md
  -> 主线与横向能力关系

docs/AI_AGENT_EFFICIENCY_ENGINEERING.md
  -> 效率工程定义、目标和指标

docs/STORAGE_AND_RECOVERY_DISCUSSION.md
  -> supervisor、持久化、checkpoint、restore

docs/HARNESS_PRINCIPLES.md
  -> 外部 harness 文档精华提炼
```

## 当前结论

`steerBox` 不应把所有能力揉成一篇大文档，也不应把所有横向能力都升级成业务主线。

更稳的结构是：

- 两条业务主线保持清晰：软件开发智能体、安全智能体
- 通用 harness 核心作为共享底座
- goal、loop、efficiency、memory、tool/skill/MCP、dream learning、code logic index、auto-evolution 作为横向支撑能力

这样后续进入代码实现时，才更容易区分哪些能力属于 core，哪些属于 dev track，哪些属于 security track，哪些只是可插拔支撑模块。
