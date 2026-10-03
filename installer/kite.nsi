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
  ExecWait '"$SYSDIR\taskkill.exe" /F /IM kite-plugin-calculator.exe /IM kite-plugin-window-switcher.exe /IM kite-plugin-devtools.exe /IM kite-plugin-loci.exe'
  Sleep 300
SectionEnd

; 覆盖安装不单独卸载旧版本：InstallDirRegKey 已把默认目录对齐到上一版安装位置，
; 下面的主程序 section 直接就地覆盖文件、快捷方式和卸载器，设置与历史本来就存在用户目录。
; 代价是用户换过安装目录时，旧副本会留在原路径，需要手动卸载。

Section "Kite 主程序" SEC_MAIN
  SectionIn RO
  ; 覆盖升级清理已移除的旧文件搜索资源；不触碰用户目录与插件索引。
  Delete "$INSTDIR\resources\Everything64.dll"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.txt"
  SetOutPath "$INSTDIR"
  File "..\target\release\kite.exe"
  SetOutPath "$INSTDIR\resources"
  File "..\resources\open.wav"
  SetOutPath "$INSTDIR\resources\official-plugins"
  File /r "..\resources\official-plugins\*"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "Software\Kite" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kite" "DisplayName" "Kite"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kite" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kite" "UninstallString" "$INSTDIR\uninstall.exe"
SectionEnd

; 默认选中，可在组件页取消；服务独立于 Kite 与插件进程运行。
Section "Loci 全盘文件索引服务" SEC_LOCI_SERVICE
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=install-loci-service.ps1 "..\resources\loci\install-service.ps1"
  ; 新脚本先核实服务归属，再等待服务正常停止；失败时不得覆盖运行中的文件。
  StrCpy $0 1
  ExecWait '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\install-loci-service.ps1" -ExecutablePath "$INSTDIR\resources\loci\loci-service.exe" -StopOnly' $0
  ${If} $0 != 0
    DetailPrint "Loci 服务检查或停止失败（退出码 $0），未覆盖服务文件。"
    MessageBox MB_ICONSTOP|MB_OK "无法安全更新 Loci 服务。服务可能属于其他安装目录，或停止超时。请检查服务状态后重试。"
    SetErrorLevel 1
    Abort
  ${EndIf}
  SetOutPath "$INSTDIR\resources\loci"
  File "..\resources\loci\loci-service.exe"
  File "..\resources\loci\loci.exe"
  File "..\resources\loci\install-service.ps1"
  StrCpy $0 1
  ExecWait '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\resources\loci\install-service.ps1" -ExecutablePath "$INSTDIR\resources\loci\loci-service.exe"' $0
  ${If} $0 != 0
    DetailPrint "Loci 服务安装或启动失败（退出码 $0），文件搜索尚不可用。"
    MessageBox MB_ICONSTOP|MB_OK "Kite 文件已安装，但 Loci 服务未能启动，文件搜索尚不可用。请检查服务权限或在系统服务中查看错误后重试。"
    SetErrorLevel 1
    Abort
  ${EndIf}
  DetailPrint "Loci 全盘文件索引服务已启动。"
SectionEnd

Section "开始菜单快捷方式" SEC_START
  CreateDirectory "$SMPROGRAMS\Kite"
  CreateShortcut "$SMPROGRAMS\Kite\Kite.lnk" "$INSTDIR\kite.exe" "" "$INSTDIR\kite.exe" 0
SectionEnd

Section "桌面快捷方式" SEC_DESKTOP
  CreateShortcut "$DESKTOP\Kite.lnk" "$INSTDIR\kite.exe" "" "$INSTDIR\kite.exe" 0
SectionEnd

Section "Uninstall"
  ; 脚本只操作 ImagePath 精确归属本安装目录的固定名称服务，不删除第三方服务。
  IfFileExists "$INSTDIR\resources\loci\install-service.ps1" loci_uninstall_service loci_uninstall_no_script
  loci_uninstall_service:
    StrCpy $0 1
    ExecWait '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\resources\loci\install-service.ps1" -ExecutablePath "$INSTDIR\resources\loci\loci-service.exe" -Uninstall' $0
    ${If} $0 != 0
      DetailPrint "Loci 服务卸载失败（退出码 $0），保留安装文件。"
      MessageBox MB_ICONSTOP|MB_OK "Loci 服务未能安全停止或删除，安装文件已保留。请处理服务错误后重新卸载。"
      SetErrorLevel 1
      Abort
    ${EndIf}
    Goto loci_uninstall_done
  loci_uninstall_no_script:
    IfFileExists "$INSTDIR\resources\loci\loci-service.exe" 0 loci_uninstall_done
    MessageBox MB_ICONSTOP|MB_OK "缺少 Loci 服务卸载脚本，无法安全删除服务文件。请重新安装后再卸载。"
    SetErrorLevel 1
    Abort
  loci_uninstall_done:
  Delete "$INSTDIR\resources\loci\loci-service.exe"
  Delete "$INSTDIR\resources\loci\loci.exe"
  Delete "$INSTDIR\resources\loci\install-service.ps1"
  RMDir "$INSTDIR\resources\loci"
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
