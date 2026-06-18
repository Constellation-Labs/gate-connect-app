; Tauri NSIS installer hooks.
;
; On uninstall, remove the local proxy root CA the app installed into the
; current user's "Trusted Root Certification Authorities" store. Leaving it
; behind means a later reinstall generates a fresh CA while the stale one
; stays trusted, so apps that validate via the system trust store reject every
; MITM'd cert and all proxied HTTPS fails with no clear error. The CA is
; installed per-user (certutil -user -addstore Root) and matched here by its
; Common Name. Best-effort: a missing cert just makes certutil a no-op.
!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'certutil -user -delstore Root "Gate Connect Local CA"'
!macroend
