# steerBox Storage and Recovery Discussion Draft

## 文档目的

本文件用于沉淀当前关于 `steerBox` 存储、恢复、checkpoint、supervisor 边界的讨论结果，作为后续架构实现讨论的直接输入。

本文件是讨论稿，不是最终设计稿。

它的目标不是一次性定完所有实现细节，而是先把那些“现在就应该想清楚，否则以后很难补”的核心边界固定下来。

## 当前讨论结论

### 1. 起步不能做成 demo 架子

当前明确方向：

- 第一阶段就应按“未来可用于真实开发”的标准设计
- 可以允许部分能力先占位、后实现
- 但不能把关键边界留空
- 尤其不能把 supervisor、持久化、checkpoint、restore 当成后补功能

换句话说，第一阶段可以不完整，但不能是“只能演示、无法长期演进”的架子。

### 2. 不直接照搬 Codex，而是吸收其优点后做得更完整

当前明确方向：

- `Codex` 的一些做法值得参考
- 但 `steerBox` 不应只是复刻其实现风格
- 目标应是吸收其优点后，在以下方面做得更完整：
  - checkpoint/restore 语义更清楚
  - durability 分级更清楚
  - side-effect boundary 更清楚
  - operator-labeled checkpoint 更明确
  - 高可靠恢复语义从第一阶段就进入设计

## 设计目标

这部分设计首先服务于以下目标：

- 高可靠性
- 高性能
- 低系统资源占用
- 长时运行稳定性
- agent / subagent 可重启
- 重启后任务状态不断
- 可支持高可靠 restore checkpoint

## 当前最重要的架构判断

### 1. 外层必须有一个极简 supervisor

当前已明确：

- 所有 agent / subagent 都运行在一个最外层外框中
- 这个外框应尽量极简
- 它的职责不应扩展成业务逻辑中心

它最核心的职责应是：

- 拉起 agent / subagent
- 管理生命周期
- 检测异常退出
- 检测长时间运行中的失效情况
- 在需要时重启内部 agent
- 尽量避免内部 agent 崩溃导致任务状态丢失

### 2. agent 不是状态拥有者

当前最关键的设计判断是：

- agent 可以是可替换执行体
- 但状态不应属于 agent
- 状态应属于 harness

这意味着：

- agent 可以重启
- subagent 可以替换
- 任务可以恢复
- 但任务的 source of truth 不能放在某个 agent 进程的内存里

### 3. 关键状态不能只存在内存

当前已明确：

- 不能依赖上下文窗口或进程内存保存长期状态
- 关键状态必须持续外化
- 关键状态必须能够在进程重启后恢复

更准确地说：

- 不是要求“所有字节都绝对不能丢”
- 而是要求 `source of truth` 不能只在内存里
- 能从 durable source 重建的派生状态，可以允许重算

## 存储分层建议

### 推荐结构

当前推荐采用三层持久化结构：

1. `state.sqlite`
2. `events.sqlite`
3. `sessions/*.jsonl` 或 `traces/*.jsonl`

这是当前最推荐的第一阶段方向。

### 1. `state.sqlite`

主要负责保存：

- 当前任务状态
- 当前步骤
- 任务快照
- approval state
- 恢复元数据
- ownership / lease
- 当前有效 checkpoint 元数据

设计目标：

- 快速恢复
- 快速查询
- 可供 UI / control plane 读取当前状态

### 2. `events.sqlite`

主要负责保存：

- append-only 事件流
- 审计事件
- 工具调用结果
- 决策记录
- 用户介入记录
- 幂等键
- side-effect ledger

设计目标：

- 提供真实来源
- 支持审计
- 支持回放
- 支持恢复补偿逻辑

### 3. `sessions/*.jsonl` 或 `traces/*.jsonl`

主要负责保存：

- 长对话过程
- session transcript
- 详细 trace
- 调试事件
- 高频附加观测记录

设计目标：

- 轻量 append
- 便于 grep / tail / 手工排查
- 避免让高频 trace 污染核心状态库

## 为什么推荐这个结构

