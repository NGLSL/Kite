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

Function LaunchKiteUnelevated
  ; 安装器以管理员权限运行，但 Kite 本身不需要提权。
  ; ShellExecute 让 Explorer 使用当前用户上下文启动，避免 UIPI 阻断外部快捷键。
  ExecShell "open" "$INSTDIR\kite.exe"
FunctionEnd

; 用户确认开始安装后才关闭旧 Kite。取消安装向导时，旧版继续运行。
Section "-关闭旧版 Kite" SEC_CLOSE_OLD
  SectionIn RO
  ; 只结束 Kite 自身，不递归结束它唤起的应用。
  ExecWait '"$SYSDIR\taskkill.exe" /F /IM kite.exe'
  Sleep 300
SectionEnd

Section "卸载旧版本（推荐）" SEC_REMOVE_OLD
  SectionIn 1
  ; 勾选后始终先卸载旧版本，再由后续主程序 section 重新安装。
  StrCmp $OldInstallDir "" done
  IfFileExists "$OldInstallDir\uninstall.exe" 0 done
    ; _?= 阻止 NSIS 卸载器复制到临时目录后提前返回，确保旧版完全卸载后再写入新版。
    ExecWait '"$OldInstallDir\uninstall.exe" /S _?=$OldInstallDir'
done:
SectionEnd

Function .onInit
  ; 兼容新旧安装器保存的安装位置。
  ReadRegStr $OldInstallDir HKLM "Software\Kite" "InstallLocation"
  StrCmp $OldInstallDir "" 0 +2
    ReadRegStr $OldInstallDir HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kite" "InstallLocation"

  ; 首次安装或卸载后的残留路径没有可执行的旧卸载器，不向用户展示升级选项。
  StrCmp $OldInstallDir "" no_old_install
  IfFileExists "$OldInstallDir\uninstall.exe" old_install_found
no_old_install:
  StrCpy $OldInstallDir ""
  SectionSetText ${SEC_REMOVE_OLD} ""
old_install_found:
FunctionEnd

Section "Kite 主程序" SEC_MAIN
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "..\target\release\kite.exe"
  File "..\THIRD_PARTY_NOTICES.txt"
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
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.txt"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kite"
SectionEnd
