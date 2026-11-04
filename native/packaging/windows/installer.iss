#ifndef AppVersion
  #error Build with scripts\build-installer.ps1.
#endif
#ifndef PayloadDir
  #error PayloadDir is required.
#endif
#ifndef ReleaseDir
  #error ReleaseDir is required.
#endif

[Setup]
AppId={{9981D37B-7783-4723-AE53-477B3371B5EF}
AppName=ClashFx
AppVersion={#AppVersion}
AppPublisher=Yiwu Fanxing Visual Design Co., Ltd.
AppMutex=Local\ClashFx.Installer.9981d37b-7783-4723-ae53-477b3371b5ef
DefaultDirName={localappdata}\Programs\ClashFx
DefaultGroupName=ClashFx
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
LicenseFile=..\..\..\LICENSE
SetupIconFile=..\..\assets\clashfx.ico
WizardStyle=modern
DisableWelcomePage=yes
DisableReadyPage=yes
ShowLanguageDialog=no
LanguageDetectionMethod=uilanguage
OutputDir={#ReleaseDir}
OutputBaseFilename=ClashFx-{#AppVersion}-x64-Setup
Compression=lzma2
SolidCompression=yes
CloseApplications=yes
RestartApplications=no
UninstallDisplayName=ClashFx
UninstallDisplayIcon={app}\ClashFx.exe
VersionInfoVersion={#AppVersion}.0
VersionInfoDescription=ClashFx Setup
VersionInfoProductName=ClashFx
VersionInfoProductVersion={#AppVersion}
SignTool=clashfx
SignedUninstaller=yes
SignedUninstallerDir={#PayloadDir}\signed-uninstaller

[Languages]
Name: "chinesesimp"; MessagesFile: "ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Messages]
chinesesimp.SetupAppRunningError=ClashFx 正在运行。%n%n请从系统托盘退出 ClashFx，再继续安装。
chinesesimp.UninstallAppRunningError=ClashFx 正在运行。%n%n请从系统托盘退出 ClashFx，再继续卸载。

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#PayloadDir}\ClashFx.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\mihomo.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\vcruntime140.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\core-version.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\NOTICE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\USAGE.zh-CN.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\geodata\Country.mmdb"; DestDir: "{app}\geodata"; Flags: ignoreversion
Source: "{#PayloadDir}\geodata\geoip.dat"; DestDir: "{app}\geodata"; Flags: ignoreversion
Source: "{#PayloadDir}\geodata\geosite.dat"; DestDir: "{app}\geodata"; Flags: ignoreversion
Source: "{#PayloadDir}\geodata\ASN.mmdb"; DestDir: "{app}\geodata"; Flags: ignoreversion
Source: "{#PayloadDir}\geodata\manifest.json"; DestDir: "{app}\geodata"; Flags: ignoreversion

[Icons]
Name: "{group}\ClashFx"; Filename: "{app}\ClashFx.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\ClashFx"; Filename: "{app}\ClashFx.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\ClashFx.exe"; Description: "{cm:LaunchProgram,ClashFx}"; Flags: nowait postinstall skipifsilent

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Command: String;
  Executable: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    Executable := '"' + ExpandConstant('{app}\ClashFx.exe') + '"';
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'ClashFx', Command) then
    begin
      if CompareText(Copy(Command, 1, Length(Executable)), Executable) = 0 then
        RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'ClashFx');
    end;
  end;
end;
