# GitHub Releases and all currently supported builds

Kordra publishes **pre-alpha releases automatically** from each successful main commit and version-tagged build. Release creation is gated on GitHub-hosted Rust tests, React compilation, all headless builds, both Tauri desktop package builds and multiarch GHCR publishing.

A release includes:
- Windows x64: headless daemon + CLI ZIP, Desktop NSIS EXE, MSI and desktop portable ZIP
- Linux x64: headless daemon + CLI ZIP, Desktop DEB/RPM/AppImage and desktop portable ZIP
- Linux ARM64: headless daemon + CLI ZIP
- `SHA256SUMS.txt` covering release assets
- Docker image: `ghcr.io/kordraproject/kordra:sha-<full-commit-sha>` (linux/amd64 and linux/arm64)
- Kordra-Agent independently publishes its own release ZIPs for Windows x64, Linux x64 and Linux ARM64.

The release refuses to publish if even one required desktop package is missing or fails to build. This is deliberate: we never call incomplete archives a full release.

**Planned, not shipped yet:** Windows ARM64 desktop/headless and Linux ARM64 desktop installers; reliable portable data mode, native Tauri background service/tray, signing, Telegram, TLS and fully featured downloader engines.

On release pushes, existing CI and desktop workflows are invoked as reusable workflows by the unified pipeline to prevent duplicate compilation. No VPS, personal server or self-hosted runner is used in development, build or testing.

## Native multiarch builds

Docker images are compiled on two independent **GitHub-hosted** Ubuntu runners: `ubuntu-24.04` for amd64 and `ubuntu-24.04-arm` for arm64. No QEMU emulation or personal servers are used. The build pushes two immutable commit-qualified platform tags, then combines them via `docker buildx imagetools create`. The release pipeline requires a successful multiarchitecture manifest before publishing release assets. BuildKit cache scopes are isolated by architecture. 
