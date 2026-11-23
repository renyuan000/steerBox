# Markdown CLI：离线预检合同

日期：2026-11-23；基线 `b25a8b1`。本片接线已有 Markdown library runner 的纯预检，
不是模型驱动写入命令；实现、测试和提交状态以本轮外置收据为准。

```text
steerbox loop preflight-markdown <config.json> <project_id> <goal_id> <loop_run_id> <task_run_id> <model_id> <objective> <observation> --workspace <absolute> --path <relative.md>...
```

沿用 `BOUNDED_MARKDOWN_RUNTIME_GATES.md` 的模型/提示绑定和 scope/runtime 检查，
不复制一套 runner。不开放 `provider-markdown`、任意文件工具、Bash、自动授权或后台运行。

## 输入和检查顺序

1. 在读取配置前完整解析参数。workspace 必须绝对；1..256 个不同的普通相对
   Markdown 路径；拒绝路径穿越、绝对路径、反斜杠、控制字符、重复 workspace、重复路径及未知参数。
   本片不接受 credential、approval 或 max-iterations 参数。
2. 读取并验证配置；错误输出固定分类，不回显路径、模型 ID、objective/observation、
   endpoint、auth_ref 或原始 JSON/OS 错误。配置只能包含既有 schema 支持的模型参数及
   opaque auth_ref，不应向配置或命令行填入真实密钥。配置加载仍是既有整文件 JSON 读取，
   本片不提供配置文件自身的安全打开/大小上限保证。
3. 用 `approved=false`、`HumanSteerableLoop` 构造 `BoundedMarkdownRunConfig`，
   复用 `prepare_markdown`。scope 只检查真实文件元数据，不读取文件正文。
4. 用 `validate_runtime_paths` 检查 events、model 成败/lock、tool/lock、checkpoint、
   trace、side-effect 和 state 的全部十一条目的地，保留 no-symlink、single-link、
   常规文件类型和别名拒绝。父目录须已存在；预检不自动初始化。
5. CLI 额外要求 canonical workspace 与 canonical runtime `.steerbox` 双向不嵌套、
   不相等。workspace 可放在 `data_root/workspace`，与 `.steerbox` 为兄弟目录；
   本片不扩大 library runner 的隔离合同。

## 结果

成功 exit0，仅输出一行：

```text
preflight=passed scope=markdown execution=disabled credentials=not_checked transport=not_checked history=not_checked
```

失败 exit1，输出固定参数、配置或 scope/runtime 检查错误；沿用 CLI 的静态 usage。
不打开 EventLog/ModelCallStore，不读历史正文、不解析凭据、不创建 transport、不联网、
不创建运行文件/目录、不编辑 Markdown、不消费审批/预算、不预留运行身份。

这是离散时点元数据检查，不验证可写性、文件正文/链接、历史身份/完整性、帐号可用性，
不是执行许可，也不防同 UID/root 并发替换。重复预检不等于重试执行，不写状态或审计。
真实写入 CLI、状态/取消接线、实际写链安全复审和获准 live E2E 仍为独立待办。

## 验证入口

仅使用合成配置和自动清理的 fixture，外置 Cargo target；先记录新 CLI 的行为 RED，
再运行 `cargo test --locked --offline --test provider_markdown_cli`，复跑既有
`provider_markdown_gates`、`provider_markdown_runner`、`provider_readonly_cli`。
覆盖双协议、固定无回显输出、完整参数先拒绝、缺失父目录、全部 store/lock/state
类型/别名、scope symlink/hardlink、workspace/runtime 隔离、现有坏历史不解析及fixture不变。
无正文读取/运行写入另有生产调用链审查依据；本片没有执行syscall跟踪，
不将最终fixture不变夸大为测试已证明执行期间无瞬时写入或零read系统调用。
最终需要全量测试、严格 Clippy、实际 build、fmt/diff、独立规格及质量审核。
测试不代表 live 验收或 Phase 1 完成。
