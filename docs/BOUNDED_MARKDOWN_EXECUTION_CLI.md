# 受限 Markdown 执行 CLI 合同

日期：2026-11-23；基线 `45d4e73`。本片目标是接线已有单动作 Markdown runner，
不是新增通用编码循环。用户批准本片实施不等于批准 live 请求或真实 workspace 写入验收。
本文记录实现合同；离线切片的定向75项、全量1284项及独立规格/质量审核已通过，
严格Clippy、实际lib/bin build、fmt/diff通过；提交及主树复验以本片外置收据为准。

## 命令与授权

```text
steerbox loop provider-markdown <config.json> <project_id> <goal_id>
  <loop_run_id> <task_run_id> <model_id> <objective> <observation>
  --workspace <absolute-path> --path <relative.md>...
  --authorize-markdown once [--credential-source env]
```

所有选项均为 flag/value 对；`--path` 可重复指定 1..256 个不同的现存普通 Markdown
相对路径。拒绝未知选项、重复单值选项、重复路径及缺失值；不接受 `--approved`、
`--max-iterations`、任意 shell 或内联凭据参数。完整解析参数应先于配置读取。

`--authorize-markdown once` 为必需选项，只接受值 `once`。它表示可信调用者明确授权
本次命令在所列 workspace/path scope 内执行至多一次模型提出的 Markdown 编辑及
同文件验证。CLI 以 `HumanSteerableLoop`、`approved=true` 复用既有 runner；
不能由模型输出、配置中的任意文本或预检成功替代该显式宿主授权。

这里的 `once` 不是 `ApprovalStore` 的 `ApprovalScope::Once` 记录，不创建持久化审批，
没有可恢复的消费账本、跨进程去重或重新执行许可。失败不会自动退款、回滚或重试；
不确定现场必须人工核对，不能给同一任务换新 ID 后盲目重跑。

## 配置、凭据与运行前检查

- 默认 credential resolver 拒绝解析；仅显式 `--credential-source env` 才在既有 HTTPS
  transport 边界解析 profile 的 opaque `env:NAME` 引用。默认拒绝凭据解析不是禁用
  网络开关：未配置 `auth_ref` 的 OpenAI-compatible profile 按既有匿名 HTTPS 请求
  合同执行，不会调用 resolver；`--authorize-markdown once` 的本次启动授权涵盖模型
  请求。Anthropic 必须提供凭据引用。当前双协议默认凭据拒绝进程测试的 fixture 均
  配置 `env:` 引用，故在凭据解析处拒绝而不发送请求，不能推广为任意默认配置均不
  联网。真实值不得写入配置、参数、objective/observation、进度或审计；本片不新增
  凭据存储或扫描。
- 复用 `prepare_markdown`、`validate_runtime_paths` 和双协议 runner，支持既有
  OpenAI-compatible/Anthropic 分派，不降低 HTTPS、重定向或重试限制。
- workspace 必须绝对；授权目标沿用 scope、普通文件、single-link 和 no-symlink
  门禁。runtime `.steerbox` 及必要父目录须已存在，不为运行自动初始化。
- 打开 stores 前检查 events、model 成功/失败及两锁、tool 及其锁、checkpoint、trace、
  side-effect 和 state 共十一条目的地。canonical runtime 与 workspace 不相等，
  双向不嵌套；可使用 `data_root/workspace` 与 `data_root/.steerbox` 兄弟布局。
- 实际运行必须重查门禁，不能把另一次 `preflight-markdown` 的通过结果当执行许可。
  library 的共址兼容不变；双向目录隔离是 CLI 的额外要求，不是原子 OS sandbox。

执行沿用 runner 的历史身份 guard：检查 EventLog 和成功/失败模型记录，拒绝重复
loop/task/next-call 身份及不可用历史；不只检查已完成记录。该阶段会读历史，
store 打开或锁查询可能产生运行文件，故实际 run 不承诺零磁盘写入。
预检中的 `history=not_checked` 不适用于实际执行；历史 guard 也不是恢复入口。
完整身份规则及并发限制见 `PROVIDER_RUN_IDENTITY_GUARD_CONTRACT.md`。

