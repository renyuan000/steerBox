# 受限只读 provider CLI

2026-11-08。接线目标是把已实现的 bounded provider loop 暴露为可运行命令，
不是重写循环，也不是完整通用编码 Agent。实现及验收状态以本轮验证记录为准。

## 命令与准备

```text
steerbox loop provider-readonly <config.json> <project_id> <goal_id>
  <loop_run_id> <task_run_id> <model_id> <objective> <observation>
  --workspace <absolute-path> --max-iterations <1..16>
  --path <relative.md>... [--credential-source env]
```

先用 `runtime init <project_ref>` 创建项目配置和 runtime 父目录，再在配置中添加
目标模型 profile，并用 `config validate <config.json>` 校验。初始化默认不含模型。
配置的 `data_root` 仍为 `/temp/steerbox/<project_ref>`；新命令不会为了通过预检而
自动创建缺失的 runtime 父目录。不要把 runtime 目录放进授权工作区。

例如，配置已完成且模型 ID 为 `review-model` 时：

```bash
steerbox loop provider-readonly /temp/steerbox/demo-project/config.json \
  demo-project goal-review review-loop-1 review-task-1 review-model \
  'Check authorized documentation links and Git metadata' \
  'Inspect README.md without editing files' \
  --workspace /absolute/path/to/repository --max-iterations 4 \
  --path README.md --credential-source env
```

这是使用格式示例，不是本项目已完成的真实 LLM 验收记录。显式启用 `env` 时，
profile 的 `auth_ref` 必须采用已有 resolver 支持的 `env:NAME` 引用形式；真实值
不写进命令参数、objective、observation 或模型上下文。省略凭据来源时默认拒绝
凭据解析，不隐式扫描环境或磁盘。生产 HTTP transport 只允许 HTTPS，无测试绕过。
当前命令不提供非环境变量凭据存储的新实现。

同一日志集合中的 `loop_run_id` / `task_run_id` 不可当成恢复句柄重复执行；
已使用身份会在模型发送前被拒绝。新任务使用新身份，不静默重跑旧任务。

## 离线只读预检

把上述命令的 `provider-readonly` 换成 `preflight-readonly`，其余参数完全相同：

```bash
steerbox loop preflight-readonly /temp/steerbox/demo-project/config.json \
  demo-project goal-review review-loop-1 review-task-1 review-model \
  'Check authorized documentation links and Git metadata' \
  'Inspect README.md without editing files' \
  --workspace /absolute/path/to/repository --max-iterations 4 \
  --path README.md
```

通过时退出码为 0，stdout 固定为一行：

```text
preflight=passed scope=readonly credentials=not_checked transport=not_checked history=not_checked
```

参数错误或本地门禁不通过时退出非零，不输出通过摘要。该命令读取配置、文件系统
元数据和时钟，复用运行入口的模型/profile、workspace、授权路径、迭代及八条
store/lock/state 目的地检查；不打开 stores、不写文件、不预留运行身份，不执行
Git、工具或模型请求。同一身份可重复预检，不代表该身份可以重复运行。

| 已检查 | 尚未检查 |
|---|---|
| 模型绑定、文本能力、认证引用元数据 | 引用对应的真实凭据是否存在或有效 |
| workspace 和授权 Markdown 路径的元数据门禁 | 文件链接语义、Git 状态及实际工具结果 |
| runtime 目的地的父目录、类型、别名与工作区隔离 | 实际可写性、日志内容、历史身份冲突和恢复状态 |
| 本地配置与参数有效性 | HTTP client、TLS、网络可达性及真实 provider 接受情况 |

即使传入 `--credential-source env`，预检也不会解析环境变量或联网。
损坏的现存日志可以通过，因为 `history=not_checked`；运行命令仍会自行执行
历史门禁。预检只是当时的本地快照，不是执行许可，也不是竞态隔离保证；正式
`provider-readonly` 会重新检查自己的门禁。它不替代 live 验收。

## 授权与停止语义

- 仅允许 `git_status` 和显式授权 Markdown 文件的 `markdown_link_check`，包括
  verifier；没有通用文件读取、写入或 Bash。路径列表为 1..256 个现存相对文件。
- Git 可能发现祖先仓库；其状态元数据不受工作区子树或 Markdown 清单隔离。
  可信调用者应传入预期仓库根目录。此授权不包括任意文件正文。
- workspace、路径及运行目录均预检；七条 store/lock 路径加 `state.json` 必须
  互不别名、位于工作区外，不接受目录、软链接和 Unix 硬链接冲突。
  全部检查先于打开 stores，状态投影写入前再次检查。
- 迭代上限、模型/工具次数和已配置 reported-token 预算由同一 TaskRun 共享；
  不支持自动重试、fallback、升档或重置预算。
- 摘要区分 decision、stop_reason、iterations 和实际 loop_status。human/no-action
  是同步调用已返回的停止结果；即使继承状态为 Running，也不表示存在后台 worker。
- 执行失败后尝试保存已有事件的状态投影；投影也失败时保留原执行错误。预检
  不通过不打开 stores、不写投影。投影不是完整 TaskRun/Attempt 跨进程恢复。

当前路径身份保护依赖 Unix；不宣称 Windows/macOS 已完成实测，也不宣称可抵御
恶意同用户进程并发替换文件的 OS 级隔离。

## 验证及性能边界

自动化验证使用合成配置与注入式 provider 响应，不读取真实账号。
独立规格审核、质量安全审核及主代理验证记录见 `PHASE_1_VALIDATION_PLAN.md`。
CPU/内存已按 release CLI 启动、实际只读循环合成执行和同步 provider 等待分别
实测，见 [资源基线](CLI_RESOURCE_BASELINE_2026_11_08.md)。本命令不是常驻服务，
不能将退出后的零占用描述为后台空闲性能。

真实 provider 验收、网络 token streaming、调用期间控制/取消、后台调度、
通用写入/Bash、完整恢复及其余 Phase 1 验收仍是独立工作。
