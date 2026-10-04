# Releases

Terra uses one repository version and `v`-prefixed tags.

The [release workflow](../.github/workflows/release.yaml) runs release-please on
pushes to `main` or manual dispatch from `main`. Conventional Commits drive a
release PR updating `CHANGELOG.md`, workspace crate versions,
`Cargo.lock`, and the release manifest. Features bump the minor version; fixes
bump the patch version. Breaking changes bump the minor version before 1.0.
Merging that PR lets the next workflow run create the tag and GitHub Release.
It does not build or upload binaries or generated assets.

The workflow uses the repository `GITHUB_TOKEN` and requires **Allow GitHub
Actions to create and approve pull requests** in the repository's Actions settings.

Keep the crate names in the lockfile updater aligned with workspace membership.
The `go` release strategy provides changelog-only updates without a separate
version file; `extra-files` updates the Cargo manifests and lockfile.
Configuration lives in [release-please-config.json](../.github/release-please-config.json)
and the last released version in [release-please-manifest.json](../.github/release-please-manifest.json).
