<!--
  Copyright (C) 2026 luoqingci
  SPDX-License-Identifier: GPL-3.0-or-later
  Part of TriProxy. See LICENSE and ADDITIONAL-TERMS.md.
-->

## 本次更新（v0.2.5）

**这一版补的是"每天都要用、但一直缺"的那部分。** 装核心、切节点、刷新订阅、改端口、
写规则、点订阅链接——以前这些要么没有入口，要么要开终端。

### 节点：自动选择

编译结果里现在有两个组：

```text
AUTO   每 5 分钟重测，取延迟最低的
PROXY  手动选择，成员是 AUTO + 全部节点
```

**没选过节点 = 自动选择**，而不是「配置里排第一的那个」——后者是一个用户从没做过的选择。
节点页顶部多了一行「自动选择」，点任意节点会排到它前面成为默认；想回到自动，再点那一行。

Mihomo 与 sing-box 点一下立刻生效（控制器 API），Xray 没有这套 API，选择保存、下次启动生效。

### 分流：写自己的规则

设置 → 自定义规则，一行一条，Clash 语法：

```text
DOMAIN-SUFFIX,corp.internal,DIRECT
DOMAIN-KEYWORD,telemetry,PROXY
IP-CIDR,10.0.0.0/8,DIRECT,no-resolve
```

**自定义规则排在预设之前**，所以 `DOMAIN-SUFFIX,cn,PROXY` 能压过 split 的
`GEOSITE,cn,DIRECT`。同一个解析器喂给三个核心，各渲染成自己的方言。

保存时校验并给出**行号**；解析失败的规则不会让整个连接起不来（会被丢弃并记日志）。

### 订阅：自动刷新 + 点链接导入

- **后台自动刷新**：进程活着就有一个 1 分钟的调度器，按每个订阅自己的间隔判断到期。
  单个订阅源失败不会停表，下一轮照常重试。
- **`triproxy://install?url=…`**：在浏览器里点一下就能导入，不用再把链接粘进输入框。
  应用没在运行时由系统拉起，已在运行时（包括缩在托盘里）由单实例转交给它。

只注册 `triproxy://`，没有抢 `clash://`——装了 Clash 系客户端的机器上抢那个 scheme 是不礼貌的。

### 首次使用：界面里装核心

lite 包按设计不带核心，以前界面只显示「未安装」，唯一的出路是打开终端敲 `triproxy core install`。
现在仪表盘那一行就是「下载核心」按钮，装好的核心旁边多一个「更新」（先问上游，已是最新就只说一句）。

### 三个每天都会碰到的设置

- **监听端口**：默认 7890。被别的程序占用时改这里——以前它是写死的，那种情况下整个应用都用不了。
- **开机自启**：登录时启动 TriProxy。开关状态从操作系统读回来，不是从配置文件猜。
- **启动后自动连接**：有活动配置时，打开应用就启动核心。
- **代理绕过列表**：分号分隔。留空 = 用默认（localhost 与内网网段），不是「什么都不绕过」——
  把 localhost 也代理走会毁掉本机所有开发服务器。

### 看得见的信息

- **托盘菜单第一行**显示 `Mihomo · 自动选择` 或 `未运行`。托盘常驻的应用，最不能接受的就是
  图标看起来一样、但代理是开是关看不出来。
- **连接列表**多了「进程」列，目标下面显示命中的规则——排查「这个站为什么走了直连」时，
  这两个字段比流量数字有用得多。
- **仪表盘**画最近 3 分钟的上下行曲线（内存里，不落盘）。

### 配置备份

设置 → 配置备份，导出一个 JSON（设置 + 全部配置 + 当前活动项）。
**导入是合并，不是覆盖**：从另一台机器带来的备份不会删掉本机已有的配置。

### 修掉的两个毛病

- **控制台窗口乱闪**：更新后每 2 秒闪一个黑框。原因是判断「进程还活着吗」的方式是跑 `tasklist`，
  而 GUI 自己没有控制台，每次启动控制台程序就给它开一个窗口——应用内更新后核心被收养，
  这条路径每 2 秒走一次。现在直接问内核。顺带修掉了停止核心时闪几十次的问题。
- **启动白屏**：WebView2 在第一帧之前画的是白色，窗口本身也没有背景色。现在两处都设成主题色。

### 第一次真正验证自动更新

v0.2.3 是第一个带更新功能的版本，v0.2.4 是第一次可以真正从上一版升级。
**如果你装的是 v0.2.4，设置页点「检查更新」应该能直接升到 v0.2.5**，不用再下载安装包。

### v0.2.0 起的功能

从「能管核心」到「能当客户端用」。v0.1.0 只有核心生命周期管理，此后补齐了订阅、配置、选核、GUI、系统代理、TUN、DNS 与分流。

**订阅与配置**

- 分享链接解析：VLESS / VMess / Trojan / Shadowsocks / Hysteria2 / TUIC，支持 URI 列表、base64 订阅与 Clash 配置
- URL 订阅：条件请求缓存（`ETag` / `Last-Modified`），刷新失败不会毁掉已有节点
- 同一份 Unified Profile 编译成三个核心各自的配置，且**由核心自己校验**后才使用

**智能选核**

- 硬约束淘汰 → 原生特性数量 → 文档化默认顺序，同一份配置永远选出同一个核心
- 决策可解释：给出理由，并列出被淘汰的核心及原因

**桌面客户端**

- 仪表盘 / 节点 / 连接 / 配置 / 日志 / 设置
- 实时流量、连接列表与逐个关闭、延迟测试、订阅刷新

**网络接管**

