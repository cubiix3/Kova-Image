# Security policy

## Supported versions

Kova Image 0.1.0 is **Early Development**. Security fixes target the current
`main` branch. No stable release or long-term support commitment exists yet.
Use the newest reviewed source; do not assume older preview builds are supported.

## Reporting a vulnerability

Use GitHub's **private vulnerability reporting**:
[Report a vulnerability](https://github.com/cubiix3/Kova-Image/security/advisories/new).
Please do not open a public issue with sensitive exploit details, weaponized
media files or personal data. If private reporting is unavailable, open a
minimal issue asking the maintainer for a private contact channel, without
including the exploit or sample.

Include the affected commit/version, Windows version, reproduction steps,
expected impact and a minimal sample when it is safe to share privately.
Please allow time for investigation and a coordinated fix before disclosure.
No response-time guarantee is made during early development.

## Scope and boundaries

Every opened JPEG, PNG/APNG, GIF, WebP, BMP, TIFF, ICO and MP4/MOV/WebM/MKV
video is **untrusted input**.
Decoder failures, parser bugs, overflow, memory exhaustion, animation corruption,
file races and unintended file changes are security-relevant. Unsupported
formats are rejected; SVG, external resources and scripts are never executed.
There is no application networking, telemetry or automatic upload path.

Input size, dimensions, decoded animation storage, cache size and request queues
are bounded. Decoding runs on a worker and Rust decoder panics are caught.
These measures are **not process isolation**: allocator aborts, native faults,
codec scratch allocations and GPU memory remain limitations. Read
[security architecture](docs/SECURITY_ARCHITECTURE.md) before assessing claims.

Native video parsing uses installed Windows Media Foundation components. Local
streams are admitted without user URLs; external MP4/MOV data references and
compressed/reference movies are rejected. No codecs are downloaded. Native
codec faults and allocations are not isolated by Rust panic recovery. See
[video boundaries](docs/VIDEO.md) when reporting playback or parser problems.
