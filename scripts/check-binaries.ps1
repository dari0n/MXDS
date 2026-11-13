$bin = 'C:\Users\MINX\Documents\rust\divert\example\bin'
foreach ($f in 'WinDivert.dll', 'WinDivert64.sys', 'winws.exe') {
    $p = Join-Path $bin $f
    $vi = (Get-Item $p).VersionInfo
    $sig = Get-AuthenticodeSignature $p
    $signer = if ($sig.SignerCertificate) { $sig.SignerCertificate.Subject } else { '(none)' }
    '{0} | FileVersion={1} | Product={2} | Sig={3} | Signer={4}' -f $f, $vi.FileVersion, $vi.ProductVersion, $sig.Status, $signer
}
Get-ChildItem 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' |
    ForEach-Object { 'OS: {0} {1} (Build {2}.{3})' -f $_.GetValue('ProductName'), $_.GetValue('DisplayVersion'), $_.GetValue('CurrentBuild'), $_.GetValue('UBR') }
