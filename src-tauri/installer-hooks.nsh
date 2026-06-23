; Tauri NSIS installer hooks.
;
; On uninstall, remove every trace of the local proxy root CA the app created:
; the trusted cert in the per-user store, the public cert on disk, and the
; private key in Credential Manager. Leaving any half behind means a later
; reinstall reuses or collides with the stale material, so apps that validate
; via the system trust store can reject every MITM'd cert and all proxied
; HTTPS fails with no clear error. All three are per-user; each step is
; best-effort, so a missing item is a no-op.
!macro NSIS_HOOK_PREUNINSTALL
  ; Trusted cert in the per-user root store, matched by Common Name.
  nsExec::Exec 'certutil -user -delstore Root "Gate Connect Local CA"'

  ; Public cert on disk: %LOCALAPPDATA%\Gate Connect\proxy\ca-cert.pem. Drop the
  ; proxy dir too if it ends up empty (RMDir leaves it alone if not).
  Delete "$LOCALAPPDATA\Gate Connect\proxy\ca-cert.pem"
  RMDir "$LOCALAPPDATA\Gate Connect\proxy"

  ; Private key in Windows Credential Manager. The keyring crate stores it as a
  ; generic credential whose target name is "<user>.<service>", where the
  ; service is ai.constellation.gate-connect.proxy.ca-key and the user is the
  ; current account name (GetUserNameW, == %USERNAME%).
  ReadEnvStr $0 "USERNAME"
  nsExec::Exec 'cmdkey /delete:$0.ai.constellation.gate-connect.proxy.ca-key'
!macroend
