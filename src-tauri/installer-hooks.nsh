; TriProxy — a modern multi-core proxy client.
; Copyright (C) 2026 luoqingci
; SPDX-License-Identifier: GPL-3.0-or-later
;
; This program is free software: you can redistribute it and/or modify it under
; the terms of the GNU General Public License as published by the Free Software
; Foundation, either version 3 of the License, or (at your option) any later
; version. It is distributed in the hope that it will be useful, but WITHOUT ANY
; WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
; PARTICULAR PURPOSE. See LICENSE for the full text.
;
; Additional terms under GNU GPL v3 section 7 apply: see ADDITIONAL-TERMS.md.
;
; NSIS installer hooks.
;
; The application keeps all of its state in <install dir>\data. The GUI always
; runs elevated (its manifest requires it), so it can write there either way —
; but the bundled `triproxy.exe` is meant to be usable from an ordinary
; terminal, and a standard user cannot write into Program Files. Granting the
; Users group modify rights on the data directory is what makes the portable
; layout actually portable.

!macro NSIS_HOOK_POSTINSTALL
  CreateDirectory "$INSTDIR\data"
  ; S-1-5-32-545 is the built-in Users group. (OI)(CI) makes the grant inherit
  ; into the subdirectories the app creates later; M is modify.
  nsExec::Exec 'icacls "$INSTDIR\data" /grant *S-1-5-32-545:(OI)(CI)M'
!macroend
