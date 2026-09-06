# Documentation

Kova Image is an early-development Windows viewer for local images, animations
and videos. These documents describe implemented behavior and its current limits.

| Start here | What it covers |
| --- | --- |
| [Project overview](../README.md) | Screenshots, formats, building and controls |
| [Validation record](VALIDATION.md) | Completed checks and remaining coverage gaps |
| [Windows builds and packaging](WINDOWS_RELEASE.md) | Portable builds, dependencies and installer strategy |
| [File associations](FILE_ASSOCIATIONS.md) | Opt-in Open with registration and Windows defaults |
| [Local video](VIDEO.md) | Playback architecture, codec requirements and safety limits |

| Engineering | What it covers |
| --- | --- |
| [Architecture](ARCHITECTURE.md) | Workers, rendering, cancellation and media boundaries |
| [Interface design](DESIGN.md) | Palette, components, spacing and interaction rules |
| [Security architecture](SECURITY_ARCHITECTURE.md) | Resource limits, file handling and known risks |
| [Dependency decisions](DEPENDENCIES.md) | Licensing, runtime costs and maintenance warnings |
| [Performance protocol](PERFORMANCE.md) | Reproducible measurements and test fixtures |
| [Video and compact UI measurements](VIDEO_MEASUREMENTS.md) | Recorded startup, binary size and video CPU/RAM |
| [Earlier UI measurements](UI_MEASUREMENTS.md) | Previous interface comparison |
| [Initial measurements](MEASUREMENTS.md) | Baseline viewer measurements |

For changes, see [Contributing](../CONTRIBUTING.md). For sensitive reports, use
the [Security policy](../SECURITY.md), not a public issue.
