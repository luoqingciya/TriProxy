<!--
  TriProxy — a modern multi-core proxy client.
  Copyright (C) 2026 luoqingci
  SPDX-License-Identifier: GPL-3.0-or-later

  This program is free software: you can redistribute it and/or modify it under
  the terms of the GNU General Public License as published by the Free Software
  Foundation, either version 3 of the License, or (at your option) any later
  version. It is distributed in the hope that it will be useful, but WITHOUT ANY
  WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
  PARTICULAR PURPOSE. See LICENSE for the full text.

  Additional terms under GNU GPL v3 section 7 apply: see ADDITIONAL-TERMS.md.
-->

# ARCHITECTURE — Phase 1 实现说明

本文记录 Phase 1（Rust 多核心管理器 MVP）的实际架构，以及与 `PROJECT_PLAN.md` 的差异和原因。
规划书描述的是目标态，本文描述的是**已经落地并被验证的部分**。

---

## 1. 分层

```text
triproxy-cli                 命令行（未来替换为 Tauri 后端 + React UI）
      │
triproxy-core
      │
CoreManager  ── 独占性、切换流程、状态持久化、崩溃恢复
      │
ProxyCore    ── 统一 trait：version / check / start / stop / health / adopt
      │
CoreDialect  ── 每个核心的差异：参数、版本解析、配置渲染、能力声明
      │
Supervisor   ── 进程生命周期、就绪探测、日志捕获、PID 探活
      │
Mihomo / sing-box / Xray
```

关键约束：`triproxy-core` 不依赖任何 UI 技术。Phase 5 的 Tauri 后端只需把它作为依赖引入，
不需要重写任何业务逻辑。这正是规划书 3.3「UI 与 Core 完全解耦」的落地方式。

---

## 2. 与规划书的差异（重要）

### 2.1 `ProxyCore` 使用 `&self`，而非 `&mut self`

规划书 9 章的 trait 使用 `async fn start(&mut self, ...)`。落地时改为 `&self`，
状态放在实现内部的 `tokio::sync::Mutex` 里。

原因：`CoreManager` 需要持有 `Arc<dyn ProxyCore>`。若方法签名是 `&mut self`，Manager 只能写成
`Arc<Mutex<dyn ProxyCore>>`，形成「Manager 锁 → Core 锁」的嵌套，切换核心时极易死锁，
且无法并发查询三个核心的状态。改为 `&self` + 内部锁后，Manager 自己用一把 `op_lock`
串行化生命周期操作（对应规划书 14 章的「Lock CoreManager」），互斥关系只有一层。

### 2.2 新增 `adopt()`：跨进程接管

CLI 每次调用是独立进程，而核心进程必须常驻。若只在内存里持有 `Child` 句柄，
`triproxy start` 退出后 `status` / `stop` 就彻底失效。

因此管理器把 `{active_core, pid, listen_port}` 持久化到 `state.json`，
下次调用时接管仍在运行的进程（`Supervisor::adopt`），并把存活判定从
`Child::try_wait()` 切换为 PID 探活（Windows 用 `tasklist`，Unix 用 `kill -0` / `/proc`）。
PID 已消失则清理陈旧状态。

GUI 是单进程长驻形态，接管逻辑自然退化为普通句柄管理，不会产生副作用。

### 2.3 `bootstrap.rs` 不是 Config Compiler

Phase 1 需要真实可用的配置才能验收「三个核心能稳定启停」，
但又不能提前把 Phase 3 的 Unified Profile 拉进来。因此单独设 `bootstrap` 模块，
只生成「直连 + 本地入站」的最小配置，并在代码注释里明确标注：

> 这是 Phase 1 的占位实现，Phase 3 的 `ConfigCompiler` 会整体替换它，
> 期间不得让它长成第二个编译器。

生成的配置会先经过各核心自己的校验命令（`mihomo -t` / `sing-box check` / `xray -test`），
校验失败即视为启动失败，不做「先跑起来再说」。

### 2.4 核心二进制：默认不带，发布时可选带

规划书 6 章把二进制放在 `src-tauri/binaries/`。实际拆成两种发行形态：

