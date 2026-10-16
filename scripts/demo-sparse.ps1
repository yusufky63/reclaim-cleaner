param([Parameter(Mandatory = $true)][string]$Manifest)
# Creates the files of a demo profile manifest (scripts/demo-profile.mjs) as empty sparse files
# with the given size and time, then dates each project folder tree.
$ErrorActionPreference = "Stop"
Add-Type -TypeDefinition @"
using System;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class DemoSparse {
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool DeviceIoControl(SafeFileHandle h, uint code, IntPtr inBuf, uint inSize, IntPtr outBuf, uint outSize, out uint ret, IntPtr ov);
  const uint FSCTL_SET_SPARSE = 0x000900C4;
  public static void Make(string path, long size, long unixMs) {
    Directory.CreateDirectory(Path.GetDirectoryName(path));
    using (var fs = new FileStream(path, FileMode.Create, FileAccess.ReadWrite, FileShare.None)) {
      uint ret;
      if (!DeviceIoControl(fs.SafeFileHandle, FSCTL_SET_SPARSE, IntPtr.Zero, 0, IntPtr.Zero, 0, out ret, IntPtr.Zero))
        throw new IOException("FSCTL_SET_SPARSE failed: " + Marshal.GetLastWin32Error());
      fs.SetLength(size);
    }
    var t = DateTimeOffset.FromUnixTimeMilliseconds(unixMs).UtcDateTime;
    File.SetCreationTimeUtc(path, t);
    File.SetLastWriteTimeUtc(path, t);
    File.SetLastAccessTimeUtc(path, t);
  }
}
"@
$data = Get-Content -Raw -Encoding UTF8 $Manifest | ConvertFrom-Json
foreach ($f in $data.files) { [DemoSparse]::Make([string]$f[0], [long]$f[1], [long]$f[2]) }
foreach ($d in $data.dirTimes) {
  $t = [DateTimeOffset]::FromUnixTimeMilliseconds([long]$d[1]).LocalDateTime
  # Deepest first, so setting a child's time does not bump its parent again.
  $dirs = @(Get-Item -LiteralPath ([string]$d[0])) + @(Get-ChildItem -LiteralPath ([string]$d[0]) -Recurse -Directory -Force)
  $dirs | Sort-Object { $_.FullName.Length } -Descending | ForEach-Object { $_.CreationTime = $t; $_.LastWriteTime = $t }
}
"created $($data.files.Count) sparse files"
