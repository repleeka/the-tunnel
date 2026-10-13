; Installer hooks for The Tunnel (included by Tauri's NSIS template).

; Writes a small script that (re)creates the firewall rule and runs it with one
; admin prompt. Phone hotspots usually count as "Public" networks, which Windows
; blocks by default, so the rule covers every network profile. The Tunnel itself
; refuses anything that isn't a local address.
!macro TUNNEL_FIREWALL_SCRIPT ACTION
  InitPluginsDir
  FileOpen $0 "$PLUGINSDIR\tunnel-firewall.cmd" w
  FileWrite $0 'netsh advfirewall firewall delete rule name="The Tunnel" >nul 2>&1$\r$\n'
  !if "${ACTION}" == "add"
    FileWrite $0 'netsh advfirewall firewall add rule name="The Tunnel" dir=in action=allow program="$INSTDIR\${MAINBINARYNAME}.exe" enable=yes profile=any >nul$\r$\n'
  !endif
  FileClose $0
  ExecShellWait "runas" "$PLUGINSDIR\tunnel-firewall.cmd" "" SW_HIDE
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Only ask for admin rights if the rule isn't there yet (e.g. not on updates).
  nsExec::Exec 'netsh advfirewall firewall show rule name="The Tunnel"'
  Pop $1
  ${If} $1 <> 0
    !insertmacro TUNNEL_FIREWALL_SCRIPT "add"
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Leave everything in place when the uninstaller runs as part of an update.
  ${If} $UpdateMode <> 1
    !insertmacro TUNNEL_FIREWALL_SCRIPT "remove"
    DeleteRegKey HKCU "Software\Classes\*\shell\TheTunnel"
    DeleteRegKey HKCU "Software\Classes\Directory\shell\TheTunnel"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "The Tunnel"
  ${EndIf}
!macroend
