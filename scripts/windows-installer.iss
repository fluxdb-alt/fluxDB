; Windows 安装包由 tag 触发的 release workflow 构建；当前未做代码签名。
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
SetupIconFile=..\apps\fluxdb-desktop\assets\app-icon.ico
UninstallDisplayIcon={app}\fluxdb-desktop.exe

[Files]
Source: "..\target\x86_64-pc-windows-msvc\release\fluxdb-desktop.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\apps\fluxdb-desktop\assets\*"; DestDir: "{app}\assets"; Flags: ignoreversion recursesubdirs createallsubdirs

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: checkedonce

[Icons]
Name: "{group}\FluxDB"; Filename: "{app}\fluxdb-desktop.exe"
Name: "{autodesktop}\FluxDB"; Filename: "{app}\fluxdb-desktop.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\fluxdb-desktop.exe"; Description: "Launch FluxDB"; Flags: nowait postinstall skipifsilent
