; Build: cargo build --release, then makensis installer\rsnap.nsi
; Output: target\RSnap-<version>-setup.exe

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma

!define APP "RSnap"
!define EXE "rsnap.exe"
!define COMPANY "RAPL Group, s.r.o."
!define URL "https://github.com/EmperorHeyman/RSnap"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

; Version comes from Cargo.toml (the first `version = "..."` line is the package's).
!ifndef VERSION
  !searchparse /file "..\Cargo.toml" 'version = "' VERSION '"'
!endif

Name "${APP}"
OutFile "..\target\${APP}-${VERSION}-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
RequestExecutionLevel user
BrandingText "${COMPANY}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "CompanyName" "${COMPANY}"
VIAddVersionKey "LegalCopyright" "Copyright (c) 2026 ${COMPANY}"
VIAddVersionKey "FileDescription" "${APP} Setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "Sections.nsh"
!include "FileFunc.nsh"
!include "WinMessages.nsh"

!define MUI_ICON "..\assets\rsnap.ico"
!define MUI_UNICON "..\assets\rsnap.ico"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_NODESC
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Start RSnap now"

!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; Ask a running RSnap to quit so its exe can be replaced or removed.
!macro CloseRSnap
  FindWindow $0 "RSnapMain"
  ${If} $0 <> 0
    SendMessage $0 ${WM_CLOSE} 0 0 /TIMEOUT=2000
    ${For} $1 1 30
      FindWindow $0 "RSnapMain"
      ${IfThen} $0 = 0 ${|} ${Break} ${|}
      Sleep 100
    ${Next}
    Sleep 300
  ${EndIf}
!macroend

Section "RSnap" SecApp
  SectionIn RO
  !insertmacro CloseRSnap

  SetOutPath "$INSTDIR"
  File "..\target\release\${EXE}"
  File "..\LICENSE"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${COMPANY}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "${URL}"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $0
SectionEnd

Section "Start menu shortcut" SecShortcut
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}"
SectionEnd

; Same value the tray menu's "Start with Windows" writes, so the two stay in sync.
Section "Start with Windows" SecAutostart
  WriteRegStr HKCU "${RUN_KEY}" "${APP}" '"$INSTDIR\${EXE}"'
SectionEnd

Section "-Cleanup"
  ${IfNot} ${SectionIsSelected} ${SecAutostart}
    DeleteRegValue HKCU "${RUN_KEY}" "${APP}"
  ${EndIf}
  ${IfNot} ${SectionIsSelected} ${SecShortcut}
    Delete "$SMPROGRAMS\${APP}.lnk"
  ${EndIf}
SectionEnd

Section "Uninstall"
  !insertmacro CloseRSnap

  DeleteRegValue HKCU "${RUN_KEY}" "${APP}"
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  ; Clipboard temp files. Snips saved to Pictures\RSnap are yours and stay.
  RMDir /r "$TEMP\RSnap"
  DeleteRegKey HKCU "${UNINST_KEY}"
SectionEnd
