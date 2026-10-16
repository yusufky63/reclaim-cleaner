; NSIS hooks for the ReClaim Cleaner installer.

; Removes the per-user "Analyze with ReClaim" Explorer entries, which the app itself writes when
; the user turns them on in Settings. A setup that replaces the installed copy ("Uninstall before
; installing", the default on upgrades and the choice a passive reinstall makes) runs this
; uninstaller too, without /UPDATE but in place (`_?=`), while a real uninstall runs from a copy
; in TEMP. The entries stay in place mode, or every upgrade would silently turn the feature off.
; It runs after the uninstall, so one cancelled because ReClaim is running keeps them too.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
  ${AndIf} $EXEDIR != $INSTDIR
    DeleteRegKey HKCU "Software\Classes\Directory\shell\ReClaim.Analyze"
    DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\ReClaim.Analyze"
    DeleteRegKey HKCU "Software\Classes\Drive\shell\ReClaim.Analyze"
  ${EndIf}

  ; Tauri's "Delete app data" only removes `$APPDATA\${BUNDLEID}` and `$LOCALAPPDATA\${BUNDLEID}`,
  ; while settings, history, user rules and `.reg` backups live in `$APPDATA\ReClaim`
  ; (`settings::data_dir`). Same condition as Tauri's own removal: the box is unticked by default
  ; and never shown in silent and passive runs, so replacing the installed copy keeps the data
  ; unless the user ticks it. The portable build keeps its data beside the exe and has no
  ; uninstaller. The `$APPDATA` check keeps an unresolved shell folder from turning the path into
  ; `\ReClaim` at the root of the current drive.
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    ${If} $APPDATA != ""
      RmDir /r "$APPDATA\ReClaim"
    ${EndIf}
  ${EndIf}
!macroend
