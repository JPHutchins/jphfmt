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

## First Release

```sh
# Bump version in Cargo.toml and editors/vscode/package.json
# Commit, then tag:
git tag v0.1.0
git push origin v0.1.0
```

CI will:
1. Build and test on push (the `v*` tag triggers release jobs)
2. Build binaries for linux/macos/windows
3. Upload binaries and `.vsix` to the GitHub Release
4. Publish to crates.io (`cargo publish`)
5. Publish to VS Code Marketplace (`vsce publish`)

## Ongoing

- **DeepSeek code review** runs on every PR after tests pass. It ingests test output as context and posts findings inline.
- **Mutation testing** runs nightly at 3:03 AM UTC. Surviving mutants open a labeled issue.
- **cargo-audit** runs on every push/PR for RUSTSEC advisories.
