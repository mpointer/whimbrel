; Installer hooks (bundle.windows.nsis.installerHooks in tauri.conf.json).
;
; Through 0.9.32 the app was called Kestrel. Tauri's installer finds a previous install by
; product name (Uninstall\<productName>), so a Whimbrel installer never sees the Kestrel
; one and installs beside it. Before installing, this runs the Kestrel uninstaller.
;
; It runs it with /P (passive) and without /UPDATE: in update mode the uninstaller keeps
; its Start menu shortcut and its Run value, and both should go. It still never deletes
; app data: that needs the "delete app data" box, which passive mode never shows. And it
; deletes only the files it installed, then a non-recursive RMDir. Kestrel's install
; folder is also the data folder (%LOCALAPPDATA%\Kestrel), so this matters.
;
; Removing Kestrel removes its Start menu shortcut, and an update (the updater runs the
; installer with /UPDATE) never creates one. So after a Kestrel install was removed, the
; post-install hook adds the Whimbrel shortcut when it's missing. Without it, a PC that
; updated from Kestrel would have no Start menu entry at all.

Var RetiredKestrel

!macro NSIS_HOOK_PREINSTALL
  ; An update starts this installer and then exits the app, and the app's supervisor exits after it.
  ; The installer's next step shuts down a Whimbrel it finds running (Restart Manager), and that fails
  ; for one in the middle of exiting: "Failed to kill Whimbrel. Please close it first then try again"
  ; (a Windows user, 2026-11-10). So first wait up to 15 s for them to finish exiting on their own.
  ; A Whimbrel that is still running after that is left to the installer's own check (Restart Manager
  ; shuts it down), as before. Only in an update: run by hand, the installer asks at once.
  Push $R8
  Push $R9
  StrCpy $R8 0
  ${If} $UpdateMode <> 1
    Goto whimbrel_wait_done
  ${EndIf}
  whimbrel_wait_for_exit:
    nsis_tauri_utils::FindProcessCurrentUser "${MAINBINARYNAME}.exe"
    Pop $R9
    ; 0 means it is still running.
    ${If} $R9 = 0
    ${AndIf} $R8 < 30
      Sleep 500
      IntOp $R8 $R8 + 1
      Goto whimbrel_wait_for_exit
    ${EndIf}
  ${If} $R8 > 0
    DetailPrint "Waited $R8 x 0.5 s for Whimbrel to exit"
  ${EndIf}
  whimbrel_wait_done:
  Pop $R9
  Pop $R8

  StrCpy $RetiredKestrel 0
  Push $0
  Push $1
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kestrel" "UninstallString"
  ReadRegStr $1 HKCU "Software\mpointer\Kestrel" ""
  ${If} $0 != ""
  ${AndIf} $1 != ""
  ${AndIf} ${FileExists} "$1\uninstall.exe"
    DetailPrint "Removing the previous Kestrel install from $1"
    ; _?= runs the uninstaller in place, so ExecWait waits for it to finish.
    ExecWait '"$1\uninstall.exe" /P _?=$1'
    ; Running in place, it can't delete itself.
    Delete "$1\uninstall.exe"
    DeleteRegKey HKCU "Software\mpointer\Kestrel"
    StrCpy $RetiredKestrel 1
  ${EndIf}
  ; The uninstaller removes this. Delete it too in case the uninstall didn't run.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Kestrel"
  Pop $1
  Pop $0
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $RetiredKestrel = 1
  ${AndIfNot} ${FileExists} "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    Push $0
    Push $1
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    Pop $1
    Pop $0
  ${EndIf}
!macroend
