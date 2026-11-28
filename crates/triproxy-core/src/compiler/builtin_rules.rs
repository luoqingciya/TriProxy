// TriProxy — a modern multi-core proxy client.
// Copyright (C) 2026 luoqingci
// SPDX-License-Identifier: GPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version. It is distributed in the hope that it will be useful, but WITHOUT ANY
// WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
// PARTICULAR PURPOSE. See LICENSE for the full text.
//
// Additional terms under GNU GPL v3 section 7 apply: see ADDITIONAL-TERMS.md.

//! Built-in out-of-the-box split routing rules.
//!
//! Used automatically when a subscription does not supply its own `rules:`
//! (for example, bare URI lists or base64 subscriptions). Subscription rules,
//! when present, take precedence over this fallback.

use super::rules::{CustomRule, RuleKind, RuleTarget};

const LAN_CIDRS: &[&str] = &[
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "224.0.0.0/4",
    "::1/128",
    "fc00::/7",
    "fe80::/10",
];

const LOCAL_SUFFIXES: &[&str] = &["local", "localhost", "lan", "home.arpa", "internal"];

const DIRECT_CN_SUFFIXES: &[&str] = &[
    "cn",
    "qq.com",
    "weixin.qq.com",
    "wechat.com",
    "tencent.com",
    "qcloud.com",
    "gtimg.com",
    "taobao.com",
    "tmall.com",
    "alicdn.com",
    "aliyun.com",
    "alipay.com",
    "alibaba.com",
    "jd.com",
    "360buyimg.com",
    "baidu.com",
    "bdstatic.com",
    "bcebos.com",
    "bilibili.com",
    "bilivideo.com",
    "hdslb.com",
    "zhihu.com",
    "zhimg.com",
    "163.com",
    "126.com",
    "netease.com",
    "meituan.com",
    "dianping.com",
    "douyin.com",
    "toutiao.com",
    "bytedance.com",
    "byteimg.com",
    "weibo.com",
    "sina.com.cn",
    "xiaohongshu.com",
    "xhscdn.com",
    "mi.com",
    "xiaomi.com",
    "huawei.com",
    "huaweicloud.com",
    "pinduoduo.com",
    "yangkeduo.com",
    "ctrip.com",
    "12306.cn",
    "douban.com",
    "csdn.net",
    "gitee.com",
    "feishu.cn",
    "dingtalk.com",
    "wps.cn",
    "iqiyi.com",
    "youku.com",
    "mgtv.com",
    "steamcontent.com",
];

const PROXY_GLOBAL_SUFFIXES: &[&str] = &[
    "google.com",
    "googleapis.com",
    "gstatic.com",
    "googleusercontent.com",
    "youtube.com",
    "ytimg.com",
    "googlevideo.com",
    "github.com",
    "githubusercontent.com",
    "githubassets.com",
    "openai.com",
    "chatgpt.com",
    "anthropic.com",
    "claude.ai",
    "x.com",
    "twitter.com",
    "twimg.com",
    "t.me",
    "telegram.org",
    "discord.com",
    "discordapp.com",
    "reddit.com",
    "wikipedia.org",
    "cloudflare.com",
    "docker.io",
    "npmjs.org",
    "crates.io",
    "pypi.org",
    "huggingface.co",
    "netflix.com",
    "spotify.com",
    "twitch.tv",
    "instagram.com",
    "facebook.com",
    "whatsapp.com",
];

/// Return the built-in split ruleset (LAN direct, CN domains direct, common
/// overseas/AI/dev services proxied). Core-native geo rules (`geosite-cn` and
/// `geoip-cn`) are appended by each compiler after this list.
pub fn builtin_split_rules() -> Vec<CustomRule> {
    let mut rules = Vec::with_capacity(
        LAN_CIDRS.len()
            + LOCAL_SUFFIXES.len()
            + DIRECT_CN_SUFFIXES.len()
            + PROXY_GLOBAL_SUFFIXES.len(),
    );

    for cidr in LAN_CIDRS {
        rules.push(CustomRule {
            kind: RuleKind::IpCidr,
            value: (*cidr).to_string(),
            target: RuleTarget::Direct,
            group: None,
        });
    }
    for suffix in LOCAL_SUFFIXES {
        rules.push(CustomRule {
            kind: RuleKind::DomainSuffix,
            value: (*suffix).to_string(),
            target: RuleTarget::Direct,
            group: None,
        });
    }
    for suffix in DIRECT_CN_SUFFIXES {
        rules.push(CustomRule {
            kind: RuleKind::DomainSuffix,
            value: (*suffix).to_string(),
            target: RuleTarget::Direct,
            group: None,
        });
    }
    for suffix in PROXY_GLOBAL_SUFFIXES {
        rules.push(CustomRule {
            kind: RuleKind::DomainSuffix,
            value: (*suffix).to_string(),
            target: RuleTarget::Proxy,
            group: None,
        });
    }

    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_rules_cover_lan_cn_and_global_proxy() {
        let rules = builtin_split_rules();
        assert!(rules.iter().any(|r| r.kind == RuleKind::IpCidr
            && r.value == "192.168.0.0/16"
            && r.target == RuleTarget::Direct));
        assert!(rules.iter().any(|r| r.kind == RuleKind::DomainSuffix
            && r.value == "qq.com"
            && r.target == RuleTarget::Direct));
        assert!(rules.iter().any(|r| r.kind == RuleKind::DomainSuffix
            && r.value == "github.com"
            && r.target == RuleTarget::Proxy));
    }
}