- 系统代理：先落盘快照再改系统设置，崩溃后下次启动自愈
- TUN：管理员实测，硬杀进程后网卡与路由均被清理
- DNS：`off` / `split` / `fakeip` 三种模式，被代理的域名在隧道内解析
- 分流：`global` 全部走代理，`split` 局域网与国内目标直连

**诊断与维护**

- 两种延迟分开命名：`TCP 延迟` 只说明端点可达，`真实延迟` 由核心通过节点发请求、能证明代理可用
- `core update` 检查并安装核心更新

## 下载哪一个？

每个平台都提供桌面安装包，两个变体：

| 变体 | 内容 | 适用 |
|---|---|---|
| `*-lite` | 图形界面 + 命令行，不含核心 | 首次运行在界面里点「下载核心」，或自行管理核心版本 |
| `*-full` | 同上 + `cores/` 内三个已校验的核心 | 开箱即用，首次运行无需联网 |

| 平台 | 文件 | 安装方式 |
|---|---|---|
| Windows | `TriProxy-<版本>-windows-x64-{lite,full}.exe` | 双击安装到 `C:\Program Files\TriProxy` |
| macOS (Apple Silicon) | `TriProxy-<版本>-macos-arm64-{lite,full}.dmg` | 打开后拖进「应用程序」 |
| Linux | `TriProxy-<版本>-linux-x64-{lite,full}.AppImage` / `.deb` | AppImage 加执行权限直接运行；deb 用包管理器安装 |

文件名读作「平台-架构-变体」：`windows-x64-lite` 就是 Windows / 64 位 Intel / 精简版，
不用再靠扩展名反推系统。

**命令行都在同一个包里**，与图形界面共用同一份数据，不需要单独下载。

### 数据放在哪里

| 平台 | 位置 | 说明 |
|---|---|---|
| Windows | `<安装目录>\data` | 整个应用一个文件夹；卸载会一并删除 |
| macOS | `~/Library/Application Support/TriProxy` | `.app` 是密封包，不应写入 |
| Linux | `~/.local/share/TriProxy` | 遵循 XDG 约定 |

Windows 上首次启动会把旧版 `%APPDATA%\TriProxy` 的数据自动迁移过来。

> 安装包未做代码签名：
> - Windows SmartScreen 提示「未知发布者」→ 点「更多信息 → 仍要运行」
> - macOS 首次打开提示「无法验证开发者」→ 右键点图标选「打开」

`full` 版的核心由 CI 通过 TriProxy 自身的下载与校验流程获取，SHA256 与上游 Release
一致，未经修改。程序优先使用用户自行安装的更新版本（`triproxy core install`），
其次才用随包的 `cores/`。

随包核心是独立作品，许可文本在 `cores/LICENSE-<core>.txt`，版本、来源与对应源码地址
见 `cores/THIRD-PARTY-NOTICES.txt`。

## 首次运行

```bash
triproxy doctor      # 环境自检
triproxy profile add https://example.com/sub --name my-sub --use
triproxy start mihomo --proxy
triproxy status
```

lite 版安装核心：

```bash
triproxy core install mihomo
triproxy core install sing-box
triproxy core install xray
```

中国大陆网络直连 GitHub Release 通常不可用，可用镜像或本地文件：

```bash
triproxy core install mihomo --mirror https://ghfast.top/
triproxy core install xray --file ./Xray-windows-64.zip
```

桌面客户端源码在同一仓库，`npm install && npm run tauri build` 构建。

## 已知限制

这些是刻意留在这一版之外的，不是遗漏：

- **Xray 没有控制器 API**，因此「连接」「流量」「真实延迟」「运行时切换节点」在 Xray 下不可用；界面会说明原因
- **Xray 不支持 Hysteria2 与 TUIC**，含这两类节点的配置无法用 Xray 运行，智能选核会自动避开
- **Xray 没有代理组**，只能选单个节点，没有「自动选择」
- **协议不全**：现在支持 VLESS / VMess / Trojan / Shadowsocks / Hysteria2 / TUIC。
  缺 SOCKS5 / HTTP 出站、WireGuard、SSH、SSR、Snell、AnyTLS
- **没有规则集订阅**：自定义规则只能内联写，不能订阅远程 rule-set
- **流量曲线不落盘**：只有最近 3 分钟，关掉窗口就没了
- **节点管理很薄**：能搜索、按协议过滤、点选切换，但没有收藏、分组、手动排序
- **系统代理只在 Windows 上实测过**，macOS（`networksetup`）与 Linux（`gsettings`）的实现已写好但未经真机验证
- **托盘与深链接只在 Windows 上验证过**：macOS 与 Linux 走同一套 Tauri 接口，但没有真机点验；
  没有托盘环境时应用会退回「关闭即退出」的行为
- **开机自启在 Windows 上会弹一次 UAC**（应用清单要求管理员权限）；改用计划任务可以静默启动，尚未做
- **macOS 与 Linux 的桌面安装包仍未经真机验证**：构建与产物内容由 CI 校验（确认核心已打包），
  但没有实际安装过。Windows 版本是本地实测的。这两个平台如果装不上，请反馈，不要当成「应该能用」

## 授权

TriProxy 以 GNU GPL v3（或更新版本）发布，并附 GPLv3 第 7 条附加条款：
分发须保留版权与许可声明；分发修改版须更换名称，不得暗示与原版同源或获得原作者支持。
详见 `LICENSE` 与 `ADDITIONAL-TERMS.md`。

随包分发的核心是独立作品，适用各自授权（Mihomo GPL-3.0、sing-box GPL-3.0-or-later、
Xray MPL-2.0），完整来源、版本与对应源码地址见包内 `cores/THIRD-PARTY-NOTICES.txt`。