### 1. 比“单一大库”更稳

如果把所有状态、事件、trace、transcript 全部塞进一个库：

- 热点过多
- 读写路径容易互相干扰
- 高优先级状态提交会受高频 trace 影响
- 维护难度更高

### 2. 比“全部用多份数据库”更轻

如果三份都做成重数据库：

- 调试成本更高
- 高频日志的写入和清理更重
- 资源占用更高

使用 `jsonl` 承载 transcript / trace 的好处是：

- append 非常轻
- 直观
- 容易 grep / tail
- 出问题时人工排查成本低

### 3. 更接近当前观察到的 Codex 风格

这是一个重要参考，但不是唯一依据。

## 本机运行态观察

以下内容是当前在这台 `WSL2 + Ubuntu` 环境中，对本机运行态做的直接观察，不代表公开官方架构说明。

### 1. Codex 本机运行态观察

本机观察到：

- `codex` 进程中存在 `sqlx-sqlite-wor...` 线程
- 打开了：
  - `/root/.codex/state_5.sqlite`
  - `/root/.codex/state_5.sqlite-wal`
  - `/root/.codex/logs_2.sqlite`
  - `/root/.codex/logs_2.sqlite-wal`
- 同时存在会话级 `jsonl` 文件持续写入

这说明在本机运行态上，`Codex` 至少体现出以下倾向：

- `SQLite + WAL`
- `state` 与 `logs` 分离
- 会话级流式记录独立存在

这对 `steerBox` 有直接参考价值。

### 2. OpenCode 本机运行态观察

本机观察到：

- `opencode -c` 在空闲时仍然占用较高内存和 CPU
- 主进程 RSS 大约在 `560MB` 级别
- 常驻拉起多个 MCP 侧车：
  - filesystem
  - memory
  - sequential-thinking
  - gdb
  - semgrep
- 也使用了本地 SQLite WAL：
  - `/root/.local/share/opencode/opencode.db`
  - `/root/.local/share/opencode/opencode.db-wal`
  - `/root/.local/share/opencode/opencode.db-shm`

当前基于观察的结论是：

- 它更像是常驻了过多长期运行的能力
- 资源浪费更像架构偏重
- 当前没有足够证据说明它在做恶意外联或异常后台动作

对 `steerBox` 的启发是：

- 不应为了“能力齐全”而默认常驻过多侧车
- 空闲时的资源占用应是第一阶段就要约束的目标

## 写入路径建议

### 核心原则

不要让每个 agent 自己随意开线程直接写库。

更稳的方式应是：

- `agent` 产生命令、状态变化、事件、结果
- 统一交给 harness 内部的写入通道
- 再由专门的存储写入组件落盘

### 推荐分工

建议至少拆成：

- `state writer`
- `event writer`
- `trace/session writer`

职责分别是：

- `state writer`
  - 负责高优先级状态提交
- `event writer`
  - 负责 append-only 审计与事件记录
- `trace/session writer`
  - 负责高频附加日志与 trace

### 同步与异步边界

这里必须区分：

- 哪些写入必须先 durable 再返回成功
- 哪些写入可以异步微批量

当前建议：

#### 应优先 durable 的内容

- 当前任务状态变更
- 已完成步骤
- 待执行步骤
- 审批结果
- 用户介入记录
- 外部动作结果
- 幂等键
- checkpoint 元数据

#### 可异步处理的内容

- 详细 trace
- 调试日志
- 指标统计
- transcript 附加视图
- 检索索引

### 一个关键反模式

当前不建议以 `1s / 3s / 5s` 定时统一刷盘作为第一真源。

原因：

- 两次刷盘之间依然可能丢关键状态
- 定时 flush 更适合快照、压缩、清理、聚合
- 不适合承担关键状态的第一持久化职责

因此当前建议是：

- `关键事件驱动持久化`
- `快照 / 压缩 / 投影使用定时或微批量`

## checkpoint / restore 设计方向

### 当前明确方向

`steerBox` 应支持 restore checkpoint，并且要支持高可靠性的 restore checkpoint。

