; Kite native installer (NSIS)
Unicode True
Name "Kite"
OutFile "..\artifacts\kite-setup.exe"
InstallDir "$PROGRAMFILES64\Kite"
; 独立保存安装目录，卸载时保留，便于下一次安装继续使用用户选择的盘符。
; 覆盖安装依赖这一项：默认目录与上一版一致，新文件就地覆盖旧文件。
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
  ; 插件是独立子进程，旧版宿主退出异常时可能仍占用待覆盖的官方插件文件。
  ; 这里只结束 Kite 官方插件进程，不影响用户启动的其他程序或第三方插件。
  ExecWait '"$SYSDIR\taskkill.exe" /F /IM kite-plugin-calculator.exe /IM kite-plugin-window-switcher.exe /IM kite-plugin-devtools.exe'
  Sleep 300
SectionEnd

; 覆盖安装不单独卸载旧版本：InstallDirRegKey 已把默认目录对齐到上一版安装位置，
; 下面的主程序 section 直接就地覆盖文件、快捷方式和卸载器，设置与历史本来就存在用户目录。
; 代价是用户换过安装目录时，旧副本会留在原路径，需要手动卸载。

Section "Kite 主程序" SEC_MAIN
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "..\target\release\kite.exe"
  File "..\THIRD_PARTY_NOTICES.txt"
  SetOutPath "$INSTDIR\resources"
  File "..\resources\Everything64.dll"
  File "..\resources\open.wav"
  SetOutPath "$INSTDIR\resources\official-plugins"
  File /r "..\resources\official-plugins\*"
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
  RMDir /r "$INSTDIR\resources\official-plugins"
  RMDir "$INSTDIR\resources"
  Delete "$INSTDIR\kite.exe"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.txt"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kite"
SectionEnd