## 动作、验证与结果

本命令至多调用一次模型，接受至多一次授权范围内的 `edit_markdown`，并要求同文件
`markdown_link_check`。成功必须到达 `CompleteLoop`/`Completed`；`RequestHuman`、
无动作或其它未完成结果不算成功，返回执行失败分类，不输出成功摘要。
不能把模型自报完成当成产物验证。
沿用 PolicyGate、checkpoint、trace、side-effect 和模型/工具审计，不能绕过预算或授权。

controller 的 expected/replacement 与 checkpoint hash 上限为 2 MiB；Markdown tool
verifier 完整读取至多 2 MiB 并严格拒绝无效 UTF-8，不验证 16 KiB 展示前缀后报成功。
`read_file` 的 16 KiB 展示截断和独立 snapshot 的 1 MiB 限额不变；本片不自动接入
snapshot、restore 或 rollback。限制及 hash 域见 `BOUNDED_MARKDOWN_RUNTIME_GATES.md`。

运行结束后，CLI 在写 StateStore 派生投影前重新检查目的地和 runtime/workspace 隔离；
投影由实际事件重建，不是持久 TaskRun/Attempt、预算或授权恢复。
若执行与投影都失败，返回固定 `EXECUTION_FAILED` 分类，不保留动态执行错误正文；
执行成功而投影失败返回固定 `PROJECTION_FAILED` 分类，不能输出成功摘要。
编辑后验证、审计或投影失败不证明 workspace 未变；需检查现场，不自动重做。

错误输出按参数、配置、scope/runtime、执行或投影阶段固定分类，不回显底层错误正文、
凭据、endpoint、auth_ref 或调用者输入。执行/投影错误的静态正文分别为：

```text
provider-markdown execution failed; audit or workspace may have changed; no automatic rollback
provider-markdown state projection failed; audit or workspace may have changed; no automatic rollback
```

`MetadataProgress` 将 subject ID 替换为固定 `bounded-markdown`，移除原始 subject ID、
summary 和 progress 计数，只保留封闭 subject/status 枚举，经 reporter 输出为
`<subject> bounded-markdown <status>`。这是 lifecycle 反馈，不是 token streaming，
阶段 completed 也不等于整个执行已成功。

执行与投影均成功时 exit0，生命周期输出之后的结果摘要固定为一行：

```text
decision=CompleteLoop loop_status=Completed scope=markdown
```

失败 exit1，可能已经输出 lifecycle 行，但不输出成功摘要；main 在固定分类错误之后
附静态 usage。精确输出及退出码的实测状态由本片结果登记，不将本段源码合同当作测试通过。

## 验证与未交付边界

本片验收须使用合成配置、注入式响应及临时授权 workspace，核对参数/授权先拒绝、
双协议、十一目的地与双向隔离、历史身份、默认凭据拒绝、真实编辑/同文件验证、
执行错误优先和投影失败不报成功。既有纯预检的零写入合同仍独立成立。
定向75项、全量1284项、严格Clippy/build/fmt/diff及独立规格/质量审核通过；
完整命令与提交/合入复验见`/temp/codex/steerbox-validation/markdown-execution-cli-20261123/RECEIPT.md`。
未执行真实配置/凭据或live LLM验收。进程测试只覆盖其实际
执行的参数、路径/隔离和默认凭据拒绝；双协议成功编辑、历史 guard、非完成结果、
投影单独/双重失败及进度字段清除另由注入式 CLI 单元测试覆盖，不能称作进程成功或 live 证据。

不交付任意文件/Bash、多步写入、自动重试、fallback、自动升档、后台 worker、
调用期间 status/cancel 或完整恢复。`execution` 历史查询、inbox Resume 和状态投影
不等于跨进程续跑；身份 guard 不保证原子认领或 exactly-once。
本片实际代码链已独立复审；真实环境的写链审查及获准live E2E仍是放行门禁，
本片不代表完整Phase 1完成。