这不应是后续附加功能，而应在第一阶段进入架构设计。

### 为什么终端类 agent 很少支持高可靠 restore checkpoint

当前讨论判断：

- 代码编辑类 restore 相对容易
- 终端类 agent 的恢复难度高很多

因为终端 agent 涉及：

- 文件系统
- 子进程
- shell 命令
- 外部 API
- 网络动作
- side effects
- 审批状态
- 用户介入记录

因此，终端类 checkpoint 的核心难点不是“保存一个快照”，而是：

- checkpoint 的范围到底是什么
- restore 的保证到底是什么
- side effects 是否可撤销
- 哪些结果只能补偿，不能回滚

### 推荐分级

建议从设计上先区分两类 checkpoint：

#### 1. `Soft Checkpoint`

保存：

- workspace / code state
- harness task state
- event offsets
- approvals
- user intervention records

特点：

- 高频生成
- 恢复快
- 不承诺撤销所有外部副作用

#### 2. `Hard Checkpoint`

要求：

- state / event / session 已 durable flush
- child agents 已进入可恢复边界
- 当前 step 已形成明确 restore token
- side-effect ledger 已落盘
- 必要时允许人工标记

特点：

- 可靠性要求更高
- 创建频率更低
- 适合“可标记、可回查、可审计”的恢复点

### 推荐元数据

即使第一阶段不全部实现，也应先在设计中占位：

- `checkpoint_id`
- `checkpoint_type`
- `checkpoint_label`
- `checkpoint_scope`
- `durability_level`
- `restore_preconditions`
- `restore_guarantees`
- `side_effect_boundary`
- `compensation_required`
- `operator_marked`

## restore 边界建议

当前建议把 restore 能力分成三层讨论：

### 设计目标边界

设计上应考虑到最高边界：

1. 代码 / 工作区
2. harness 任务状态
3. child agent 进程树
4. 外部副作用账本

也就是说，设计上应朝如下边界考虑：

- `代码/工作区 + harness任务状态 + child agent进程树 + 外部副作用账本`

### 第一阶段落地边界

第一阶段实现不一定一步到位。

当前建议：

- 设计边界按高等级考虑
- 第一落地实现按较小边界实现

当前最稳建议是：

- 设计目标：覆盖到外部副作用账本
- 第一实现：先确保
  - 代码 / 工作区
  - harness 任务状态

换句话说：

- `高等级恢复语义现在就设计进去`
- `第一版恢复能力可以先收敛到较小实现面`

## durability 与资源目标

当前最推荐的平衡目标是：

- `state/events` 优先可靠
- `trace/session` 优先轻量

这类分级的意义在于：

- 不让高频 trace 拖慢关键状态提交
- 不让所有东西都进入“重事务路径”
- 保持空闲时资源开销可控

## 当前建议的实现原则

### 应做的事

- 把 supervisor 保持极简
- 把状态拥有权从 agent 拿出来
- 使用 `SQLite WAL`
- 分离 `state` 与 `events`
- 让 transcript / trace 走轻量 append 路径
- 从第一阶段就设计 checkpoint / restore 元数据
- 从第一阶段就设计 side-effect ledger

### 不应做的事

- 不要让关键状态只存在 agent 内存里
- 不要把所有写入都塞进一个混杂的重数据库
- 不要让高频 trace 挤占关键状态写入通道
- 不要默认常驻太多高资源侧车
- 不要把 restore 简化成“代码回退”后就称作高可靠 checkpoint

## 当前建议的明日推进顺序

明天如果继续更新架构实现，建议顺序是：

1. 先确定 `state / events / sessions|traces` 三层结构是否接受
2. 再定义 supervisor 与 writer 的职责边界
3. 再定义 checkpoint 元数据模型
4. 再定义 restore guarantee 分级
5. 最后再决定第一阶段最小实现范围

## 当前文档定位

本文件是讨论稿。

它的用途是：

- 保留今天已经想清楚的关键边界
- 防止后续架构实现退化成 demo 化方案
- 为明天继续进入架构实现讨论提供直接输入
