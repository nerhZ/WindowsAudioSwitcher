# Regenerates the icon set from Flaticon source PNGs (upnow-graphic) in
# src-tauri/icons/source/ (16.png, 24.png, 32.png required; 64, 128, 256, 512
# recommended). Every native size lands in the .ico as-is; only the sizes
# Flaticon doesn't offer (20, 48) are derived.
# Produces:
#   - icon.ico           multi-size (16, 20, 24, 32, 48, 64, 128, 256) for
#                        Explorer, taskbar and the MSI - Windows picks the
#                        right frame per DPI (256 is the ICO format cap)
#   - 32x32.png          tray icon source (crisp 1:1 at 200% display scale)
#   - 128x128.png, 128x128@2x.png  installer/asset sizes
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Drawing

$iconsDir = (Resolve-Path "src-tauri\icons").Path
$sourceDir = Join-Path $iconsDir "source"

foreach ($size in 16, 24, 32) {
    if (-not (Test-Path (Join-Path $sourceDir "$size.png"))) {
        Write-Error "Missing source icon: $sourceDir\$size.png - download the $size px PNG from https://www.flaticon.com/authors/upnow-graphic and re-run."
    }
}
$masterLarge = if (Test-Path (Join-Path $sourceDir "512.png")) { "512.png" } else { "32.png" }

function Get-ScaledPng {
    param([int]$Size, [string]$SourcePath)
    $src = [System.Drawing.Bitmap]::FromFile($SourcePath)
    try {
        $out = [System.Drawing.Bitmap]::new($Size, $Size)
        $g = [System.Drawing.Graphics]::FromImage($out)
        try {
            $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
            $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $g.Clear([System.Drawing.Color]::Transparent)
            $g.DrawImage($src, 0, 0, $Size, $Size)
        } finally {
            $g.Dispose()
        }
        $ms = [System.IO.MemoryStream]::new()
        $out.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        return $ms.ToArray()
    } finally {
        $src.Dispose()
    }
}

# Native source file for a frame size when Flaticon provides it, else the
# upscale master. 20 and 48 are always derived (24 and 64 are their closest
# native sources).
function Get-FrameSource {
    param([int]$Size)
    if ($Size -eq 20) { return (Join-Path $sourceDir "24.png") }
    if ($Size -eq 48) { return (Join-Path $sourceDir "64.png") }
    $native = Join-Path $sourceDir "$Size.png"
    if (Test-Path $native) { return $native }
    return (Join-Path $sourceDir $masterLarge)
}

$frames = [System.Collections.Generic.List[object]]::new()
foreach ($size in 16, 20, 24, 32, 48, 64, 128, 256) {
    $frames.Add(@{ Size = $size; Bytes = (Get-ScaledPng -Size $size -SourcePath (Get-FrameSource -Size $size)) })
}

# Pack PNG-encoded frames into an ICO container (PNG compression in ICO is
# supported on Windows Vista+).
function New-Ico {
    param([System.Collections.Generic.List[object]]$Frames, [string]$OutPath)
    $count = $Frames.Count
    $header = [byte[]]::new(6 + 16 * $count)
    $header[0] = 0; $header[1] = 0   # reserved
    $header[2] = 1; $header[3] = 0   # type: icon
    [BitConverter]::GetBytes([uint16]$count).CopyTo($header, 4)
    $offset = 6 + 16 * $count
    $cursor = 6
    foreach ($f in $Frames) {
        $w = if ($f.Size -eq 256) { 0 } else { $f.Size }   # 0 = 256
        $header[$cursor] = [byte]$w
        $header[$cursor + 1] = [byte]$w
        $header[$cursor + 2] = 0; $header[$cursor + 3] = 0  # colors, reserved
        [BitConverter]::GetBytes([uint16]1).CopyTo($header, $cursor + 4)   # planes
        [BitConverter]::GetBytes([uint16]32).CopyTo($header, $cursor + 6)  # bpp
        [BitConverter]::GetBytes([uint32]$f.Bytes.Length).CopyTo($header, $cursor + 8)
        [BitConverter]::GetBytes([uint32]$offset).CopyTo($header, $cursor + 12)
        $offset += $f.Bytes.Length
        $cursor += 16
    }
    $stream = [System.IO.MemoryStream]::new()
    try {
        $stream.Write($header, 0, $header.Length)
        foreach ($f in $Frames) { $stream.Write($f.Bytes, 0, $f.Bytes.Length) }
        [System.IO.File]::WriteAllBytes($OutPath, $stream.ToArray())
    } finally {
        $stream.Dispose()
    }
}

New-Ico -Frames $frames -OutPath (Join-Path $iconsDir "icon.ico")

[System.IO.File]::WriteAllBytes((Join-Path $iconsDir "32x32.png"), (Get-ScaledPng -Size 32 -SourcePath (Join-Path $sourceDir "32.png")))
[System.IO.File]::WriteAllBytes((Join-Path $iconsDir "128x128.png"), (Get-ScaledPng -Size 128 -SourcePath (Get-FrameSource -Size 128)))
[System.IO.File]::WriteAllBytes((Join-Path $iconsDir "128x128@2x.png"), (Get-ScaledPng -Size 256 -SourcePath (Get-FrameSource -Size 256)))

Write-Host "Icons regenerated. Rebuild with scripts/build.ps1 to apply."
