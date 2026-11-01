<!--
  Copyright (C) 2026 luoqingci
  SPDX-License-Identifier: GPL-3.0-or-later
  Part of TriProxy. See LICENSE and ADDITIONAL-TERMS.md.
-->

## 本次更新（v0.2.1）

**修复：v0.2.0 的发布包里没有安装程序。** 当时流水线只打包了命令行工具，图形界面客户端从未进入发布流程——下载页上只有压缩包，没有可安装的程序。这一版补上了 Windows 安装包，并把发布说明改成先讲安装程序、再讲命令行工具。

### v0.2.0 起的功能

从「能管核心」到「能当客户端用」。v0.1.0 只有核心生命周期管理，此后补齐了订阅、配置、选核、GUI、系统代理、TUN、DNS 与分流。

**订阅与配置**

- 分享链接解析：VLESS / VMess / Trojan / Shadowsocks，支持 URI 列表与 base64 订阅
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

发布里有两类东西，用途完全不同：

### 桌面客户端（推荐普通用户）

`TriProxy_<版本>_x64-setup.exe` —— Windows 安装程序，双击安装，装完在开始菜单里。

- 这是完整的图形界面客户端：仪表盘、节点、连接、配置、日志、设置
- 安装到 `Program Files`，启动时请求管理员权限（TUN 模式需要）

> 安装包未做代码签名，Windows SmartScreen 可能提示「未知发布者」。
> 点「更多信息 → 仍要运行」即可；校验方式见下方「校验」。

### 命令行工具（脚本、服务器、进阶用户）

每个平台提供两种压缩包：

| 包 | 内容 | 适用 |
|---|---|---|
| `*-lite` | 仅 `triproxy` 可执行文件 | 想自行管理核心、或需要自行指定镜像/代理下载的用户 |
| `*-full` | `triproxy` + `cores/` 内已校验的三个核心 | 开箱即用，首次运行无需联网下载 |

压缩包**不含图形界面**，解压后得到的是 `triproxy.exe` 命令行程序。

`full` 版的核心由 CI 通过 TriProxy 自身的下载与校验流程获取，SHA256 与上游 Release
一致，未经修改。程序优先使用用户自行安装的更新版本（`triproxy core install`），
其次才用随包的 `cores/`。

`full` 版同时附带完整的许可材料：顶层 `LICENSE` 与 `ADDITIONAL-TERMS.md`（TriProxy 自身），
`cores/LICENSE-<core>.txt`（各核心的许可文本，取自与二进制相同的 tag），
`cores/LICENSE-GPL-3.0.txt`（GPLv3 正文），以及 `cores/THIRD-PARTY-NOTICES.txt`
（版本、来源、对应源码地址、sha256）。

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

- **Xray 没有控制器 API**，因此「连接」「流量」「真实延迟」三项在 Xray 下不可用；界面会说明原因
- **系统代理只在 Windows 上实测过**，macOS（`networksetup`）与 Linux（`gsettings`）的实现已写好但未经真机验证
- **订阅没有后台定时更新**，自动刷新只在显式执行 `triproxy profile refresh` 时发生
- **`split` 分流是预设规则**（局域网 + 国内直连），尚不支持自定义规则
- 暂无节点收藏与流量历史图表

## 授权

TriProxy 以 GNU GPL v3（或更新版本）发布，并附 GPLv3 第 7 条附加条款：
分发须保留版权与许可声明；分发修改版须更换名称，不得暗示与原版同源或获得原作者支持。
详见 `LICENSE` 与 `ADDITIONAL-TERMS.md`。

随包分发的核心是独立作品，适用各自授权（Mihomo GPL-3.0、sing-box GPL-3.0-or-later、
Xray MPL-2.0），完整来源、版本与对应源码地址见包内 `cores/THIRD-PARTY-NOTICES.txt`。
