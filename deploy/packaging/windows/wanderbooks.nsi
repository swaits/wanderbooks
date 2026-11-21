; The Wanderbooks installer for Windows (plan P15 unit C1; WB-pkg-1, WB-pkg-2,
; WB-pkg-7). Built by deploy/packaging/windows/build-windows.sh with NSIS
; (https://nsis.sourceforge.io, free software, zlib licence).
;
;   makensis -DVERSION=0.1.0 -DPROGRAM=...\wanderbooks.exe -DICON=...\wanderbooks.ico ^
;            -DLICENSES=<folder> -DOUTFILE=...\Wanderbooks-0.1.0-windows-x64-setup.exe wanderbooks.nsi
;
; What it does, and the rules it keeps:
;   * Per user: RequestExecutionLevel user, so double-clicking it shows no
;     administrator prompt. Everything goes under the person's own profile
;     and HKEY_CURRENT_USER.
;   * One program file, wanderbooks.exe. It is the window, the books
;     server's child and the assistants' bridge; an assistant is registered
;     with this installed path, which an update keeps (the same folder, the
;     same file name, replaced in place).
;   * A Start Menu entry named Wanderbooks, with the icon, and an entry in
;     Settings > Apps (the uninstaller).
;   * The uninstaller removes the files this installer put in the install
;     folder, the Start Menu entry and its two registry keys. It NEVER
;     removes the person's books, keys, backups or settings: they live in
;     the data folder (%LOCALAPPDATA%\wanderbooks), which this script
;     names nowhere, and no command below deletes a folder's contents or
;     uses a wildcard (a test, crates/ledger-cli/tests/packaging.rs, reads
;     this file and fails if one appears).
;   * It asks the person to quit Wanderbooks fully before it replaces or
;     removes the program (a running program cannot be replaced).
;   * Unsigned until the owner has a code-signing certificate: Windows
;     SmartScreen says so on the first run (docs/install.md).

Unicode true
SetCompressor /SOLID lzma
!include "MUI2.nsh"
!include "LogicLib.nsh"

!ifndef VERSION
  !error "pass -DVERSION=x.y.z (deploy/packaging/version.sh)"
!endif
!ifndef PROGRAM
  !error "pass -DPROGRAM=<path to wanderbooks.exe>"
!endif
!ifndef ICON
  !error "pass -DICON=<path to wanderbooks.ico>"
!endif
!ifndef LICENSES
  !error "pass -DLICENSES=<folder with LICENSE-MIT, LICENSE-APACHE and THIRD-PARTY-NOTICES.txt>"
!endif
!ifndef OUTFILE
  !error "pass -DOUTFILE=<the installer to write>"
!endif

!define NAME "Wanderbooks"
!define PUBLISHER "Wanderbooks"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Wanderbooks"
!define APP_KEY "Software\Wanderbooks"

Name "${NAME}"
OutFile "${OUTFILE}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\Wanderbooks"
InstallDirRegKey HKCU "${APP_KEY}" "InstallDir"
BrandingText "${NAME} ${VERSION}"
ShowInstDetails nevershow
ShowUninstDetails nevershow

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${NAME}"
VIAddVersionKey "FileDescription" "${NAME} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT OR Apache-2.0"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\wanderbooks.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${NAME}"

!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; A program that is running cannot be replaced or removed. Try to open the
; installed file for writing; if it is in use, ask the person to quit
; Wanderbooks (its window, which closes the books and the server) and retry.
!macro WaitUntilNotRunning
  ${Do}
    ClearErrors
    ${If} ${FileExists} "$INSTDIR\wanderbooks.exe"
      FileOpen $0 "$INSTDIR\wanderbooks.exe" a
      ${IfNot} ${Errors}
        FileClose $0
        ${Break}
      ${EndIf}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${NAME} is open. Quit it fully (close its window), then choose Retry." IDRETRY +2
      Abort "${NAME} is still open."
    ${Else}
      ${Break}
    ${EndIf}
  ${Loop}
!macroend

Section "Install"
  SetOutPath "$INSTDIR"
  !insertmacro WaitUntilNotRunning
  File "/oname=wanderbooks.exe" "${PROGRAM}"
  File "/oname=wanderbooks.ico" "${ICON}"
  File "/oname=LICENSE-MIT" "${LICENSES}\LICENSE-MIT"
  File "/oname=LICENSE-APACHE" "${LICENSES}\LICENSE-APACHE"
  File "/oname=THIRD-PARTY-NOTICES.txt" "${LICENSES}\THIRD-PARTY-NOTICES.txt"
  WriteUninstaller "$INSTDIR\Uninstall Wanderbooks.exe"

  CreateShortCut "$SMPROGRAMS\${NAME}.lnk" "$INSTDIR\wanderbooks.exe" "" "$INSTDIR\wanderbooks.ico" 0

  WriteRegStr HKCU "${APP_KEY}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\wanderbooks.ico"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "https://wanderbooks.ai"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall Wanderbooks.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall Wanderbooks.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  !insertmacro WaitUntilNotRunning
  Delete "$INSTDIR\wanderbooks.exe"
  Delete "$INSTDIR\wanderbooks.ico"
  Delete "$INSTDIR\LICENSE-MIT"
  Delete "$INSTDIR\LICENSE-APACHE"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.txt"
  Delete "$INSTDIR\Uninstall Wanderbooks.exe"
  ; Only if empty: a folder the person put something in stays.
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${NAME}.lnk"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "${APP_KEY}"
  ; The person's books, keys, backups and settings are not here: they are
  ; in the data folder, which an uninstall leaves exactly as it is.
SectionEnd
