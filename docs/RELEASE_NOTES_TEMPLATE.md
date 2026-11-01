<!--
  Copyright (C) 2026 luoqingci
  SPDX-License-Identifier: GPL-3.0-or-later
  Part of TriProxy. See LICENSE and ADDITIONAL-TERMS.md.
-->

## 下载哪一个？

每个平台提供两种压缩包：

| 包 | 内容 | 适用 |
|---|---|---|
| `*-lite` | 仅 `triproxy` 可执行文件 | 想自行管理核心、或需要自行指定镜像/代理下载的用户 |
| `*-full` | `triproxy` + `cores/` 内已校验的三个核心 | 开箱即用，首次运行无需联网下载 |

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
triproxy start       # 启动（lite 版会提示先 core install）
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

## 授权

TriProxy 以 GNU GPL v3（或更新版本）发布，并附 GPLv3 第 7 条附加条款：
分发须保留版权与许可声明；分发修改版须更换名称，不得暗示与原版同源或获得原作者支持。
详见 `LICENSE` 与 `ADDITIONAL-TERMS.md`。

随包分发的核心是独立作品，适用各自授权（Mihomo GPL-3.0、sing-box GPL-3.0-or-later、
Xray MPL-2.0），完整来源、版本与对应源码地址见包内 `cores/THIRD-PARTY-NOTICES.txt`。
