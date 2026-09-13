; Kite native installer (NSIS)
Unicode True
Name "Kite"
OutFile "..\artifacts\kite-setup.exe"
InstallDir "$PROGRAMFILES64\Kite"
; 独立保存安装目录，卸载时保留，便于下一次安装继续使用用户选择的盘符。
InstallDirRegKey HKLM "Software\Kite" "InstallLocation"
RequestExecutionLevel admin

!include "MUI2.nsh"
!include "LogicLib.nsh"
!define MUI_ABORTWARNING
!define MUI_ICON "..\icons\icon.ico"
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "安装完成后启动 Kite"
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchKiteUnelevated
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

Var OldInstallDir

Function .onInit
  ; 兼容新旧安装器保存的安装位置。
  ReadRegStr $OldInstallDir HKLM "Software\Kite" "InstallLocation"
  StrCmp $OldInstallDir "" 0 +2
    ReadRegStr $OldInstallDir HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kite" "InstallLocation"
  ; 覆盖安装前结束正在运行的旧版，避免 kite.exe 被占用而复制失败。
  ExecWait '"$SYSDIR\taskkill.exe" /F /T /IM kite.exe'
  Sleep 300
FunctionEnd

Function LaunchKiteUnelevated
  ; 安装器以管理员权限运行，但 Kite 本身不需要提权。
  ; ShellExecute 让 Explorer 使用当前用户上下文启动，避免 UIPI 阻断外部快捷键。
  ExecShell "open" "$INSTDIR\kite.exe"
FunctionEnd

Section "卸载旧版本（推荐）" SEC_REMOVE_OLD
  SectionIn 1
  ; 勾选后始终先卸载旧版本，再由后续主程序 section 重新安装。
  StrCmp $OldInstallDir "" done
  IfFileExists "$OldInstallDir\uninstall.exe" 0 done
    ExecWait '"$OldInstallDir\uninstall.exe" /S'
done:
SectionEnd

Section "Kite 主程序" SEC_MAIN
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "..\target\release\kite.exe"
  SetOutPath "$INSTDIR\resources"
  File "..\resources\Everything64.dll"
  File "..\resources\open.wav"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "Software\Kite" "InstallLocation" "$INSTDIR"
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