- **`lite`（默认）**：不随包分发，首次运行下载 + 强制校验。安装包小，不承担再分发义务。
- **`full`**：由 GitHub Actions 在打包时下载并校验后放入 `cores/`，随包分发未修改的上游构建。

校验和始终取自 `api.github.com` 的 asset `digest`（回退 `.dgst` / `checksums.txt`），
取不到就拒绝安装（`--allow-unverified` 是显式逃生舱，不是默认值）。
发布流水线复用同一套下载与校验代码，因此 CI 打包与终端用户下载受同等保护。

配套的解析顺序（`BinarySource`）：

```text
env(TRIPROXY_*_BIN) → managed(<data>/bin) → bundled(<exe_dir>/cores) → PATH
```

`managed` 排在 `bundled` 之前：用户用 `core install` 更新过核心后，不应悄悄回落到
随包的旧版本。`cores/` 在运行时只读，所有写入都进 `bin/`。
版本展示不需要执行核心——`full` 包同时附带安装记录（`cores/<core>.json`），
`status` 直接读记录即可显示版本与来源。

### 2.5 下载必须支持镜像、卡死检测与吞吐下限

实测（2026-11，中国大陆网络）：直连 GitHub Release 资源约 90 B/s，22 MB 的 Mihomo 包
180 秒只下到 16 KB。因此：

- 引入 `--mirror` / `TRIPROXY_GITHUB_MIRROR`，候选顺序为「用户镜像 → 直连 → 内置镜像」；
- 下载改为流式 + 15 秒无进展即放弃并换下一个源；
- **额外加 40 kB/s 的平均吞吐下限**（20 秒宽限期后生效），300 秒为兜底上限；
- 额外提供 `--file`，允许用户用任意方式下载后交给 TriProxy 安装。

吞吐下限是发布流水线干跑时补上的：最初只有「无进展」判定，结果一个 90 B/s 的源
每 14 秒还能挤出几个字节，卡死计时器永远不触发，单次尝试被整体超时拖了 10 分钟，
回退链形同虚设。只判断「有没有数据」不够，必须判断「速率是否可用」。

由于校验和来自 `api.github.com` 而非镜像，镜像只能造成失败，无法投毒。

### 2.6 日志双写

内存环形缓冲随进程销毁，跨调用不可见。核心输出同时写入 `logs/<core>.log`
（`<epoch_ms>\t<level>\t<text>`），`triproxy logs` 读文件，`-f` 轮询追加。
超过 8 MB 轮转一代。

### 2.7 CLI 退出方式

启动核心后 CLI 必须立即返回，但 tokio 的进程驱动器在运行时销毁时会等待它派生过的子进程。
普通 `return` 会导致 `triproxy start` 永不返回（实测挂起 3 分钟以上）。
因此 `main` 在刷完 stdout/stderr 后直接 `std::process::exit`，跳过运行时析构。
这是「CLI 故意留下常驻守护进程」场景的标准做法。

### 2.8 Windows 句柄继承：一个会误导人的坑

`triproxy start | tee log` 这类管道用法最初会「挂死」：命令输出已经完整打印，核心也已启动，
但管道永不结束。实测确认——杀掉核心进程的瞬间管道立刻结束。

根因不在 CLI 退出，而在 Windows 的句柄继承：子进程创建时继承句柄是开启的，
TriProxy 自己从父进程（shell 管道 / IDE 终端 / 未来 GUI 的日志面板）继承来的 stdout
是可继承句柄，于是核心进程在整个生命周期里一直持有那个管道的写端，
读取方永远等不到 EOF。

修复：在 `spawn` 前对自身的 STD_INPUT/OUTPUT/ERROR 调用 `SetHandleInformation(HANDLE_FLAG_INHERIT, 0)`，
只清掉「继承来的」可继承标志；TriProxy 为核心创建的 stdout/stderr 管道不受影响，
它们本来就是为该子进程创建的可继承句柄。

这个坑对脚本化和 GUI 集成都会致命，因此记录在此，避免后续被当成「偶发卡顿」重新排查一遍。

---

## 3. 状态机与切换流程

