; tailzip (꼬리집) Windows installer — build with: makensis -DVERSION=x.y.z -DSRC=<dir with tailzip.exe> -DOUT=<setup.exe> tailzip.nsi
Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !define VERSION "0.1.0"
!endif
!define APPID "tailzip"
!define EXE "tailzip.exe"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPID}"

Name "tailzip"
OutFile "${OUT}"
InstallDir "$PROGRAMFILES64\tailzip"
InstallDirRegKey HKLM "Software\${APPID}" "InstallDir"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
BrandingText "tailzip ${VERSION}"

!define MUI_ICON "tailzip.ico"
!define MUI_UNICON "tailzip.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Korean"

LangString AppTitle ${LANG_ENGLISH} "tailzip"
LangString AppTitle ${LANG_KOREAN} "꼬리집"

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=${LANG_ENGLISH} "ProductName" "tailzip"
VIAddVersionKey /LANG=${LANG_ENGLISH} "FileDescription" "tailzip installer"
VIAddVersionKey /LANG=${LANG_ENGLISH} "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "LegalCopyright" "BBQ-MAN"

Function .onInit
  !insertmacro MUI_LANGDLL_DISPLAY
FunctionEnd

Section "tailzip" SecMain
  SectionIn RO
  SetShellVarContext all
  SetOutPath "$INSTDIR"
  File "${SRC}\${EXE}"
  File "tailzip.ico"
  File "..\README.md"
  File "..\LICENSE"
  File "..\THIRD_PARTY_NOTICES.md"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\$(AppTitle).lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\tailzip.ico"
  CreateShortcut "$DESKTOP\$(AppTitle).lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\tailzip.ico"

  ; "Open with" entry for the supported archive types (does not take over the default app)
  WriteRegStr HKLM "Software\Classes\Applications\${EXE}" "FriendlyAppName" "$(AppTitle)"
  WriteRegStr HKLM "Software\Classes\Applications\${EXE}\DefaultIcon" "" "$INSTDIR\tailzip.ico"
  WriteRegStr HKLM "Software\Classes\Applications\${EXE}\shell\open\command" "" '"$INSTDIR\${EXE}" "%1"'
  WriteRegStr HKLM "Software\Classes\Applications\${EXE}\SupportedTypes" ".zip" ""
  WriteRegStr HKLM "Software\Classes\Applications\${EXE}\SupportedTypes" ".zipx" ""
  WriteRegStr HKLM "Software\Classes\Applications\${EXE}\SupportedTypes" ".7z" ""
  WriteRegStr HKLM "Software\Classes\Applications\${EXE}\SupportedTypes" ".rar" ""
  WriteRegStr HKLM "Software\Classes\Applications\${EXE}\SupportedTypes" ".alz" ""
  WriteRegStr HKLM "Software\Classes\Applications\${EXE}\SupportedTypes" ".egg" ""
  WriteRegStr HKLM "Software\Classes\Applications\${EXE}\SupportedTypes" ".tar" ""
  WriteRegStr HKLM "Software\Classes\Applications\${EXE}\SupportedTypes" ".tgz" ""
  WriteRegStr HKLM "Software\Classes\.zip\OpenWithList\${EXE}" "" ""
  WriteRegStr HKLM "Software\Classes\.zipx\OpenWithList\${EXE}" "" ""
  WriteRegStr HKLM "Software\Classes\.7z\OpenWithList\${EXE}" "" ""
  WriteRegStr HKLM "Software\Classes\.rar\OpenWithList\${EXE}" "" ""
  WriteRegStr HKLM "Software\Classes\.alz\OpenWithList\${EXE}" "" ""
  WriteRegStr HKLM "Software\Classes\.egg\OpenWithList\${EXE}" "" ""
  WriteRegStr HKLM "Software\Classes\.tar\OpenWithList\${EXE}" "" ""
  WriteRegStr HKLM "Software\Classes\.tgz\OpenWithList\${EXE}" "" ""

  WriteRegStr HKLM "Software\${APPID}" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "$(AppTitle) (tailzip)"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "BBQ-MAN"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\tailzip.ico"
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "URLInfoAbout" "https://github.com/BBQ-MAN/tailzip"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Function un.onInit
  !insertmacro MUI_UNGETLANGUAGE
FunctionEnd

Section "Uninstall"
  SetShellVarContext all
  Delete "$SMPROGRAMS\tailzip.lnk"
  Delete "$SMPROGRAMS\꼬리집.lnk"
  Delete "$DESKTOP\tailzip.lnk"
  Delete "$DESKTOP\꼬리집.lnk"
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\tailzip.ico"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "Software\Classes\Applications\${EXE}"
  DeleteRegKey HKLM "Software\Classes\.zip\OpenWithList\${EXE}"
  DeleteRegKey HKLM "Software\Classes\.zipx\OpenWithList\${EXE}"
  DeleteRegKey HKLM "Software\Classes\.7z\OpenWithList\${EXE}"
  DeleteRegKey HKLM "Software\Classes\.rar\OpenWithList\${EXE}"
  DeleteRegKey HKLM "Software\Classes\.alz\OpenWithList\${EXE}"
  DeleteRegKey HKLM "Software\Classes\.egg\OpenWithList\${EXE}"
  DeleteRegKey HKLM "Software\Classes\.tar\OpenWithList\${EXE}"
  DeleteRegKey HKLM "Software\Classes\.tgz\OpenWithList\${EXE}"
  DeleteRegKey HKLM "Software\${APPID}"
  DeleteRegKey HKLM "${UNINST_KEY}"
SectionEnd
