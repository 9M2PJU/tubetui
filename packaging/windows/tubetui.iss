; Windows installer for tubetui (Inno Setup 6). Built by the release workflow:
;   iscc /DAppVersion=0.5.0 /DExe=path\to\tubetui.exe /DOutDir=dist packaging\windows\tubetui.iss
; Installs for the current user only (no administrator), adds tubetui to the
; user's PATH and the Start menu, and can install mpv and yt-dlp with winget.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Exe
  #define Exe "..\..\target\release\tubetui.exe"
#endif
#ifndef OutDir
  #define OutDir "..\..\dist"
#endif

[Setup]
AppId={{8C5E2F4B-7D1A-4E3B-9A6C-2B7F0D1E5A93}
AppName=tubetui
AppVersion={#AppVersion}
AppPublisher=polblancoo
AppPublisherURL=https://polblancoo.github.io/tubetui/
AppSupportURL=https://github.com/polblancoo/tubetui/issues
DefaultDirName={autopf}\tubetui
DefaultGroupName=tubetui
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ChangesEnvironment=yes
OutputDir={#OutDir}
OutputBaseFilename=tubetui-x86_64-windows-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayName=tubetui

[Languages]
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
es.Deps=Instalar mpv y yt-dlp con winget (necesarios para ver videos)
en.Deps=Install mpv and yt-dlp with winget (needed to play videos)
es.Installing=Instalando %1 con winget…
en.Installing=Installing %1 with winget…

[Tasks]
Name: "deps"; Description: "{cm:Deps}"

[Files]
Source: "{#Exe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme

[Icons]
Name: "{autoprograms}\tubetui"; Filename: "{app}\tubetui.exe"; WorkingDir: "{userdocs}"

[Run]
Filename: "winget"; Parameters: "install --id mpv-player.mpv-CI.MSVC -e --accept-source-agreements --accept-package-agreements"; StatusMsg: "{cm:Installing,mpv}"; Tasks: deps; Flags: waituntilterminated
Filename: "winget"; Parameters: "install --id yt-dlp.yt-dlp -e --accept-source-agreements --accept-package-agreements"; StatusMsg: "{cm:Installing,yt-dlp}"; Tasks: deps; Flags: waituntilterminated

[Registry]
; The install folder goes on the user's PATH (only once).
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Check: NeedsAddPath(ExpandConstant('{app}'))

[Code]
function NeedsAddPath(Dir: string): Boolean;
var
  Path: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';') = 0;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Path, Dir: string;
  P: Integer;
begin
  if CurUninstallStep <> usPostUninstall then
    exit;
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path) then
    exit;
  Dir := ExpandConstant('{app}');
  P := Pos(';' + Uppercase(Dir), Uppercase(Path));
  if P > 0 then
  begin
    Delete(Path, P, Length(Dir) + 1);
    RegWriteExpandStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path);
  end;
end;
