# Format coverage and limits

The file picker, folder navigation, Windows registration and thumbnail provider
use the image extension list in `src/format.rs`. Content identifies the image;
TGA's signatureless header and camera RAW containers need additional checks.
Every viewer image decoder runs inside Kova Image with its decode worker and
resource limits. The viewer does not invoke external conversion tools.

| Family | Supported viewing behavior | Remaining limits |
| --- | --- | --- |
| JPEG, PNG/APNG, GIF, WebP | Still images; APNG/GIF/WebP animation; JPEG orientation and ICC conversion | Damaged files and images beyond configured pixel/frame/input limits are rejected |
| BMP, ICO, TIFF | BMP and ICO; first TIFF page | CUR, bare DIB and later TIFF pages are not advertised |
| TGA/TARGA/ICB/VDA/VST | Uncompressed/RLE, palette and grayscale; image origins; declared alpha and TGA 2 alpha attributes | Unsupported interleaving/layouts return an error |
| DDS | BC1–BC5/BC7, RGB/RGBA masks, luminance/alpha, palette, R16/RG16/RGBA16 UNORM; row pitch and alpha attributes | BC6H, signed/float textures, later mip levels, cube faces and array layers are not shown |
| JPEG XL | Still images and bounded animation; sRGB conversion | Large/long animations can reach the retained-frame budget |
| AVIF | AV1 still images, alpha, grids and orientation | Animated sequences are not shown; after a caught decoder panic AVIF remains unavailable until restart to bound leaked native state |
| HEIC/HEIF/HIF | HEVC intra still pictures, alpha, grids and orientation | Inter prediction, several tiles and rarer range-extension tools remain unsupported |
| Camera RAW | Embedded full-size JPEG preview, camera metadata and orientation | Files without a usable embedded JPEG preview; raw sensor development |
| SVG/SVGZ | Bounded rasterization at the requested viewing size, embedded data images | External files, network resources and scripts are disabled |
| PNM/PAM, QOI, HDR, EXR, farbfeld | Still images; float values converted for display | No editing or export; viewing limits still apply |
| MP4/M4V, MOV, WebM, MKV | Bundled FFmpeg/dav1d video decoding, WASAPI audio, seek/pause/loop | See [video coverage](VIDEO.md); some codecs/containers, HDR tone mapping and hostile native-code faults remain outside the contract |
| Audio | MP3, M4A/M4B, AAC, WAV, FLAC, OGG/OGA, Opus, WMA | Audio-only playback depends on available Windows codecs |

JPEG's JFIF/JIF names and TGA/HEIF aliases use their existing decoders; adding
an alias also adds navigation, file-picker and Windows registration coverage.
Aliases do not increase the number of distinct codec implementations.

## Comparison reference

[ImageGlass documents](https://imageglass.org/docs/supported-formats) a broader
set through Magick.NET, SkiaSharp, Svg.Skia and optional codec plugins. Its list
includes PSD/PSB, JPEG 2000, JPEG XR, PCX, FITS, XBM/XPM, WMF/EMF and other
formats that Kova currently does not decode. These remain explicit gaps rather
than file associations that lead to a missing decoder.

ImageGlass's [repository license](https://github.com/d2phap/ImageGlass/blob/develop/LICENSE)
is GPLv3. Kova's fixes are original implementations using its existing reviewed
dependencies. Broad new codec engines require a separate license, allocation,
failure and packaging review under [the dependency policy](DEPENDENCIES.md).
