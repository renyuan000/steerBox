# steerBox Harness Principles

## 文档目的

本文件用于提炼三篇官方 harness 相关文章中的高价值结论，并转化为适合 `steerBox` 后续学习与开发的指导原则。

本文件分为三类内容：

- `官方结论`：可直接追溯到原文
- `推断`：基于原文、结合 `steerBox` 当前方向得出的判断
- `行动建议`：面向后续学习和实现的可执行建议

本文件不是最终架构方案。

## 官方来源

1. Anthropic, `Effective harnesses for long-running agents`, published November 26, 2025  
   https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents
2. Anthropic, `Harness design for long-running application development`, published March 24, 2026  
   https://www.anthropic.com/engineering/harness-design-long-running-apps
3. OpenAI, `Harness engineering: leveraging Codex in an agent-first world`, published February 11, 2026  
   https://openai.com/index/harness-engineering/

## 一句话总纲

`官方结论`：长时间 agent 的效果，不主要取决于“模型单轮会不会做”，而更取决于 harness 是否提供了可持续执行的环境、状态、约束、反馈、验证和可追溯性。

## 一、三篇文章共同强调的核心原则

### 1. Humans steer, agents execute

`官方结论`

- OpenAI 明确把工程师角色重心从“亲手写代码”转向“设计环境、表达意图、建立反馈闭环”。
- 这不是说人类退出，而是说人类更多负责 steering、验收、约束和纠偏。

`推断，高置信`

- 对 `steerBox` 来说，人类不应只是最终审批者，还应是约束定义者、验收标准定义者和中途接管者。

### 2. 长任务不能只靠上下文窗口

`官方结论`

- Anthropic 2025 文指出，多轮 session 的核心问题是新 session 没有前一轮记忆。
- 仅靠 compaction 不足以稳定支撑长任务。
- Anthropic 2026 文进一步指出，某些模型会出现 `context anxiety`，越接近上下文极限越容易提前收尾。

`推断，高置信`

- `steerBox` 必须把长期任务状态外化成 repo 或系统中的可读取工件，而不能把连续性建立在“模型应该记得”上。

### 3. Structured handoff 比“让下一个 agent 猜”更可靠

`官方结论`

- Anthropic 2025 文使用了 initializer agent、`init.sh`、进度文件和 git 历史，让后续 agent 能快速接手。
- 后续 session 启动时，要先读进度文件、git log、feature list，再决定下一步。
- Anthropic 2026 文也强调了结构化 handoff。

`推断，高置信`

- `steerBox` 后续必须有明确的 `handoff artifact` 概念，至少覆盖：
  - 当前任务状态
  - 最近完成内容
  - 未完成项
  - 下一步建议
  - 验证状态
  - 风险与阻塞

### 4. 某些工件应故意做成窄而结构化

`官方结论`

- Anthropic 2025 文不只是强调 handoff，还强调把某些持续更新的状态信息做成更结构化、更窄的可编辑工件，以减少 agent 在长任务中误覆盖上下文或误改无关内容的风险。

`推断，高置信`

- `steerBox` 后续对状态、进度、审批结果、验证结果这类高频更新对象，应该优先考虑结构化格式和受约束写入，而不是完全自由文本。

### 5. Incremental progress 比 one-shot 更可靠

`官方结论`

- Anthropic 2025 文明确指出，agent 容易一次做太多，导致中途耗尽上下文并留下半成品。
- 他们改为“一次只做一个 feature”，并要求每轮结束时留下干净现场。
- Anthropic 2026 文则把这种拆解进一步发展成 `sprint` 和 `sprint contract`。

`推断，高置信`

- `steerBox` 不应把长任务建模为“大 prompt 持续运行”，而应显式建模为一系列可验证的增量阶段。

### 6. 不要让 agent 自己同时生成又自己宽松打分

`官方结论`

