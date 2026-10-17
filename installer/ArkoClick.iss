; ArkoClick installer (Inno Setup 6).
; Build:  "D:\programs\InnoSetup6\ISCC.exe" installer\ArkoClick.iss
; Output: dist\ArkoClick-Setup.exe
;
; Per-user install (no administrator rights, no UAC prompt), English + Arabic wizard,
; Start-menu and optional desktop shortcuts, optional "start with Windows", clean uninstall.

#define AppName "ArkoClick"
#define AppVersion "0.4.0"
#define AppPublisher "Arkam Rashed"
#define AppExe "ArkoClick.exe"
#define AppContact "arko313m@gmail.com"
#define AppSupport "https://wa.me/201008446123"

[Setup]
AppId={{A3DAFB37-CC6B-4986-A377-3B3419A873AD}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppContact={#AppContact}
AppPublisherURL={#AppSupport}
AppSupportURL={#AppSupport}
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\dist
; Version-less name so README links to releases/latest/download/ArkoClick-Setup.exe keep working.
OutputBaseFilename=ArkoClick-Setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
; Fewer pages for non-technical users: no folder choice, no "ready to install" summary.
DisableDirPage=yes
DisableReadyPage=yes
ShowLanguageDialog=auto
VersionInfoVersion={#AppVersion}.0
VersionInfoCompany={#AppPublisher}
VersionInfoDescription={#AppName} Setup
VersionInfoCopyright=Copyright (C) 2026 {#AppPublisher}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"; InfoAfterFile: "guide-en.txt"
Name: "arabic"; MessagesFile: "compiler:Languages\Arabic.isl"; InfoAfterFile: "guide-ar.txt"

[CustomMessages]
english.TestPage=ArkoClick test page
arabic.TestPage=صفحة تجربة ArkoClick
english.StartWithWindows=Start ArkoClick when Windows starts
arabic.StartWithWindows=تشغيل ArkoClick مع بدء تشغيل ويندوز
english.ExtraTasks=Extra options:
arabic.ExtraTasks=خيارات إضافية:
english.CloseApp=ArkoClick is running. Close it (right-click its icon near the clock, then Exit) and press OK.
arabic.CloseApp=برنامج ArkoClick يعمل الآن. أغلقه (انقر بزر الماوس الأيمن على أيقونته بجوار الساعة ثم خروج) واضغط موافق.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "autostart"; Description: "{cm:StartWithWindows}"; GroupDescription: "{cm:ExtraTasks}"; Flags: unchecked

[Files]
Source: "..\dist\ArkoClick.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\test-target\ArkoClick-TestTarget.html"; DestDir: "{app}"; Flags: ignoreversion
Source: "guide-ar.txt"; DestDir: "{app}"; DestName: "اقرأني.txt"; Flags: ignoreversion
Source: "guide-en.txt"; DestDir: "{app}"; DestName: "ReadMe.txt"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\{cm:TestPage}"; Filename: "{app}\ArkoClick-TestTarget.html"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; Same value the app itself uses for its "start with Windows" switch, so both stay in sync.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "ArkoClick"; ValueData: """{app}\{#AppExe}"" --autostart"; Tasks: autostart

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
; Started by ArkoClick's updater (/UPDATE=1): reopen the app when done.
Filename: "{app}\{#AppExe}"; Flags: nowait; Check: IsUpdateRun

[Code]
function IsUpdateRun: Boolean;
begin
  Result := ExpandConstant('{param:UPDATE|0}') = '1';
end;

// The app holds this mutex while running (see src/main.rs). When the updater starts setup,
// the app is quitting, so wait a few seconds for it before asking the user.
function InitializeSetup: Boolean;
var
  Tries: Integer;
begin
  Result := True;
  Tries := 0;
  while CheckForMutexes('ArkoClick.SingleInstance') and (Tries < 40) do
  begin
    Sleep(250);
    Tries := Tries + 1;
  end;
  while CheckForMutexes('ArkoClick.SingleInstance') do
  begin
    if SuppressibleMsgBox(CustomMessage('CloseApp'), mbError, MB_OKCANCEL, IDCANCEL) = IDCANCEL then
    begin
      Result := False;
      Exit;
    end;
  end;
end;

// The app can turn "start with Windows" on by itself, so always remove that entry on
// uninstall, not only when the installer created it.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RegDeleteValue(HKEY_CURRENT_USER, 'Software\Microsoft\Windows\CurrentVersion\Run', 'ArkoClick');
end;
