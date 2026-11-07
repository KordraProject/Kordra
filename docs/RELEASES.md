# Releases and pre-alpha artifacts

No personal servers, VPS or self-hosted runners are used. All packages, Docker images and tests are built exclusively on GitHub-hosted Actions runners.

- Every main push runs the Rust CI checks and creates temporary **headless** artifacts for Windows x64, Linux x64 and Linux arm64.
- Tags beginning with `v` create pre-release GitHub Releases with those headless archives after successful build.
- Docker publishes `ghcr.io/kordraproject/kordra:edge` on main and a versioned tag on a tagged release, building for linux/amd64 and linux/arm64 with Buildx/QEMU.
- A separate Docker CI workflow performs dry-run builds on main without pushing.
- Kordra-Agent has independent CI, artifacts and release workflow.

**Important:** desktop installers (.msi/.exe/AppImage/.deb/.rpm), portable GUI archives, the Telegram bot and advanced downloader integrations have not been implemented. An installed Docker image exposes the pre-alpha Web UI and API but native public HTTPS is not yet ready.

GitHub Packages may require the organization owner to set the GHCR package visibility to public after its first publish.