- Anthropic 2026 文明确指出，自我评估是长任务中的一个硬问题。
- 生成者倾向于对自己的结果宽容。
- 将 generator 与 evaluator 分离，是提升质量的重要杠杆。

`推断，高置信`

- `steerBox` 至少在逻辑上要区分：
  - 任务执行者
  - 结果验证者
- 二者未必必须永远是两个模型实例，但验证职责不能被执行职责吞掉。

### 7. “Done” 需要事先被定义

`官方结论`

- Anthropic 2026 文里的 generator 和 evaluator 会先协商 `sprint contract`，也就是先约定这一轮什么算完成、如何验证。
- 这样可以把高层 spec 桥接到可验证的实现层。

`推断，高置信`

- `steerBox` 后续应把 `task contract` 当作核心对象，而不是把 done 定义留到任务尾声再临时判断。

### 8. 计划应桥接目标与验证，而不是过度冻结低层实现

`官方结论`

- Anthropic 2026 文中的 planner 作用，不是把所有实现细节一次性写死，而是把目标扩展成更清晰、可执行、可验证的任务描述，避免高层意图在后续回合中丢失，同时减少低质量细节约束带来的连锁错误。

`推断，高置信`

- `steerBox` 后续的计划文档更应该强调：
  - 目标
  - 边界
  - 验收条件
  - 风险与约束
- 而不是过早把所有低层技术细节写死。

### 9. End-to-end verification 是硬需求

`官方结论`

- Anthropic 2025 文明确说，agent 如果只做代码修改、单测或 `curl` 检查，仍然可能错误地宣布完成。
- 加入浏览器自动化后，性能显著提升，因为 agent 能发现仅看代码看不出的 bug。
- OpenAI 文则把 UI、logs、metrics、traces 暴露给 agent，使 agent 能真实驱动、验证和回归。

`推断，高置信`

- `steerBox` 后续无论是开发方向还是安全方向，都必须有“证据型完成”机制，而不是接受 agent 的口头完成声明。

### 10. 可运行且隔离的任务环境很重要

`官方结论`

- Anthropic 2025 文依赖 initializer、启动脚本和明确的运行现场，让后续 agent 可以重新进入同一个任务。
- OpenAI 文强调让 agent 直接使用标准开发工具，并让 UI、日志、指标与 traces 进入 agent 可见范围。

`推断，高置信`

- `steerBox` 后续应优先考虑为任务提供可重复进入、可验证、可观察的运行环境。
- 这类环境是否最终落到 worktree、沙盒、容器或其他形式，可以后续再定，但“隔离且可运行”本身应是设计目标。

### 11. AGENTS.md 不该是百科全书

`官方结论`

- OpenAI 明确指出，“一个超大的 `AGENTS.md`”会带来上下文拥挤、重点失焦、文档腐烂和难以机械校验等问题。
- 他们把 `AGENTS.md` 定位为 `table of contents`，真正的系统知识存放在结构化的 `docs/` 中。

`推断，高置信`

- `steerBox` 应坚持：
  - `AGENTS.md` 短而稳定
  - 深层规则、设计、计划、质量文档进入结构化知识库

### 12. Repo knowledge 应是 system of record

`官方结论`

- OpenAI 把计划、设计、技术债、参考文档、质量状态都纳入 repo，并作为 agent 可发现的系统知识。
- 他们强调：agent 看不到的知识，等于不存在。

`推断，高置信`

- `steerBox` 需要把关键知识尽可能收敛到可版本化、可链接、可校验的本地文档与结构化工件中。

### 13. Documentation 不够，必须 mechanical enforcement

`官方结论`

- OpenAI 强调仅有文档不够，需要 lint、CI、结构测试、doc-gardening agent 等机械化手段防止漂移。
- 他们不是微管实现，而是强制边界、不变量和结构规则。

`推断，高置信`

- `steerBox` 的很多原则最终都应该从“文档规则”演进到“系统规则”。

### 14. Agent legibility 是设计目标

`官方结论`

