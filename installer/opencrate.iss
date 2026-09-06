; Build with scripts/build-installer.ps1. Product identity stays stable across upgrades.
#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef PayloadDir
  #error PayloadDir is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif

[Setup]
AppId={{E83E52E6-E77A-4C03-B131-21AEBC26DDE9}
AppName=OpenCrate
AppVersion={#AppVersion}
AppVerName=OpenCrate {#AppVersion}
AppPublisher=OpenCrate contributors
DefaultDirName={localappdata}\Programs\OpenCrate
DefaultGroupName=OpenCrate
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
MinVersion=10.0.17763
OutputDir={#OutputDir}
OutputBaseFilename=OpenCrate-{#AppVersion}-windows-x64-setup
SetupIconFile=..\assets\branding\opencrate-icon.ico
WizardSmallImageFile=..\assets\branding\opencrate-icon-48.png
UninstallDisplayIcon={app}\OpenCrate.exe
WizardStyle=modern dark
WizardSizePercent=110
Compression=lzma2/max
SolidCompression=yes
AppMutex=Local\opencrate.ui.v1
SetupMutex=Local\opencrate.setup.v1
CloseApplications=no
RestartApplications=no
UsePreviousLanguage=yes
ShowLanguageDialog=yes
DisableWelcomePage=no
Uninstallable=yes
UninstallLogging=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl,locales\en.isl"; InfoBeforeFile: "locales\before-en.txt"
Name: "zh_CN"; MessagesFile: "languages\ChineseSimplified.isl,locales\zh-CN.isl"; InfoBeforeFile: "locales\before-zh-CN.txt"
Name: "tr"; MessagesFile: "compiler:Languages\Turkish.isl,locales\tr.isl"; InfoBeforeFile: "locales\before-tr.txt"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\OpenCrate.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\OpenCrate"; Filename: "{app}\OpenCrate.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\OpenCrate"; Filename: "{app}\OpenCrate.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\OpenCrate.exe"; Description: "{cm:LaunchProgram,OpenCrate}"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent

[Code]
const
  StartupKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  StartupValue = 'opencrate';

function InstalledStartupCommand: String;
begin
  Result := '"' + ExpandConstant('{app}\OpenCrate.exe') + '" --startup';
end;

function IsOpenCrateStartup(const Command: String): Boolean;
var
  ClosingQuote: Integer;
  Executable, Arguments: String;
begin
  Result := False;
  if (Length(Command) = 0) or (Command[1] <> '"') then Exit;
  ClosingQuote := Pos('"', Copy(Command, 2, Length(Command)));
  if ClosingQuote = 0 then Exit;
  Executable := Copy(Command, 2, ClosingQuote - 1);
  Arguments := Trim(Copy(Command, ClosingQuote + 2, Length(Command)));
  Result := (CompareText(Arguments, '--startup') = 0) and
    ((CompareText(ExtractFileName(Executable), 'OpenCrate.exe') = 0) or
     (CompareText(ExtractFileName(Executable), 'opencrate-ui.exe') = 0));
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ExistingCommand: String;
begin
  if CurStep = ssPostInstall then begin
    // Preserve an enabled startup preference when moving from a portable build.
    // First-time installs never enable startup on the user's behalf.
    if RegQueryStringValue(HKCU, StartupKey, StartupValue, ExistingCommand) and
       IsOpenCrateStartup(ExistingCommand) then begin
      if not RegWriteStringValue(HKCU, StartupKey, StartupValue, InstalledStartupCommand) then
        Log('Could not update the existing OpenCrate startup command.');
    end;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ExistingCommand: String;
begin
  if CurUninstallStep = usUninstall then begin
    // Do not remove a startup entry that now points to another copy of the app.
    if RegQueryStringValue(HKCU, StartupKey, StartupValue, ExistingCommand) and
       (CompareText(ExistingCommand, InstalledStartupCommand) = 0) then
      RegDeleteValue(HKCU, StartupKey, StartupValue);
  end;
end;
