# CI Workflows

This document summarizes the GitHub Actions CI workflows for the IBC Attestor project.

## Workflows

### release

Triggered on GitHub release publish events or manual dispatch.

Builds multi-architecture Docker images (linux/amd64 and linux/arm64) and publishes them to GitHub Container Registry (ghcr.io).

Jobs:
- `build-image`: Builds platform-specific images in parallel using matrix strategy
- `merge-and-publish`: Creates a multi-arch manifest and pushes with version and `latest` tags

Inputs (workflow_dispatch):
- `tag`: The git tag to build

### docker-publish

Manual workflow for building and publishing Docker images from any git ref.

Builds a multi-architecture image (linux/amd64 and linux/arm64) in a single job and pushes to GitHub Container Registry.

Inputs:
- `tag`: Git ref (branch or tag) to build

### lint-and-test

Automated workflow that runs when:
- A PR is opened
- Commits are pushed to an open PR

Runs a unit testing and linting with cargo tools. Linting follows strict clippy standards.