- OpenAI 明确提出，代码库应首先对 agent 可理解。
- 他们倾向于采用更容易被 agent 内化、推理和修改的依赖与结构。

`推断，高置信`

- `steerBox` 后续做核心层设计时，应优先考虑：
  - 结构是否稳定
  - 规则是否清晰
  - 依赖是否透明
  - 上下文是否可发现

### 15. Entropy 会累积，必须有 garbage collection

`官方结论`

- OpenAI 明确说 agent 会复制 repo 中已有模式，包括坏模式。
- 他们用 `golden principles` 和周期性清理任务，持续修复漂移和 `AI slop`。

`推断，高置信`

- `steerBox` 后续必须把“持续清理坏模式”视作系统能力，而不是一次性人工整顿。

## 二、三篇文章给出的关键差异与边界

### 1. Context reset 不是永恒真理

`官方结论`

- Anthropic 2025 文和 2026 文前半段都强调了 context reset 在特定模型上的价值。
- 但 2026 文也明确指出，在后续更强模型上，他们移除了 sprint 结构和 context reset，改为连续 session + compaction。

`推断，高置信`

- `steerBox` 不应把 `context reset` 写成永恒架构原则，而应把它视为模型能力相关的 harness 策略。

### 2. Evaluator 也不是永远都要开

`官方结论`

- Anthropic 2026 文明确说 evaluator 不是固定二选一。
- 当任务超出模型稳定单独完成的边界时，evaluator 很值；当模型本身已经能可靠完成时，evaluator 可能只是额外成本。

`推断，高置信`

- `steerBox` 后续可以把 evaluator 设计成按任务风险、复杂度和模型能力可切换的模块，而不是全局刚性开启。

### 3. 高吞吐下的 merge 哲学不能直接照搬到所有领域

`官方结论`

- OpenAI 在其软件工程实验中采用了更轻的 blocking merge gate，并把很多问题交给后续快速修复。

`推断，高置信`

- 这类策略不能机械迁移到安全运维、自动响应或高影响动作场景。
- 对 `steerBox` 来说，开发方向和安全方向的审批强度很可能需要区别对待。

## 三、对 steerBox 最有价值的设计转译

以下内容属于 `推断`，但和三篇文章方向高度一致。

### 1. steerBox 的核心抽象不应先从“agent persona”开始

更应该优先定义这些对象：

- `Task`
- `TaskContract`
- `TaskState`
- `Policy`
- `Approval`
- `ToolCapability`
- `HandoffArtifact`
- `VerificationResult`
- `AuditEvent`
- `Trace`

原因：

- 这三篇文章共同说明，真正 load-bearing 的不是“有几个 agent”，而是任务如何持续、受控、可验证和可交接。

### 2. 一个项目、共享 Harness 和四个专业方向，是合理的

`推断，高置信`

- 当前确定的 `一个项目 + 共享 Harness + 四个专业方向` 与三篇文章强调的稳定底座、受控执行和领域适配原则一致。本文早期使用过“两条主线”表述，现已由四方向结构替代。
- 共享的应该是 harness 核心：
  - 长时执行
  - steering
  - 工具控制
  - 审批
  - 审计
  - tracing
  - 验证闭环
- 分化的应该是领域能力：
  - 软件开发：代码、测试、构建、review、refactor
  - 持续运维：健康、变更、容量、成本、故障和恢复
  - 安全评估与漏洞研究：授权、攻击面、漏洞、PoC 和修复复测
  - 安全运营：信号、调查、事件、响应、处置和防护验证

### 3. 行为可审计、流程可 tracing，不应是附加功能

`推断，高置信`

- 结合三篇文章，审计与 tracing 更适合作为核心对象，而不是后续补丁。
- 原因不是“为了合规写日志”，而是没有 trace 和 evidence，长任务就无法稳定 handoff、验证和纠偏。

### 4. AGENTS.md 应保持短，知识库应进入文档系统

`行动建议`