```text
Stopped ──start──▶ Starting ──就绪──▶ Running ──stop──▶ Stopping ──▶ Stopped
                      │                                      │
                      └──────────── Failed ◀─────────────────┘
```

「就绪」的判定不是「进程还活着」，而是**本地入站端口能建立 TCP 连接**（默认 10 秒超时）。
启动阶段进程提前退出，会把输出尾部拼进错误信息一起报出。

`switch` 的步骤全部记录在 `SwitchReport.steps` 里并直接打印，对应规划书 14 章的
13 步流程与 23 章「自动决策必须可解释」。Phase 1 实际执行的子集：

```text
1. stop current core        （含等待进程退出）
2. cleanup network state    （Phase 1 无系统代理/TUN/DNS 状态需还原，显式声明）
3. generate config
4. validate config
5. start new core + health check
6. persist state
```

---

## 4. 能力矩阵（Phase 4 的输入）

`CoreCapability` 是静态声明而非运行时探测——这是「自动选择可解释」的前提。
`ProfileRequirement::unmet_by()` 实现了规划书 13 章的硬约束淘汰：
硬性不兼容直接出局，不参与加权评分。

当前矩阵（仅声明已确认的部分）：

| 能力 | Mihomo | sing-box | Xray |
|---|---|---|---|
| TUN | ✓ | ✓ | ✗（需外部工具） |
| DNS | ✓ | ✓ | ✓ |
| Reality / Vision | ✓ | ✓ | ✓ |
| 原生 rule-set | ✗（用 rule-provider） | ✓ | ✗ |
| proxy-groups | ✓ | ✓ | ✗ |

Xray 缺少原生 TUN 这一条会直接影响 Smart 模式的选型结果，是真实差异而非笔误。

---

## 5. 发布流水线

`.github/workflows/release.yml`，推送 `v*` tag 或手动触发。

矩阵只使用**原生 runner**：`triproxy core install` 按 `std::env::consts` 判断目标平台，
交叉编译会把错误架构的核心打进包，所以不做交叉构建。

```text
windows-latest  → x86_64-pc-windows-msvc
ubuntu-latest   → x86_64-unknown-linux-gnu
macos-latest    → aarch64-apple-darwin
```

macOS Intel（`x86_64-apple-darwin`）**不在支持范围**：Apple 已停止销售 Intel Mac，
GitHub 的 Intel runner 也长期排队，维护一条几乎无人使用的构建线不划算。
`binaries/manifest.rs` 仍保留该目标的资产匹配规则，用户自行编译时可以正常使用，
只是不发布官方包。

每个任务产出两个包：

1. 编译 → `dist/lite/`（二进制 + README/LICENSE/NOTICE/ADDITIONAL-TERMS）
2. 用刚编译出的二进制执行 `core install mihomo|sing-box|xray --data-dir <tmp>`，
   把 `bin/` 下的二进制与安装记录复制进 `dist/full/cores/`
3. 用 `jq` 读安装记录生成 `cores/THIRD-PARTY-NOTICES.txt`（版本 / 授权 / 源码 tag 地址 / sha256）
4. **冒烟验证**：用全新数据目录运行 `dist/full/triproxy cores`，断言输出含 `bundled`。
   这一步能拦住「包做出来了但程序找不到核心」这类只在发布后才暴露的问题
5. 打包（Windows `zip` / Unix `tar.gz`）并上传 artifact

release 任务汇总所有 artifact，`gh release create` 发布，说明取自
`docs/RELEASE_NOTES_TEMPLATE.md`（两种包的取舍、lite 版安装指引、授权与随包核心的合规说明）。

## 6. Phase 2 — Unified Profile

```text
订阅文本 / 分享链接
      ↓  profile::parse_payload
Profile { id, name, metadata, nodes, groups }
      ↓  （Phase 3）
mihomo.yaml / sing-box.json / xray.json
```

### 6.1 模型

`Node` 由「协议 + 地址 + 端口 + 凭据 + 传输 + TLS」组成，`Credentials` 用枚举区分四种协议，
`Transport` 覆盖 tcp / ws / grpc / h2，`TlsConfig` 里 `reality: Option<RealityConfig>`
表示 Reality。VLESS 的 `flow` 单独建模为 `VlessFlow::{None, Vision}`，而不是塞进 `Transport`，
因为它是凭据层语义（对应能力矩阵里的 `supports_vision`）。

