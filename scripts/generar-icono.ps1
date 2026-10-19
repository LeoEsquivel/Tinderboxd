# Genera assets/icon.png (256 px) y assets/icon.ico (16-256 px) con el logo de
# Tinderboxd: tres círculos (naranja, verde, azul) sobre el fondo oscuro de Letterboxd.
#
#   powershell -ExecutionPolicy Bypass -File scripts/generar-icono.ps1

Add-Type -AssemblyName System.Drawing
$assets = Join-Path $PSScriptRoot "..\assets"
New-Item -ItemType Directory -Force $assets | Out-Null

function New-IconBitmap([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)

    # Fondo: cuadrado redondeado #14181c.
    $r = [Math]::Max(2, [int]($size * 0.22))
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.AddArc(0, 0, $r * 2, $r * 2, 180, 90)
    $path.AddArc($size - $r * 2 - 1, 0, $r * 2, $r * 2, 270, 90)
    $path.AddArc($size - $r * 2 - 1, $size - $r * 2 - 1, $r * 2, $r * 2, 0, 90)
    $path.AddArc(0, $size - $r * 2 - 1, $r * 2, $r * 2, 90, 90)
    $path.CloseFigure()
    $g.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 0x14, 0x18, 0x1c))), $path)

    # Tres círculos que se solapan un poco, como el logo de Letterboxd.
    $d = $size * 0.30
    $step = $d * 0.86
    $x0 = ($size - ($d + 2 * $step)) / 2
    $y = ($size - $d) / 2
    $colors = @(
        [System.Drawing.Color]::FromArgb(255, 0xff, 0x80, 0x00),
        [System.Drawing.Color]::FromArgb(255, 0x00, 0xe0, 0x54),
        [System.Drawing.Color]::FromArgb(255, 0x40, 0xbc, 0xf4)
    )
    for ($i = 0; $i -lt 3; $i++) {
        $g.FillEllipse((New-Object System.Drawing.SolidBrush $colors[$i]), [float]($x0 + $i * $step), [float]$y, [float]$d, [float]$d)
    }
    $g.Dispose()
    return $bmp
}

function Get-PngBytes($bmp) {
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    return , $ms.ToArray()
}

# PNG grande (ventana de la GUI e instaladores).
$big = New-IconBitmap 256
$big.Save((Join-Path $assets "icon.png"), [System.Drawing.Imaging.ImageFormat]::Png)

# ICO con imágenes PNG embebidas (formato válido desde Windows Vista).
$sizes = @(16, 24, 32, 48, 64, 128, 256)
$images = foreach ($s in $sizes) { , (Get-PngBytes (New-IconBitmap $s)) }
$ms = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $ms
$w.Write([UInt16]0); $w.Write([UInt16]1); $w.Write([UInt16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]
    $dim = if ($s -ge 256) { 0 } else { $s }
    $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([UInt16]1); $w.Write([UInt16]32)
    $w.Write([UInt32]$images[$i].Length); $w.Write([UInt32]$offset)
    $offset += $images[$i].Length
}
foreach ($img in $images) { $w.Write($img) }
$w.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $assets "icon.ico"), $ms.ToArray())
"Generados assets/icon.png y assets/icon.ico"
