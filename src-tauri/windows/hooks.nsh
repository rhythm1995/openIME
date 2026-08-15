; R11 阶段 A：TSF TIP DLL 的 per-user（HKCU）注册/反注册。
; 设计 FR-11.2：不写 HKLM、无 UAC；DllRegisterServer/DllUnregisterServer 只动 HKCU。
; DLL 被锁（升级时进程占用）的场景由宿主下次启动的自检自注册兜底，这里尽力而为。
!macro NSIS_HOOK_POSTINSTALL
  ExecWait '"$SYSDIR\regsvr32.exe" /s "$INSTDIR\resources\ime\OpenImeTsf.dll"'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ExecWait '"$SYSDIR\regsvr32.exe" /u /s "$INSTDIR\resources\ime\OpenImeTsf.dll"'
!macroend
