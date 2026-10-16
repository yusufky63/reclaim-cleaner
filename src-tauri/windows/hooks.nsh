; NSIS hooks for the ReClaim Cleaner installer.

; Removes the per-user "Analyze with ReClaim" Explorer entries, which the app itself writes when
; the user turns them on in Settings. Updates run this uninstaller too (with /UPDATE); the
; entries stay then, or every update would silently turn the feature off.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegKey HKCU "Software\Classes\Directory\shell\ReClaim.Analyze"
    DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\ReClaim.Analyze"
    DeleteRegKey HKCU "Software\Classes\Drive\shell\ReClaim.Analyze"
  ${EndIf}
!macroend
