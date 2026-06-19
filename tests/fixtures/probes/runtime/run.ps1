# Reproduce the runtime module-identity probes (P-A, P-B, P-C) against an installed
# v2.1 interpreter. P-C builds an .exe by injecting RCDATA into a copy of the
# interpreter (no Ahk2Exe) and runs it.
#
#   pwsh -File run.ps1
#   pwsh -File run.ps1 -Ahk "C:\Path\to\v2.1\AutoHotkey64.exe"
param(
  [string]$Ahk = "C:\Program Files\AutoHotkey\v2.1\AutoHotkey64.exe"
)
$ErrorActionPreference = "Stop"
$dir = $PSScriptRoot

function ReadFlat($path) {
  if (-not (Test-Path $path)) { return "" }
  $raw = [string](Get-Content $path -Raw)
  return ($raw -replace "`r?`n", " | ").Trim()
}
function Run($label, $exe, $cliArgs) {
  $out = Join-Path $dir "_$label.out"; $err = Join-Path $dir "_$label.err"
  Remove-Item $out, $err -ErrorAction SilentlyContinue
  $p = Start-Process $exe -ArgumentList $cliArgs -Wait -NoNewWindow -PassThru `
        -RedirectStandardOutput $out -RedirectStandardError $err
  $o = ReadFlat $out; $e = ReadFlat $err
  $line = "{0,-4} exit={1}  out=[{2}]" -f $label, $p.ExitCode, $o
  if ($e) { $line += "  err=[$e]" }
  $line
  Remove-Item $out, $err -ErrorAction SilentlyContinue
}

Run "P-A" $Ahk @("/ErrorStdOut", (Join-Path $dir "pa_single.ahk"))
Run "P-B" $Ahk @("/ErrorStdOut", (Join-Path $dir "pb_main.ahk"))

# --- P-C: inject RCDATA into a copy of the interpreter, then run it ---
Add-Type -Namespace ResInj -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("kernel32.dll", SetLastError=true, CharSet=System.Runtime.InteropServices.CharSet.Unicode)]
public static extern System.IntPtr BeginUpdateResourceW(string p, bool del);
[System.Runtime.InteropServices.DllImport("kernel32.dll", SetLastError=true)]
public static extern bool UpdateResourceW(System.IntPtr h, System.IntPtr type, System.IntPtr name, ushort lang, byte[] data, uint cb);
[System.Runtime.InteropServices.DllImport("kernel32.dll", SetLastError=true)]
public static extern bool EndUpdateResourceW(System.IntPtr h, bool discard);
'@
function ScriptBytes($p) { (New-Object System.Text.UTF8Encoding($true)).GetBytes([System.IO.File]::ReadAllText($p)) } # UTF-8 + BOM
$exe = Join-Path $dir "_pc.exe"
Copy-Item $Ahk $exe -Force
$RT_RCDATA = [System.IntPtr]10; $LANG = [uint16]1033
$h = [ResInj.Win32]::BeginUpdateResourceW($exe, $false)
if ($h -eq [System.IntPtr]::Zero) { throw "BeginUpdateResource: $([System.Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
function Add($h, $name, $bytes) {
  if (-not [ResInj.Win32]::UpdateResourceW($h, $RT_RCDATA, $name, $LANG, $bytes, [uint32]$bytes.Length)) {
    throw "UpdateResource: $([System.Runtime.InteropServices.Marshal]::GetLastWin32Error())"
  }
}
$pA = [System.Runtime.InteropServices.Marshal]::StringToHGlobalUni("GROUPA")
$pB = [System.Runtime.InteropServices.Marshal]::StringToHGlobalUni("GROUPB")
try {
  Add $h ([System.IntPtr]1) (ScriptBytes (Join-Path $dir "pc_main.ahk"))   # *#1 main script
  Add $h $pA (ScriptBytes (Join-Path $dir "GroupA.ahk"))
  Add $h $pB (ScriptBytes (Join-Path $dir "GroupB.ahk"))
} finally {
  [System.Runtime.InteropServices.Marshal]::FreeHGlobal($pA)
  [System.Runtime.InteropServices.Marshal]::FreeHGlobal($pB)
}
if (-not [ResInj.Win32]::EndUpdateResourceW($h, $false)) { throw "EndUpdateResource: $([System.Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
Run "P-C" $exe @("/ErrorStdOut")
Remove-Item $exe -ErrorAction SilentlyContinue
