<!--
  Copyright (C) 2026 luoqingci
  SPDX-License-Identifier: GPL-3.0-or-later
  Part of TriProxy. See LICENSE and ADDITIONAL-TERMS.md.
-->

## 本次更新（v0.2.2）

**修复：程序根本启动不了。** 安装后双击没有任何窗口，只弹一个错误：

```
无法定位程序输入点 TaskDialogIndirect 于动态链接库 ...\triproxy-app.exe 上
```

原因是上一版为了让程序以管理员身份运行，替换了 Tauri 的默认应用清单，**丢掉了 Common
Controls v6 的依赖声明**。没有它，Windows 加载的是旧版 comctl32，里面根本没有这个函数，
程序在进入 main 之前就死了。清单已补全。

同时修掉的：

- **图标换成了正式设计**（此前是占位图）
- **数据目录改到安装目录下**（`<安装目录>\data`），不再写 `%APPDATA%`。首次启动会把旧数据自动迁移过来
- **不再单独提供命令行压缩包**：Windows 的安装包里同时包含图形界面和 `triproxy.exe`，一个包解决两种用法
- **lite / full 两个变体**：lite 不带核心（首次运行时下载），full 直接带三个已校验的核心

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

每个平台都提供桌面安装包，两个变体：

| 变体 | 内容 | 适用 |
|---|---|---|
| `*-lite` | 图形界面 + 命令行，不含核心 | 首次运行会联网下载核心 |
| `*-full` | 同上 + `cores/` 内三个已校验的核心 | 开箱即用，首次运行无需联网 |

| 平台 | 文件 | 安装方式 |
|---|---|---|
| Windows | `TriProxy_<版本>_x64-{lite,full}.exe` | 双击安装到 `C:\Program Files\TriProxy` |
| macOS (Apple Silicon) | `TriProxy_<版本>_aarch64-{lite,full}.dmg` | 打开后拖进「应用程序」 |
| Linux | `TriProxy_<版本>_amd64-{lite,full}.AppImage` / `.deb` | AppImage 加执行权限直接运行；deb 用包管理器安装 |

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

- **Xray 没有控制器 API**，因此「连接」「流量」「真实延迟」三项在 Xray 下不可用；界面会说明原因
- **系统代理只在 Windows 上实测过**，macOS（`networksetup`）与 Linux（`gsettings`）的实现已写好但未经真机验证
- **macOS 与 Linux 的桌面安装包首次提供**，构建与产物内容由 CI 校验（确认核心已打包），但没有在真机上装过。
  Windows 版本是本地实测过的。这两个平台如果装不上，请反馈，不要当成「应该能用」
- **订阅没有后台定时更新**，自动刷新只在显式执行 `triproxy profile refresh` 时发生
- **`split` 分流是预设规则**（局域网 + 国内直连），尚不支持自定义规则
- 暂无节点收藏与流量历史图表

## 授权

TriProxy 以 GNU GPL v3（或更新版本）发布，并附 GPLv3 第 7 条附加条款：
分发须保留版权与许可声明；分发修改版须更换名称，不得暗示与原版同源或获得原作者支持。
详见 `LICENSE` 与 `ADDITIONAL-TERMS.md`。

随包分发的核心是独立作品，适用各自授权（Mihomo GPL-3.0、sing-box GPL-3.0-or-later、
Xray MPL-2.0），完整来源、版本与对应源码地址见包内 `cores/THIRD-PARTY-NOTICES.txt`。
