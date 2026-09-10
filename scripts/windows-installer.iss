; Windows 流程暂未启用；需在 Windows 验证动态库依赖后发布。
#ifndef AppVersion
  #error AppVersion must be supplied by the release workflow
#endif

[Setup]
AppId=com.shining3d.fluxdb
AppName=FluxDB
AppVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\FluxDB
DefaultGroupName=FluxDB
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\windows-package
OutputBaseFilename=FluxDB-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
UninstallDisplayIcon={app}\fluxdb-desktop.exe

[Files]
Source: "..\target\x86_64-pc-windows-msvc\release\fluxdb-desktop.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\apps\fluxdb-desktop\assets\*"; DestDir: "{app}\assets"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\FluxDB"; Filename: "{app}\fluxdb-desktop.exe"

[Run]
Filename: "{app}\fluxdb-desktop.exe"; Description: "Launch FluxDB"; Flags: nowait postinstall skipifsilent