- 把 `AGENTS.md` 保持为导航入口
- 把长期有效的系统知识沉淀到结构化文档
- 把计划、决策、验收、风险、质量状态做成 versioned artifact

### 5. 高风险动作必须经过 policy + approval + audit

`推断，高置信`

- 对软件开发方向，这可能主要落在：
  - 写文件
  - 执行命令
  - 修改关键配置
  - 自动合并
- 对安全方向，这可能主要落在：
  - 自动响应
  - 外部动作执行
  - 高影响规则变更
  - 处置动作触发

## 四、对后续学习最有帮助的 10 条实践原则

以下内容是给你后续学习和开发时直接用的。

1. 先学 `task lifecycle`，不要先学“怎么多开几个 agent”
2. 先学 `handoff artifact`，不要假设上下文自己会延续
3. 先学 `verification evidence`，不要让 agent 自己宣布 done
4. 先学 `policy and approval`，不要默认工具全开放
5. 先学 `repo as system of record`，不要把关键知识留在聊天记录里
6. 先学 `mechanical enforcement`，不要停留在“写了规则”
7. 先学 `agent legibility`，不要先堆复杂依赖和隐式结构
8. 先学 `incremental delivery`，不要让 agent 追求 one-shot 完成
9. 先学 `evaluator calibration`，不要默认 QA agent 天然可靠
10. 先学 `garbage collection`，不要等坏模式扩散后再补救

## 五、对后续开发最值得优先落地的能力

如果 `steerBox` 后续开始做第一阶段原型，最值得优先验证的不是“全自动”，而是下面这些能力是否真的能协同工作：

### 第一优先级

- `TaskState`
- `TaskContract`
- `HandoffArtifact`
- `VerificationResult`

### 第二优先级

- `Policy`
- `Approval`
- `ToolCapability`

### 第三优先级

- `AuditEvent`
- `Trace`
- 长时任务恢复与继续执行

`推断，高置信`

- 如果这几类对象没有先站稳，后面无论接入哪个专业方向，都会很快重新跌回“靠 prompt 硬顶”的状态。

## 六、哪些地方不要盲目照搬

1. 不要把 OpenAI 的高吞吐 merge 哲学直接套到安全方向
2. 不要把 Anthropic 的某个具体 harness 结构写成永远正确的标准答案
3. 不要把“多 agent”误解成“天然更强”
4. 不要把“自动化”误解成“不需要人在环”
5. 不要把“可审计”误解成“多打一点日志”

## 七、当前对 steerBox 的落地结论

`推断，高置信`

基于这三篇文章，`steerBox` 当前最稳的方向不是：

- 先定复杂目录
- 先定最终语言组合
- 先铺满所有 agent 场景

而是：

- 先定义通用 harness 核心对象
- 先定义长任务状态与 handoff 机制
- 先定义 policy、approval、audit、trace
- 先定义验证闭环
- 再让四个专业方向分别接入

## 八、当前可以直接采用的 5 条硬规则

1. `AGENTS.md` 只做短入口，不做大而全说明书
2. 每个长期任务都必须留下结构化 handoff 工件
3. 每个高风险动作都必须经过策略约束，并可被审计
4. 每个“完成”都必须有验证证据，而不是 agent 自述
5. 每次运行都应产生可追踪的执行痕迹、决策记录和结果状态

## 九、建议的下一步

如果要把本文继续转成工程行动，建议顺序是：

1. 基于本文更新 `README_CN.md` 和 `README_EN.md`
2. 把 [ARCHITECTURE_DIRECTION.md](./ARCHITECTURE_DIRECTION.md) 与本文建立明确引用关系
3. 从 [ARCHITECTURE_QUESTIONS.md](./ARCHITECTURE_QUESTIONS.md) 中优先讨论：
   - 核心边界
   - 长时运行模型
   - steering / policy / approval
   - audit / trace / verification
4. 第一阶段按已冻结边界实现软件开发方向切片，其他三个方向复用经验证的共享 Harness
