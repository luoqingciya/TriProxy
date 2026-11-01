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

fn main() {
    #[cfg(windows)]
    {
        // Embed app.manifest so the executable requests administrator rights.
        // TUN mode cannot work without them, and failing at launch with a UAC
        // prompt beats failing later inside a core process.
        let attributes = tauri_build::Attributes::new().windows_attributes(
            tauri_build::WindowsAttributes::new().app_manifest(include_str!("app.manifest")),
        );
        tauri_build::try_build(attributes).expect("failed to run tauri-build");
    }

    #[cfg(not(windows))]
    {
        tauri_build::build()
    }
}