**`rules` / `dns` / `network` 没有建模**，尽管规划书的 `Profile` 结构里列了它们。
理由：这三者只有在消费它们的编译器存在（Phase 3）之后才有确定形状——
Clash 的 rule-provider 与 sing-box 的 rule-set 不是同一个概念，提前拍一个统一模型必然返工。
`groups` 已定义（形状简单且确定），Phase 3 填充。

### 6.2 节点身份与显示名解耦

`Node::id` 是连接参数的哈希（协议、地址小写化、端口、凭据、传输），**不含显示名**。
服务商给节点改名不会产生新节点，收藏和延迟历史因此不会丢。
`Profile::id` 是排序后节点 id 的哈希，同一份订阅重复导入得到同一个 id（Phase 9 去重的基础）。

名称唯一性在解析阶段强制：Clash 与 sing-box 都以名称作为代理的键，重名会互相覆盖，
所以重名节点自动加 ` (2)` 后缀，而不是让导入失败。

### 6.3 分享链接的历史包袱

四种协议没有统一的 URI 规范，解析器是手写的（没有用通用 URL 库，因为 `ss://` 的
legacy 形态根本不是合法 URL）：

- **`ss://` 两种格式**：SIP002（`base64(method:password)@host:port`）与 legacy
  （整个 body 是 `base64(method:password@host:port)`）。用「有没有 `@`」区分；
  userinfo 里有没有 `:` 再决定要不要 base64 解码——base64 字母表不含 `:`，这个判定无歧义。
- **`vmess://` 是 base64 的 JSON**，且 `port` / `aid` 在不同客户端里可能是字符串或数字，
  所以用 `serde_json::Value` 再强制转换，而不是直接声明成 `u16`。
- **query 键名不统一**：`serviceName` / `path`、`security` / `tls`、`allowInsecure` 大小写不一。
  键统一转小写后按别名匹配。
- **不支持的传输/加密/flow 直接报错**，不静默降级。例如 `net=kcp` 的 VMess 节点会被拒绝并
  说明原因，而不是编译成一个跑不通的配置。

### 6.4 诊断而非失败

`parse_payload` 永不 panic，也永不因为单行失败而丢弃整份订阅：坏行变成带行号的 `Diagnostic`，
其余节点照常导入。校验同样返回诊断列表，分 `Error`（节点不可用）与 `Warning`
（能连但大概率不是用户想要的，如 `allowInsecure`、Reality 公钥长度异常、Vision 无 TLS）。

`profile check` 在有 `Error` 时退出码为 1，便于脚本化。

### 6.5 与 Phase 4 的接口

`Profile::requirement()` 直接产出 `ProfileRequirement`，供 Smart Core Selector 使用。
只报告节点层面可见的需求（协议、Reality、Vision、WireGuard）；TUN / DNS / rule-set
属于网络与规则配置，Phase 3 才有。

## 7. Phase 3 — Config Compiler

```text
Profile
  ├─→ MihomoCompiler   → config.yaml   (YAML)
  ├─→ SingBoxCompiler  → config.json   (JSON)
  └─→ XrayCompiler     → config.json   (JSON)
```

### 7.1 三条设计规则

1. **无法忠实表达就跳过，不做近似**。静默丢掉一个传输选项会生成「能连但行为不同」的配置，
   比一个明显缺失的节点危险得多。被跳过的节点带原因记录在 `CompiledConfig::skipped` 里。
2. **硬能力不匹配直接整体失败**。`compile()` 先跑 `ProfileRequirement::unmet_by()`，
   核心根本服务不了这份 Profile 时不出配置（规划书 13 章）。
3. **生成的配置必须先由核心自己校验**。`CoreManager` 写盘后立刻调用
   `mihomo -t` / `sing-box check` / `xray -test`，编译器有 bug 会表现为「配置被拒绝」，
   而不是「核心起来了但行为不对」。

