# Publishing jphfmt

## Prerequisites

- [ ] GitHub repo created at `github.com/JPHutchins/jphfmt`
- [ ] Push this repo: `git remote add origin git@github.com:JPHutchins/jphfmt.git && git push -u origin main`
- [ ] crates.io account (login at [crates.io](https://crates.io), generate API token)
- [ ] VS Code Marketplace publisher (create at [marketplace.visualstudio.com](https://marketplace.visualstudio.com/manage))
- [ ] DeepSeek API key (register at [platform.deepseek.com](https://platform.deepseek.com), create key)

## GitHub Secrets

Set these in repo Settings → Secrets and variables → Actions:

| Secret | Value |
|--------|-------|
| `CARGO_REGISTRY_TOKEN` | `cargo login` token from crates.io |
| `VSCE_PAT` | VS Code Marketplace personal access token |
| `DEEPSEEK_API_KEY` | DeepSeek API key (`sk-...`) |

## Releasing

`Cargo.toml` holds the version; `Cargo.lock`, `editors/vscode/package.json` and its lock follow it.
One command does the whole release:

```sh
camas release                      # bump the patch, ship it
camas release --VERSION=minor      # or major, or an explicit X.Y.Z
```

It checks the metadata, rewrites all four files, commits `release X.Y.Z`, tags `vX.Y.Z`, and pushes
`main` and the tag — the tag push is what starts CI's publish jobs.

Nothing is written until it is safe to release: it refuses unless you are on `main`, the tree is clean,
`main` is level with `origin/main`, the tag does not exist, and the new version follows the current one.
`main` is pushed before the tag, because the default branch has to carry what was published or
`cargo install --git` reports a version that was never released.

`camas version_check` runs as part of `check`, `all` and CI, so a file left behind fails the build
rather than surfacing as a wrong `--version` after someone installs from `main`.

From the tag, CI will:
1. Build and test on push (the `v*` tag triggers release jobs)
2. Build binaries for linux/macos/windows
3. Upload binaries and `.vsix` to the GitHub Release
4. Publish to crates.io (`cargo publish`)
5. Publish to VS Code Marketplace (`vsce publish`)

## Ongoing

- **DeepSeek code review** runs on every PR after tests pass. It ingests test output as context and posts findings inline.
- **Mutation testing** runs nightly at 3:03 AM UTC. Surviving mutants open a labeled issue.
- **cargo-audit** runs on every push/PR for RUSTSEC advisories.
