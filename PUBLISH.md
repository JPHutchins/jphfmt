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
camas owns the three steps:

```sh
camas version_sync -- 0.1.6   # rewrite the version across all four files
git commit -am "release 0.1.6"
camas release                 # re-check, then push main and the tag v0.1.6
```

`camas version_check` runs as part of `check`, `all` and CI, so a file left behind fails the build
rather than surfacing as a wrong `--version` after someone installs from `main`. `camas release`
refuses a dirty tree, an existing tag, or a branch other than `main`, and pushes `main` before the
tag — the default branch must carry what was published, or `cargo install --git` reports a version
that was never released.

Pushing the tag is what publishes. CI will:
1. Build and test on push (the `v*` tag triggers release jobs)
2. Build binaries for linux/macos/windows
3. Upload binaries and `.vsix` to the GitHub Release
4. Publish to crates.io (`cargo publish`)
5. Publish to VS Code Marketplace (`vsce publish`)

## Ongoing

- **DeepSeek code review** runs on every PR after tests pass. It ingests test output as context and posts findings inline.
- **Mutation testing** runs nightly at 3:03 AM UTC. Surviving mutants open a labeled issue.
- **cargo-audit** runs on every push/PR for RUSTSEC advisories.