### 7.2 真实核心教会我们的事

Xray 编译器最初把 Reality + WebSocket 节点照直写进 `streamSettings`，被 Xray 自己否掉了：

```text
REALITY only supports RAW, XHTTP and gRPC for now.
```

这是编译期就该知道的约束，已改为在编译时跳过并说明原因。mihomo 与 sing-box 都接受该组合，
所以只在 Xray 编译器中限制——**每个核心的限制来自该核心自己的校验器，不靠猜**。

同类约束还有：trojan 没有 TLS 就不是 trojan（三个编译器统一拒绝），
Xray 不支持 shadowsocks plugin（跳过），sing-box 反而原生支持 SIP003 plugin（透传）。

### 7.3 各核心的结构差异

| | Mihomo | sing-box | Xray |
|---|---|---|---|
| 节点选择 | `proxy-groups: select` | `selector` outbound | **无**，只有第一个 outbound 生效 |
| 传输 | `network` + `ws-opts`/`grpc-opts`/`h2-opts` | `transport` 对象 | `streamSettings` |
| Reality | `reality-opts` | `tls.reality` | `streamSettings.realitySettings` |
| 本地入站 | `mixed-port` | `mixed` inbound | socks + http 两个入站（相邻端口） |

Xray 没有 selector，所以多节点时只有第一个被使用。这是核心能力差异，不是实现缺陷；
节点选择属于 Phase 4，而 Mihomo/sing-box 已经提供了可用的选择器。

### 7.4 YAML 是手写的

Mihomo 需要 YAML 输出，但项目只**写**不读，而维护中的序列化库都会带上完整解析器。
因此写了 `compiler::yaml`：只支持 map / list / scalar，所有字符串一律双引号转义。
节点名来自订阅，可能含 `:`、`#`、引号、CJK 与 emoji——裸写会静默改变配置语义。

### 7.5 与 CLI / 管理器的衔接

```bash
triproxy profile compile sub.txt --core xray -o xray.json --check
triproxy start mihomo --profile sub.txt
triproxy switch xray --profile sub.txt
```

`CoreManager::start_with_config` / `switch_with_config` 接收已渲染的配置文本；
`validate_config` 写到 `downloads/` 而不是活动配置目录——运行中的核心会监视自己的配置文件并热重载，
校验用的临时文件不能放在那里。

`bootstrap.rs` 仍然保留：不带 `--profile` 的 `start` 还没有 Profile 可用（订阅存储属于 Phase 9），
所以「直连最小配置」仍是默认路径。Phase 9 落地后这条路径应被删除。

## 8. Phase 4 — Smart Core Selector

### 8.1 没有实现规划书的加权公式，原因如下

规划书 13 章给出 `Compatibility 50% / Feature Match 25% / Performance 15% / User Preference 10%`。
这个公式没有落地，因为它有两个站不住的地方：

- **兼容性不能是权重**。跑不了这份 Profile 的核心不是「90% 兼容」，而是不可用。
  硬约束淘汰在前，留下的候选兼容性都是 100%，那 50% 的权重贡献恒为常数。
- **性能没有测量依据**。给一个没人测过的数字配 15% 权重，只会让结果看起来精确，实则任意。

### 8.2 实际采用的字典序排序

1. **硬兼容性淘汰**（`ProfileRequirement::unmet_by`）
2. **原生特性数量**：Profile 要求的可选特性里，该核心有几个是一等公民
3. **默认顺序**：sing-box → mihomo → xray

第 3 条不是性能排名，而是照搬规划书 10 章自己的定位：sing-box 是「通用现代核心」，
mihomo 面向 Clash 生态，Xray 面向 V2Ray/Xray 生态。没有生态偏好的 Profile，
默认给通用核心是更稳妥的选择。这三条全部写死在代码与文档里，同一个 Profile 永远选出同一个核心。

### 8.3 只有「差异化」特性才计分

Reality 与 Vision 三个核心都支持，把它们算作「原生特性」会给每个候选加同一个常数，
不携带任何信息。只有 `rule-set`（仅 sing-box 原生）与 `proxy-groups`（sing-box、mihomo）
参与比较。

