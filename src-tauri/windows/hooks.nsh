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
!macroend
