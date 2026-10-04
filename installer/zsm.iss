; Z Service Manager installer (Inno Setup 6)
; Build: ISCC /DAppVersion=x.y.z installer\zsm.iss   (expects target\release\zsm.exe)

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

#define AppName "Z Service Manager"
#define AppExe "zsm.exe"
#define AppMutexName "Local\ZServiceManager.SingleInstance"
#define RunKey "Software\Microsoft\Windows\CurrentVersion\Run"
#define RunValue "ZServiceManager"

[Setup]
AppId={{76B2B80A-41F8-4E1A-9503-81F3E617D2F2}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Zeliper
AppPublisherURL=https://github.com/Zeliper/Z-Service-Manager
AppSupportURL=https://github.com/Zeliper/Z-Service-Manager/issues
AppUpdatesURL=https://github.com/Zeliper/Z-Service-Manager/releases
DefaultDirName={localappdata}\Programs\ZServiceManager
DisableDirPage=yes
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
AppMutex={#AppMutexName}
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=Output
OutputBaseFilename=ZServiceManager-Setup-{#AppVersion}
SetupIconFile=..\crates\zsm\res\icons\green.ico
UninstallDisplayIcon={app}\{#AppExe}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
LicenseFile=..\LICENSE
CloseApplications=no
ShowLanguageDialog=auto

[Languages]
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Windows 시작 시 실행"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\{#AppName} 제거"; Filename: "{uninstallexe}"

[Registry]
Root: HKCU; Subkey: "{#RunKey}"; ValueType: string; ValueName: "{#RunValue}"; ValueData: """{app}\{#AppExe}"" --tray"; Tasks: autostart; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#AppExe}"; Parameters: "--tray --resume"; Flags: nowait; Check: IsZsmUpdate
Filename: "{app}\{#AppExe}"; Description: "{#AppName} 실행"; Flags: nowait postinstall skipifsilent; Check: not IsZsmUpdate

[Code]
function IsZsmUpdate: Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/ZSMUPDATE') = 0 then
      Result := True;
end;

{ In update mode the running app exits right after launching us; give it time to release the mutex. }
function InitializeSetup: Boolean;
var
  Waited: Integer;
begin
  if IsZsmUpdate then
  begin
    Waited := 0;
    while CheckForMutexes('{#AppMutexName}') and (Waited < 60000) do
    begin
      Sleep(500);
      Waited := Waited + 500;
    end;
  end;
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RegDeleteValue(HKEY_CURRENT_USER, '{#RunKey}', '{#RunValue}');
  if CurUninstallStep = usPostUninstall then
  begin
    if SuppressibleMsgBox('설정과 로그도 삭제할까?' + #13#10 + ExpandConstant('{userappdata}\ZServiceManager') + #13#10 +
              ExpandConstant('{localappdata}\ZServiceManager'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
    begin
      DelTree(ExpandConstant('{userappdata}\ZServiceManager'), True, True, True);
      DelTree(ExpandConstant('{localappdata}\ZServiceManager'), True, True, True);
    end;
  end;
end;
