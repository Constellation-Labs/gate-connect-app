; Tauri NSIS installer hooks.
;
; Stop a running `gate-connect-forwarder.exe` before the installer touches it.
;
; Tauri's template kills the *main* binary if it is running (CheckIfAppIsRunning)
; but never the external binaries it ships beside it. The forwarder is spawned
; detached and deliberately outlives the app (see crates/forwarder/src/main.rs),
; so it is normally still running when an install or update begins. Windows
; locks the image of a running executable, so `File /oname=gate-connect-forwarder.exe`
; fails with "Error opening file for writing" and the install aborts; the
; uninstaller's `Delete` on it fails silently and leaves the file behind.
;
; Promptless: the forwarder holds no credential and no state - the marker and
; port files it reads live under %LOCALAPPDATA% - and the app respawns it on
; launch while the marker is present. A missing process is a no-op.
!macro STOP_FORWARDER
  !if "${INSTALLMODE}" == "currentUser"
    nsis_tauri_utils::FindProcessCurrentUser "gate-connect-forwarder.exe"
  !else
    nsis_tauri_utils::FindProcess "gate-connect-forwarder.exe"
  !endif
  Pop $R0
  ${If} $R0 = 0
    !if "${INSTALLMODE}" == "currentUser"
      nsis_tauri_utils::KillProcessCurrentUser "gate-connect-forwarder.exe"
    !else
      nsis_tauri_utils::KillProcess "gate-connect-forwarder.exe"
    !endif
    Pop $R0
    ; Let the kernel release the image lock before the File/Delete that follows.
    Sleep 500
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro STOP_FORWARDER
!macroend

; On uninstall, remove every trace of the local proxy root CA the app created:
; the trusted cert in the per-user store, the public cert on disk, and the
; private key in Credential Manager. Leaving any half behind means a later
; reinstall reuses or collides with the stale material, so apps that validate
; via the system trust store can reject every MITM'd cert and all proxied
; HTTPS fails with no clear error. All three are per-user; each step is
; best-effort, so a missing item is a no-op.
!macro NSIS_HOOK_PREUNINSTALL
  ; Put relay tools (Codex, OpenCode) back on their own settings first, while
  ; the forwarder is still running and the app binary is still here to do it.
  ; The forwarder holds the relay port those configs name, and it dies with
  ; the next line; nothing after an uninstall would repair them. Skipped on an
  ; update, which reinstalls straight away and keeps the forwarder's port.
  ${If} $UpdateMode <> 1
    nsExec::Exec '"$INSTDIR\${MAINBINARYNAME}.exe" --revert-relay-configs'
    Pop $R0
  ${EndIf}
  !insertmacro STOP_FORWARDER

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
