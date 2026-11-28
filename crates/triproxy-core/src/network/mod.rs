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

//! Operating-system integration.

pub mod system_proxy;
pub mod tun;

pub use system_proxy::{AppliedProxy, ProxyBackend, SystemProxyManager};
// Renamed on the way out: inside `tun` the bare name is unambiguous, but at the
// crate root `unavailable_reason` says nothing about *what* is unavailable.
pub use tun::{
    authorize_tun_binaries, binary_has_tun_capability, ensure_tun_device, is_elevated,
    is_tun_authorized_for, leftovers, relaunch_elevated,
    unavailable_reason as tun_unavailable_reason,
    unavailable_reason_for as tun_unavailable_reason_for, TunAdapter, INTERFACE_NAME,
};
