# CLI 启动与受限只读循环资源基线

日期：2026-11-08。基线：`034967f` 加本轮 `loop provider-readonly` 切片。
这是 release/headless 的本机实测，不是 live provider、TUI/Web 或后台常驻服务验收。

## 结果

每轮均为新进程，未清理页缓存，不能视作冷启动。下表耗时来自第一轮直接启动，
峰值 RSS 来自第二轮独立 GNU time 测量；两轮重复相同工作负载，但不是配对样本。
p95 使用 nearest rank；3 次样本的 p95 就是最大值，不作稳定尾延迟保证。

| 工作负载 | 次数/轮 | 耗时中位/p95 (ms) | 峰值 RSS 中位/p95 (KiB) |
|---|---:|---:|---:|
| CLI `tool list` | 30 | 0.717 / 0.923 | 3562 / 3840 |
| CLI `config validate`，合成无模型配置 | 30 | 0.680 / 0.754 | 3322 / 3588 |
| OpenAI-compatible 注入式只读循环 | 10 | 156.074 / 172.884 | 4618 / 4704 |
| 同上，每响应等待 2 秒，共 4 秒 | 3 | 4179.401 / 4279.899 | 4696 / 4760 |
| Anthropic 注入式只读循环 | 10 | 164.329 / 191.051 | 4700 / 4764 |
| 同上，每响应等待 2 秒，共 4 秒 | 3 | 4229.632 / 4331.975 | 4668 / 4764 |

换算后 CLI 峰值中位约 3.24–3.48 MiB，合成循环约 4.51–4.59 MiB；
两轮各 86 次的目标进程均退出 0。GNU time 轮所有峰值最大为 4764 KiB
（4.65 MiB）。这里不包含真实网络/TLS 客户端请求成本或本地 LLM 权重。

第一轮逐次 user+system CPU 合计的中位数：OpenAI active/wait 为
8.433/12.631 ms，Anthropic active/wait 为 9.247/14.238 ms。
CLI 两项中位记录为 0，单次范围分别为 0–2.270 / 0–0.635 ms；
记录为零不代表没有执行 CPU 指令。

### 同步等待，不是 daemon idle

第一轮每 50 ms 读取父进程 `/proc`，剔除首尾各约 0.3 秒：

| 工作负载 | 等待窗口 | 父进程 RSS | CPU tick 增量 |
|---|---:|---:|---:|
| OpenAI，3 次 | 每次 3.475–3.676 s | 3412–3840 KiB | 全部为 0 |
| Anthropic，3 次 | 每次 3.475–3.727 s | 3284–3664 KiB | 全部为 0 |

`CLK_TCK=100`，小于计数分辨率的 CPU 活动不可据此排除。窗口可能包含两次
响应之间的短暂验证；不含 Git 子进程的 CPU/RSS，不等于整个进程树资源。
本命令同步运行并退出，没有持续后台 worker；退出后零占用不是待机测试结果。

### 新 CLI 的实际 transport 构造边界

另用纯合成配置运行真实新命令，默认凭据解析拒绝，在发送前返回预期退出码 1。
这一条单样本构造了 HTTP 客户端，GNU time 记录耗时 0.12 s、峰值 5876 KiB
（5.74 MiB）；随后 `execution show` 核实为 Failed/8 events。
它是失败路径测量，不是 live 成功、正常模型等待或可比较的延迟分位数。

## 测量方法与纠错

- 合成 probe 调用实际 `prepare_readonly` 与注入 transport，使用真实临时 Git
  工作区、`git_status` 和 Markdown verifier，并断言 Completed、2 次迭代、
  2 次模型调用、2 次工具调用。包含 fixture/Git 初始化、审计、执行及清理成本；
  不能将其耗时称为 controller-only 延迟，也不代表生产 HTTP 请求链路。
- 第一轮直接启动的高精度 wall、CPU 合计及父进程 `/proc` 数据保留。
  其 Python `wait4.ru_maxrss` 继承启动器峰值约 15080–15336 KiB，
  **该字段已作废，不作为 steerBox 峰值证据**；原始文件未改写。
  仅两项 delay2000 的 wall 包含采样退出检测延迟，通常可达约一个 50 ms
  轮询周期，另有调度误差；CLI/active 项采用阻塞 wait，不受该轮询影响。
- 第二轮以 GNU time 小型 C 进程启动相同目标，记录 `%x %M %U %S %e`，
  86 次程序与 time 退出码均为 0；master 逐次核对 stats 并独立复算分位数。
  GNU time CPU/wall 输出仅有 0.01 秒精度，不替换第一轮启动耗时。
- GNU time maximum RSS / wait4 CPU 可能包含已等待子进程的内核资源记账，
  不是同时相加的进程树峰值；与父进程 `/proc` 稳态 RSS 不混用。
- 无真实凭据、账号或网络发送；没有据此修改性能策略或声称优于其他产品。

## 环境与构建

- Ubuntu 24.04.3 LTS，Linux `6.18.33.2-microsoft-standard-WSL2`，x86_64。
- Intel Core Ultra 5 125H，12 个逻辑 CPU；MemTotal 8,131,212 KiB。
- cargo `1.97.0-nightly (4f9b52075 2026-05-01)`，rustc
  `1.97.0-nightly (f964de49b 2026-05-07)`；release，单 Cargo writer、jobs=1。
- `steerbox` 二进制 7,363,888 bytes；SHA256：
  `d985d3430c51ae03a34af39662f95109fdca848b7a0e462b3210412911f6295e`。
- 外置 probe SHA256：
  `a4fe581d0766351d9c833fa7ff6cbd1028cbf2949f404961750c21deb87a85ce`。

## 证据与复现

证据根目录：`/temp/codex/steerbox-validation/readonly-provider-cli-20261108/`。
构建 target：`/temp/codex/steerbox-validation/foreground-upgrade-20261102/master-target`。
源工作树和项目目录没有新增构建产物；证据脚本为本机外置材料，不随本提交分发。

| 材料 | 相对于证据根目录 |
|---|---|
| 第一轮脚本/原始值/汇总 | `measure_resources.py`、`resources-run1/{raw,summary}.json` |
| 峰值纠正脚本/原始值/汇总 | `measure_peak_rss.py`、`resources-rss-run2/{raw,summary}.json` |
| 逐次输出/第二轮 time stats | 两个结果目录各自的 `logs/` |
| probe 源码/包 | `resource-probe/src/main.rs`、`resource-probe/Cargo.toml` |
| release 编译 | `release-{cli,probe}.{log,exit}` |
| CLI 默认凭据拒绝单样本 | `gnu-time-cli-credential-rejection.log` |

第一轮 raw SHA256（纠错前后保持一致）：
`82a607ba759826353b30405426f9fc7a43df9dd06af645be132f3e868639a520`。

基础工作负载可用上述 release 二进制分别执行 `tool list`、
`config validate <synthetic-config.json>`；probe 参数为
`openai 0`、`openai 2000`、`anthropic 0`、`anthropic 2000`。
内存单次复现形式：

```bash
/usr/bin/time -f '%x %M %U %S %e' -- /absolute/path/to/steerbox tool list
/usr/bin/time -f '%x %M %U %S %e' -- /absolute/path/to/steerbox-readonly-resource-probe openai 2000
```

两份 sampler 都用独占创建保护已有证据，重复运行前须另设输出目录，不能覆盖本轮数据。
完整新 CLI 使用及授权边界见 [命令契约](BOUNDED_READONLY_PROVIDER_CLI.md)。
后续仍需单独测量 live HTTP/TLS、实际模型延迟、调用中取消及真正后台常驻模式。