### 8.4 应用侧需求必须单独建模

`Profile::requirement()` 只能从节点推导出协议、Reality、Vision、WireGuard。
TUN、DNS、rule-set 属于应用设置，节点里没有这些信息。如果只吃 Profile，
`requires_tun` 恒为 false，Xray 永远看起来同样合适——选型就退化成纯 tie-break。

因此引入 `NetworkRequirements { tun, dns, rule_set, proxy_groups }`，
由调用方（未来的 GUI 设置页）提供，`Selector::requirement()` 把它并进 Profile 需求。
`core select --tun` 就是这条路径的演示：打开 TUN 后 Xray 立刻被判为 ineligible。

### 8.5 解释必须包含被淘汰者

```text
  CORE      VERDICT     DETAIL
  sing-box  selected    -
  mihomo    eligible    -
  xray      ineligible  missing TUN

Reason: sing-box is the default general-purpose core; no candidate needed a
specialisation; eliminated: xray (TUN)
```

只说「为什么是它」不够，还要说「为什么不是另外两个」。核心从列表里安静消失，
正是最需要解释的那类决策（规划书 23 章）。

### 8.6 手动覆盖

`CorePreference::Fixed(kind)` 仍然会跑兼容性检查：用户手选一个跑不了这份 Profile 的核心时
直接报错并说明缺什么，而不是悄悄回退到别的核心——回退会让用户以为自己选的生效了。

## 9. 存储层（Profile 库）

GUI 需要 Profile 列表，而在此之前 Profile 只活在一次命令的生命周期里。这一层是从规划书
Phase 9（订阅系统）提前拉过来的，只做「存/取/选中」，不做下载与自动更新。

### 9.1 为什么不是 SQLite

规划书 18 章建议 SQLite。这里先用「一个 Profile 一个 JSON 文件 + 一个索引文件」：

- Profile 是整体读、整体写的对象，没有需要 join 或按条件筛选的字段；
- 数量在几十份量级，遍历目录完全够用；
- 出问题时可以直接打开文件看，备份就是复制目录。

SQLite 会在有**真正需要查询的数据**时才有价值——连接历史、延迟采样、节点使用频次。
那是后续阶段的事，现在引入数据库只会增加迁移负担。这个判断写在 `store.rs` 顶部注释里，
避免以后有人以为是漏了。

### 9.2 两个 id，用途不同

| | 来源 | 稳定性 | 用途 |
|---|---|---|---|
| `Profile::id` | 节点连接参数的哈希 | 节点变化就变 | 内容指纹，用于去重 |
| `StoredProfile::id` | 首次导入时的名称+时间 | 内容更新后不变 | 记录身份，用于「活动 Profile」与 UI 位置 |

如果只有一个 id，订阅刷新（节点变了）会产生一条新记录，用户的活动 Profile 就丢了。

`profile add` 遇到同名记录时按名称匹配并**更新**该记录，而不是新建一条同名副本。

### 9.3 损坏文件不拖垮整个库

`list()` 跳过无法解析的文件并记一条 warn，而不是返回错误。
一个坏文件让用户看不到其余 30 份订阅是不可接受的失败模式。
索引里的悬空 `active` 指向也会被自动清理。

## 10. 尚未实现（明确的边界）

刻意不做，避免半成品：

- 系统代理、TUN、DNS、路由（Phase 6–8）
- Clash YAML 导入、订阅下载与本地存储（Phase 3 剩余 / Phase 9）
- Hysteria2 / TUIC / SOCKS / HTTP 节点（Phase 10 扩展）
- 规则分流：三个编译器目前都只生成 `MATCH,PROXY` / `final: PROXY` 单条规则
- 延迟测试与 Smart Core 评分（Phase 4）
- 控制器 API 健康检查（当前用 TCP 探活）
- GUI（Phase 5）

因此 `triproxy` 目前**还不是一个可用的翻墙客户端**：它能解析订阅、把节点编译成三个核心都认可的配置、
并管理核心生命周期，但还没有系统代理/TUN 把流量真正导进去，也没有规则分流与节点选择界面。
这是规划书 38 章规定的开发顺序。
