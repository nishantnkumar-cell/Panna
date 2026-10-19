; The Windows installer, built with Inno Setup 6 by CI:
;   ISCC /DAppVersion=0.1.0 /DArch=x64compatible /DExe=path\to\panna.exe /DName=windows-x86_64 panna.iss
; Arch is x64compatible or arm64.
;
; It installs for the current user without asking for administrator rights (or for everyone,
; if chosen), and registers Panna as an app that opens PDFs, so it's listed under Open with and
; in Settings > Default apps. Windows lets only the user choose the default PDF app, so the
; last page offers to open that setting at Panna's entry.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Arch
  #define Arch "x64compatible"
#endif
#ifndef Exe
  #define Exe "panna.exe"
#endif
#ifndef Name
  #define Name "windows"
#endif

[Setup]
AppId={{F4FA6D61-8327-491D-9ABC-E53AA5B68712}
AppName=Panna
AppVersion={#AppVersion}
AppVerName=Panna {#AppVersion}
AppPublisher=Nishant
AppComments=View, annotate and fill in PDF files, offline.
DefaultDirName={autopf}\Panna
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed={#Arch}
ArchitecturesInstallIn64BitMode={#Arch}
ChangesAssociations=yes
OutputDir=.
OutputBaseFilename=panna-{#Name}-setup
SetupIconFile=panna.ico
UninstallDisplayIcon={app}\panna.exe
UninstallDisplayName=Panna
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Files]
Source: "{#Exe}"; DestDir: "{app}"; DestName: "panna.exe"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Panna"; Filename: "{app}\panna.exe"

[Registry]
; How Panna opens a PDF.
Root: HKA; Subkey: "Software\Classes\Panna.PDF"; ValueType: string; ValueData: "PDF Document"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Panna.PDF\DefaultIcon"; ValueType: string; ValueData: "{app}\panna.exe,0"
Root: HKA; Subkey: "Software\Classes\Panna.PDF\shell\open\command"; ValueType: string; ValueData: """{app}\panna.exe"" ""%1"""
; Offered for PDFs under Open with.
Root: HKA; Subkey: "Software\Classes\.pdf\OpenWithProgids"; ValueType: string; ValueName: "Panna.PDF"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\panna.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Panna"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\panna.exe\SupportedTypes"; ValueType: string; ValueName: ".pdf"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\panna.exe\shell\open\command"; ValueType: string; ValueData: """{app}\panna.exe"" ""%1"""
; Listed in Settings > Default apps.
Root: HKA; Subkey: "Software\Panna"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Panna\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "Panna"
Root: HKA; Subkey: "Software\Panna\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "View, annotate and fill in PDF files, offline."
Root: HKA; Subkey: "Software\Panna\Capabilities\FileAssociations"; ValueType: string; ValueName: ".pdf"; ValueData: "Panna.PDF"
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "Panna"; ValueData: "Software\Panna\Capabilities"; Flags: uninsdeletevalue
; So "panna" works from the Run box.
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\panna.exe"; ValueType: string; ValueData: "{app}\panna.exe"; Flags: uninsdeletekey

[Run]
Filename: "ms-settings:defaultapps?registeredAppUser=Panna"; Description: "Make Panna the default PDF app (opens Settings)"; Flags: postinstall shellexec skipifsilent nowait; Check: not IsAdminInstallMode
Filename: "ms-settings:defaultapps?registeredAppMachine=Panna"; Description: "Make Panna the default PDF app (opens Settings)"; Flags: postinstall shellexec skipifsilent nowait; Check: IsAdminInstallMode
Filename: "{app}\panna.exe"; Description: "Open Panna"; Flags: postinstall nowait skipifsilent unchecked
