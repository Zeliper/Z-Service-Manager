# Generates the tray/app icons (gray, green, yellow, red) as multi-size .ico files.
# Usage: pwsh -File crates/zsm/res/icons/generate.ps1
Add-Type -AssemblyName System.Drawing

$colors = [ordered]@{
    gray   = @(0x80, 0x86, 0x8B)
    green  = @(0x2E, 0xA0, 0x43)
    yellow = @(0xE3, 0xA0, 0x08)
    red    = @(0xD1, 0x24, 0x2F)
}
$sizes = 16, 32, 48

function New-Png([int]$size, [int[]]$rgb) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.TextRenderingHint = 'AntiAliasGridFit'
    $g.Clear([System.Drawing.Color]::Transparent)
    $fill = [System.Drawing.Color]::FromArgb(255, $rgb[0], $rgb[1], $rgb[2])
    $edge = [System.Drawing.Color]::FromArgb(255, [int]($rgb[0] * 0.6), [int]($rgb[1] * 0.6), [int]($rgb[2] * 0.6))
    $pad = [Math]::Max(1, [int]($size / 16))
    $rect = New-Object System.Drawing.Rectangle $pad, $pad, ($size - 2 * $pad - 1), ($size - 2 * $pad - 1)
    $g.FillEllipse((New-Object System.Drawing.SolidBrush $fill), $rect)
    $g.DrawEllipse((New-Object System.Drawing.Pen $edge, ([Math]::Max(1, $size / 24))), $rect)
    $font = New-Object System.Drawing.Font 'Segoe UI', ($size * 0.5), ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Pixel)
    $fmt = New-Object System.Drawing.StringFormat
    $fmt.Alignment = 'Center'
    $fmt.LineAlignment = 'Center'
    $textRect = New-Object System.Drawing.RectangleF 0, ($size * 0.02), $size, $size
    $g.DrawString('Z', $font, [System.Drawing.Brushes]::White, $textRect, $fmt)
    $g.Dispose()
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return , $ms.ToArray()
}

foreach ($name in $colors.Keys) {
    $images = foreach ($s in $sizes) { , (New-Png $s $colors[$name]) }
    $out = New-Object System.IO.MemoryStream
    $w = New-Object System.IO.BinaryWriter $out
    $w.Write([UInt16]0); $w.Write([UInt16]1); $w.Write([UInt16]$sizes.Count)
    $offset = 6 + 16 * $sizes.Count
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $s = $sizes[$i]
        $w.Write([byte]($s % 256)); $w.Write([byte]($s % 256))
        $w.Write([byte]0); $w.Write([byte]0)
        $w.Write([UInt16]1); $w.Write([UInt16]32)
        $w.Write([UInt32]$images[$i].Length); $w.Write([UInt32]$offset)
        $offset += $images[$i].Length
    }
    foreach ($img in $images) { $w.Write($img) }
    $w.Flush()
    [System.IO.File]::WriteAllBytes((Join-Path $PSScriptRoot "$name.ico"), $out.ToArray())
    Write-Output "$name.ico"
}
