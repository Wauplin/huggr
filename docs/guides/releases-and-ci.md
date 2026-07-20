# Release Huggr and build huglets in CI

This guide covers the repository release dispatcher, crates.io Trusted Publishing setup, recovery from a partial publish, and the reusable workflow for building a huglet in another repository.

## Release unit

The six native crates are one release unit and always use the same version:

1. `huggr-core`
2. `huggr-replay`
3. `huggr-host`
4. `huggr-providers`
5. `huggr-agent`
6. `huggr-toolkit`

Internal dependencies use exact versions. `scripts/release.py check` verifies the lockstep version, publication metadata, downstream workflow pins, and the packaged toolkit assets. Ordinary pull request CI runs this check and its unit tests.

The WASM crate, Python extension crate, and examples are not part of this crates.io release unit.

## One-time repository setup

Create a GitHub App for release automation, install it on this repository, and grant repository permissions for Contents (read and write) and Pull requests (read and write). Store its App ID as the repository variable `RELEASE_APP_ID` and its private key as the Actions secret `RELEASE_APP_PRIVATE_KEY`.

The dispatcher uses an installation token from this App to push the release branch and open its pull request. It intentionally does not use `GITHUB_TOKEN`: GitHub documents that most events created with `GITHUB_TOKEN` do not start another workflow run, while an App installation token can trigger the normal pull request CI. See [Triggering a workflow](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow) and [Authenticating with a GitHub App](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app).

Enable auto-merge for the repository. Keep the usual branch protection and required CI checks on `main`; the release pull request uses squash auto-merge and waits for those rules. Add a required human review if releases should stop for confirmation after the dispatch.

Create a GitHub Actions environment named `crates-io`. Environment protection rules are optional, but adding required reviewers gives publication a separate approval gate.

## Bootstrap crates.io once

crates.io requires the first version of each crate to be published manually before a Trusted Publisher can be added. Publish the six crates in dependency order from the exact release commit:

```bash
python3 scripts/release.py check
cargo publish --locked -p huggr-core
cargo publish --locked -p huggr-replay
cargo publish --locked -p huggr-host
cargo publish --locked -p huggr-providers
cargo publish --locked -p huggr-agent
cargo publish --locked -p huggr-toolkit
```

Wait for each crate version to appear in the crates.io index before publishing the next dependent crate. Cargo publication is permanent, so run this only from the intended release commit. See [Publishing on crates.io](https://doc.rust-lang.org/cargo/reference/publishing.html).

After the bootstrap, add a Trusted Publisher to each of the six crates with these values:

- GitHub owner: `Wauplin`
- Repository: `huggr`
- Workflow file: `publish-crates.yml`
- Environment: `crates-io`

The publisher workflow requests `id-token: write` and exchanges GitHub's OIDC identity for a short-lived crates.io token through `rust-lang/crates-io-auth-action`. No long-lived crates.io token is stored in GitHub. See [crates.io Trusted Publishing](https://crates.io/docs/trusted-publishing).

## Dispatch a release

Open **Actions**, choose **Release Huggr crates**, and select **Run workflow**. The two inputs support three useful modes:

| Bump | Publish | Result |
| --- | --- | --- |
| `patch`, `minor`, or `major` | `true` | Compute the exact next version, open and auto-merge a version PR, then publish all six crates and create the matching GitHub Release. |
| `patch`, `minor`, or `major` | `false` | Compute the next version and merge only the version PR. |
| `none` | `true` | Publish the version already on `main`; use this after a bump-only run or to resume a partial failure. |

`bump=none` with `publish=false` is rejected as a no-op. The release concurrency groups allow only one version operation and one publisher at a time.

The version PR updates the workspace version, exact internal dependency pins, lockfile, documented install commands, skill cheat sheet, and the versioned downstream workflow example. A `-publish` release branch causes `publish-crates.yml` to run after the App-authored PR merges. A bump-only branch does not publish.

The publisher validates the immutable release commit, then publishes missing crates in dependency order. If a crate version already exists, it reads the VCS commit from that crate archive and requires it to match the release commit. This makes `bump=none`, `publish=true` safe for resuming a release after some crates succeeded. The Git tag and GitHub Release are created only after all six versions exist.

## Build a huglet from another repository

Copy [`examples/workflows/release-huglet.yml`](../../examples/workflows/release-huglet.yml) to `.github/workflows/release-huglet.yml` in the huglet repository. Adjust `agent_dir`, `surfaces`, and `artifact_name`, then dispatch it with the huglet's own release version.

The build job calls the versioned reusable workflow and pins both the workflow tag and installed toolkit version:

```yaml
jobs:
  build:
    uses: Wauplin/huggr/.github/workflows/build-huglet.yml@v0.0.2
    with:
      agent_dir: .
      huggr_version: 0.0.2
      surfaces: cli
      release: true
      artifact_name: huglet
```

For stricter supply-chain pinning, replace the `v0.0.2` workflow ref with its immutable commit SHA while keeping `huggr_version` at `0.0.2`. GitHub recommends a commit SHA as the safest reusable-workflow reference. See [Reuse workflows](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows).

The reusable workflow accepts these inputs:

| Input | Meaning |
| --- | --- |
| `agent_dir` | Repository-relative folder containing `Cargo.toml` and `huggr.toml`; defaults to `.`. |
| `huggr_version` | Required exact published `MAJOR.MINOR.PATCH` toolkit version. |
| `surfaces` | `cli`, `python`, or `cli,python`; defaults to `cli`. |
| `release` | Build optimized artifacts; defaults to `true`. |
| `models_file` | Optional repository-relative `models.toml` embedded during the build. Do not commit provider credentials to this file. |
| `artifact_name` | Uploaded artifact name; defaults to `huglet`. |
| `retention_days` | Artifact retention in days; defaults to `14`. |

Tool grants and filesystem or network scopes stay in the huglet's checked-in `huggr.toml`. The reusable workflow has no scope override and receives no secrets. It uploads the standalone binary, optional Python wheel, `agent-card.json`, `effective-config.json`, and `SHA256SUMS`. The example's second job downloads those files and creates a GitHub Release in the caller repository.
