; Installer of the ovc compiler and its language server (Inno Setup 6).
; Built by the release workflow: ISCC.exe /DAppVersion=0.1.0 installer\ovc.iss
; The two binaries are copied next to this script (installer\ovc.exe and installer\ovc-lsp.exe) before the build.
; It installs for the current user only (no administrator rights): the folder is added to the user's PATH.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppName=Open Verilog compiler
AppVersion={#AppVersion}
AppPublisher=Rololow
AppPublisherURL=https://github.com/Rololow/open-verilog
DefaultDirName={localappdata}\Programs\OVC
DefaultGroupName=Open Verilog
DisableProgramGroupPage=yes
OutputDir=out
OutputBaseFilename=ovc-setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
ChangesEnvironment=yes
UninstallDisplayName=Open Verilog compiler {#AppVersion}
LicenseFile=..\LICENSE
WizardStyle=modern

[Files]
Source: "ovc.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "ovc-lsp.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme

[Code]
const
  EnvironmentKey = 'Environment';

// the folder is on the user's PATH already (the match is on whole entries, not on parts of them)
function OnPath(Dir: string; Path: string): Boolean;
begin
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';') > 0;
end;

procedure AddToPath(Dir: string);
var
  Path: string;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Path) then
    Path := '';
  if (Path <> '') and (Path[Length(Path)] <> ';') then
    Path := Path + ';';
  if not OnPath(Dir, Path) then
  begin
    Path := Path + Dir;
    RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Path);
  end;
end;

procedure RemoveFromPath(Dir: string);
var
  Path: string;
begin
  if RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Path) then
  begin
    StringChangeEx(Path, ';' + Dir, '', True);
    StringChangeEx(Path, Dir + ';', '', True);
    StringChangeEx(Path, Dir, '', True);
    RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Path);
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    AddToPath(ExpandConstant('{app}'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath(ExpandConstant('{app}'));
end;
