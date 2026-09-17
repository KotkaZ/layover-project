#requires -Version 7
<#
.SYNOPSIS
    Prepares the Layover wordmark for the README and the documentation site.

.DESCRIPTION
    The supplied artwork is a 1254x1254 transparent PNG with the mark floating in the middle of
    it. Two things have to happen before it can be used:

      * Trim it. Padding baked into an image cannot be controlled by the page using it.
      * Produce a variant that reads on a dark background. The wordmark is near-black navy, which
        is invisible on GitHub's dark theme and on the book's ayu theme. Only the neutral navy is
        lightened; the photographic roundel inside the "O" is left exactly as drawn, because
        lightening a photograph produces a smear rather than a light photograph.

    Pixel work runs in compiled C# over a raw byte array rather than through GetPixel, which would
    take minutes over 1.5 million pixels. The C# deliberately never names a System.Drawing type:
    PowerShell can have two System.Drawing assemblies loaded at once, and passing bitmaps across
    the boundary fails with an unhelpful "Parameter is not valid".

.EXAMPLE
    ./logo-prep.ps1 -Source ~/Designer.png -OutDir assets
#>
param(
    [Parameter(Mandatory = $true)][string]$Source,
    [Parameter(Mandatory = $true)][string]$OutDir
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type -TypeDefinition @'
using System;

public static class LogoPixels
{
    /// Returns left, top, right, bottom of everything drawn. BGRA, bottom byte first.
    public static string ContentBounds(byte[] b, int width, int height, int stride, int minAlpha)
    {
        int minX = width, minY = height, maxX = -1, maxY = -1;

        for (int y = 0; y < height; y++)
            for (int x = 0; x < width; x++)
            {
                if (b[y * stride + x * 4 + 3] < minAlpha) continue;
                if (x < minX) minX = x;
                if (y < minY) minY = y;
                if (x > maxX) maxX = x;
                if (y > maxY) maxY = y;
            }

        if (maxX < 0) return "0,0," + width + "," + height;
        return minX + "," + minY + "," + (maxX + 1) + "," + (maxY + 1);
    }

    /// Bounds of the bright region: the window photograph inside the "O".
    ///
    /// Brightness separates it cleanly where saturation does not. The wordmark is uniformly dark
    /// navy (luminance around 20) and the swoosh is a mid blue; only the sunset in the window is
    /// bright. Rows and columns are counted and thin noise discarded, because a few stray light
    /// pixels along an anti-aliased edge would otherwise stretch the box across the whole mark.
    public static string BrightBounds(byte[] b, int width, int height, int stride,
                                     int minLum, int minAlpha, int minPerLine)
    {
        var perColumn = new int[width];
        var perRow = new int[height];

        for (int y = 0; y < height; y++)
            for (int x = 0; x < width; x++)
            {
                int i = y * stride + x * 4;
                if (b[i + 3] < minAlpha) continue;
                int lum = (b[i + 2] * 299 + b[i + 1] * 587 + b[i] * 114) / 1000;
                if (lum < minLum) continue;
                perColumn[x]++;
                perRow[y]++;
            }

        int minX = -1, maxX = -1, minY = -1, maxY = -1;
        for (int x = 0; x < width; x++)
            if (perColumn[x] >= minPerLine) { if (minX < 0) minX = x; maxX = x; }
        for (int y = 0; y < height; y++)
            if (perRow[y] >= minPerLine) { if (minY < 0) minY = y; maxY = y; }

        if (minX < 0 || minY < 0) return null;
        return minX + "," + minY + "," + (maxX + 1) + "," + (maxY + 1);
    }

    /// Recolours near-neutral pixels outside the keep-box, in place.
    public static void Lift(byte[] b, int width, int height, int stride,
                            int keepL, int keepT, int keepR, int keepB,
                            byte lr, byte lg, byte lb, int maxSpread)
    {
        for (int y = 0; y < height; y++)
            for (int x = 0; x < width; x++)
            {
                int i = y * stride + x * 4;
                if (b[i + 3] == 0) continue;
                if (x >= keepL && x < keepR && y >= keepT && y < keepB) continue;

                int spread = Math.Max(b[i + 2], Math.Max(b[i + 1], b[i]))
                           - Math.Min(b[i + 2], Math.Min(b[i + 1], b[i]));
                if (spread > maxSpread) continue;

                b[i] = lb; b[i + 1] = lg; b[i + 2] = lr;
            }
    }
}
'@

function Get-Pixels {
    # Untyped on purpose: PowerShell can have two System.Drawing assemblies loaded, and a
    # [System.Drawing.Bitmap] annotation may resolve to the other one.
    param($Bitmap)

    $rect = [System.Drawing.Rectangle]::new(0, 0, $Bitmap.Width, $Bitmap.Height)
    $data = $Bitmap.LockBits($rect, 'ReadOnly', 'Format32bppArgb')
    $bytes = [byte[]]::new($data.Stride * $Bitmap.Height)
    [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
    $Bitmap.UnlockBits($data)
    @{ Bytes = $bytes; Stride = $data.Stride }
}

function New-BitmapFrom {
    param([byte[]]$Bytes, [int]$Width, [int]$Height, [int]$Stride)

    $bmp = [System.Drawing.Bitmap]::new($Width, $Height, 'Format32bppArgb')
    $rect = [System.Drawing.Rectangle]::new(0, 0, $Width, $Height)
    $data = $bmp.LockBits($rect, 'WriteOnly', 'Format32bppArgb')
    [System.Runtime.InteropServices.Marshal]::Copy($Bytes, 0, $data.Scan0, $Bytes.Length)
    $bmp.UnlockBits($data)
    $bmp
}

function Get-Bounds {
    <#
    .SYNOPSIS
        Turns a "left,top,right,bottom" string from LogoPixels into four integers.
    #>
    param([string]$Text)

    if ([string]::IsNullOrWhiteSpace($Text)) { return $null }
    $parts = $Text.Split(',')
    [pscustomobject]@{
        Left   = [int]$parts[0]
        Top    = [int]$parts[1]
        Right  = [int]$parts[2]
        Bottom = [int]$parts[3]
        Width  = [int]$parts[2] - [int]$parts[0]
        Height = [int]$parts[3] - [int]$parts[1]
    }
}

$art = [System.Drawing.Bitmap]::new((Resolve-Path $Source).Path)
$pixels = Get-Pixels -Bitmap $art
Write-Host "source:  $($art.Width)x$($art.Height)"

$content = Get-Bounds ([LogoPixels]::ContentBounds($pixels.Bytes, $art.Width, $art.Height, $pixels.Stride, 8))
Write-Host "content: $($content.Left),$($content.Top) -> $($content.Right),$($content.Bottom)  ($($content.Width)x$($content.Height))"

$bright = Get-Bounds ([LogoPixels]::BrightBounds($pixels.Bytes, $art.Width, $art.Height, $pixels.Stride, 170, 24, 6))
if ($null -eq $bright) { throw "could not find the roundel; check the brightness threshold" }

# The bright region is the photograph. The roundel that holds it is a circle whose navy rim is as
# dark as the wordmark, so squaring the box around the photograph's centre is what stops the rim
# being lightened along with the letters.
[int]$centreX = ($bright.Left + $bright.Right) / 2
[int]$centreY = ($bright.Top + $bright.Bottom) / 2
[int]$radius = [Math]::Max($bright.Width, $bright.Height) / 2 + 14
Write-Host "bright:  $($bright.Left),$($bright.Top) -> $($bright.Right),$($bright.Bottom)"
Write-Host "roundel: centre $centreX,$centreY radius $radius"

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

function Save-Variant {
    param($Image, [string]$Name, [int[]]$Widths)

    # A little breathing room, so nothing in the mark touches the edge of the file.
    $pad = 10
    $x = [Math]::Max(0, $content.Left - $pad)
    $y = [Math]::Max(0, $content.Top - $pad)
    $w = [Math]::Min($Image.Width - $x, $content.Width + $pad * 2)
    $h = [Math]::Min($Image.Height - $y, $content.Height + $pad * 2)
    $crop = $Image.Clone([System.Drawing.Rectangle]::new($x, $y, $w, $h), $Image.PixelFormat)

    foreach ($width in $Widths) {
        $height = [int][Math]::Round($crop.Height * ($width / $crop.Width))
        $out = [System.Drawing.Bitmap]::new($width, $height, 'Format32bppArgb')
        $gfx = [System.Drawing.Graphics]::FromImage($out)
        $gfx.InterpolationMode = 'HighQualityBicubic'
        $gfx.PixelOffsetMode = 'HighQuality'
        $gfx.SmoothingMode = 'HighQuality'
        $gfx.Clear([System.Drawing.Color]::Transparent)
        $gfx.DrawImage($crop, [System.Drawing.Rectangle]::new(0, 0, $width, $height))
        $gfx.Dispose()

        $suffix = if ($width -eq $Widths[0]) { '' } else { "-$width" }
        $path = Join-Path $OutDir "$Name$suffix.png"
        $out.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
        $out.Dispose()
        Write-Host ("  {0}  {1}x{2}  {3:n0} bytes" -f $path, $width, $height, (Get-Item $path).Length)
    }

    $crop.Dispose()
}

Save-Variant -Image $art -Name 'logo' -Widths @(880, 440)

# A favicon needs the roundel alone: the wordmark is illegible at 32 pixels, and a favicon that
# cannot be read is just a smudge in a tab.
$mark = $art.Clone(
    [System.Drawing.Rectangle]::new($centreX - $radius, $centreY - $radius, $radius * 2, $radius * 2),
    $art.PixelFormat)
foreach ($size in 512, 180, 64, 32) {
    $icon = [System.Drawing.Bitmap]::new($size, $size, 'Format32bppArgb')
    $gfx = [System.Drawing.Graphics]::FromImage($icon)
    $gfx.InterpolationMode = 'HighQualityBicubic'
    $gfx.PixelOffsetMode = 'HighQuality'
    $gfx.SmoothingMode = 'HighQuality'
    $gfx.Clear([System.Drawing.Color]::Transparent)
    $gfx.DrawImage($mark, [System.Drawing.Rectangle]::new(0, 0, $size, $size))
    $gfx.Dispose()
    $path = Join-Path $OutDir "mark-$size.png"
    $icon.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $icon.Dispose()
    Write-Host ("  {0}  {1}x{1}" -f $path, $size)
}
$mark.Dispose()

$lifted = $pixels.Bytes.Clone()
[LogoPixels]::Lift($lifted, $art.Width, $art.Height, $pixels.Stride,
    ($centreX - $radius), ($centreY - $radius), ($centreX + $radius), ($centreY + $radius),
    0xE6, 0xED, 0xF3, 70)
$darkBitmap = New-BitmapFrom -Bytes $lifted -Width $art.Width -Height $art.Height -Stride $pixels.Stride
Save-Variant -Image $darkBitmap -Name 'logo-dark' -Widths @(880, 440)
$darkBitmap.Dispose()

$art.Dispose()
Write-Host 'done'
