; Kite native installer (NSIS)
Unicode True
Name "Kite"
OutFile "..\artifacts\kite-setup.exe"
InstallDir "$PROGRAMFILES64\Kite"
InstallDirRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kite" "InstallLocation"
RequestExecutionLevel admin

!include "MUI2.nsh"
!define MUI_ABORTWARNING
!define MUI_ICON "..\icons\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\kite.exe"
!define MUI_FINISHPAGE_RUN_TEXT "安装完成后启动 Kite"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

Function .onInit
  ; 覆盖安装前结束正在运行的旧版，避免 kite.exe 被占用而复制失败。
  ExecWait '"$SYSDIR\taskkill.exe" /F /T /IM kite.exe'
  Sleep 300
FunctionEnd

Section "Kite 主程序" SEC_MAIN
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "..\target\release\kite.exe"
  SetOutPath "$INSTDIR\resources"
  File "..\resources\Everything64.dll"
  File "..\resources\open.wav"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kite" "DisplayName" "Kite"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kite" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kite" "UninstallString" "$INSTDIR\uninstall.exe"
SectionEnd

Section "开始菜单快捷方式" SEC_START
  CreateDirectory "$SMPROGRAMS\Kite"
  CreateShortcut "$SMPROGRAMS\Kite\Kite.lnk" "$INSTDIR\kite.exe" "" "$INSTDIR\kite.exe" 0
SectionEnd

Section "桌面快捷方式" SEC_DESKTOP
  CreateShortcut "$DESKTOP\Kite.lnk" "$INSTDIR\kite.exe" "" "$INSTDIR\kite.exe" 0
SectionEnd

Section "Uninstall"
  Delete "$DESKTOP\Kite.lnk"
  Delete "$SMPROGRAMS\Kite\Kite.lnk"
  RMDir "$SMPROGRAMS\Kite"
  Delete "$INSTDIR\resources\Everything64.dll"
  Delete "$INSTDIR\resources\open.wav"
  RMDir "$INSTDIR\resources"
  Delete "$INSTDIR\kite.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kite"
SectionEnd
