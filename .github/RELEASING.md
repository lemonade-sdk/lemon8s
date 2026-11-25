# Releasing lemon8s

For the first release, do the one-time setup before merging the pull request that adds this file. Its merge drafts v0.1.0-alpha, so right after merging, publish it from step 3.

## One-time setup

In the repository's settings, as an admin:

1. **Actions → General:** tick **Require actions to be pinned to a full-length commit SHA**.
2. **Environments → New environment**, named `release`: add admins as **Required reviewers**, untick **Allow administrators to bypass configured protection rules**, and under **Deployment branches and tags**, allow only the tag rule `v*`.
3. **General → Releases:** enable release immutability.
4. **Rules → Rulesets → New tag ruleset**, targeting `v*` and **Active**: restrict creations, updates, and deletions, and let **Repository admin** bypass it **Always**.
5. **Rules → Rulesets**, for `main`: require the **Checks** status check.

Then, under **Issues → Labels**, add `breaking` and `fix`.

## Release

### 1. Prepare

In **Actions → Prepare release → Run workflow**, keep `main`, and enter the next version: `0.1.1-alpha` for fixes, or `0.2.0-alpha` for a breaking change. When the run passes, open the link in its summary and create the pull request.

Check: the pull request changes only the version.

### 2. Merge

The release notes are generated at the merge, from the pull requests merged since the last release, so first label any that break something `breaking`, and fixes `fix`. Merge when the checks pass.

Check: **Releases** has a draft, `v<version>`, marked **Pre-release**, with `lemon8s-<version>.tgz` attached and notes that start with the alpha warning.

### 3. Publish

Edit the draft. For a breaking change, add the upgrade steps and the expected downtime below the warning. Keep the warning first and **Set as a pre-release** ticked. Then **Publish release**. In the **Release** run that starts, choose **Review deployments**, then **Approve and deploy**.

Check: the run passes, and [Verify a release](#verify-a-release) succeeds. For the first release, then make the package public: in the organization's **Packages**, open **charts/lemon8s**, then **Package settings → Change visibility → Public**, which can't be undone. Check that `helm pull oci://ghcr.io/lemonade-sdk/charts/lemon8s --version 0.1.0-alpha` works on a computer that isn't logged in to ghcr.io.

## Verify a release

Users verify a chart before installing it, with the [GitHub CLI](https://cli.github.com) signed in by `gh auth login`:

```bash
version=0.1.0-alpha
helm pull oci://ghcr.io/lemonade-sdk/charts/lemon8s --version "$version"
gh attestation verify "lemon8s-$version.tgz" --repo lemonade-sdk/lemon8s \
  --signer-workflow lemonade-sdk/lemon8s/.github/workflows/release.yml --source-ref "refs/tags/v$version"
```

Then install it with the README's [Try it](../README.md#try-it) steps, with `"lemon8s-$version.tgz"` in place of `oci://ghcr.io/lemonade-sdk/charts/lemon8s --version <version>`, so Helm installs the file you verified.

Verifying is what protects users from an account with write access to this repository: such an account could push to ghcr.io from a workflow of its own, which the `release` environment doesn't stop, but only the Release workflow, run for a release's tag, makes these attestations.

## When something goes wrong

- **Wrong version, before merging:** close the pull request and delete its `release/<version>` branch.
- **Wrong version, after merging:** delete the draft, revert the release pull request, and prepare again.
- **Missing labels:** add them, delete the draft, and re-run the merge commit's **Release** run.
- **A failed Release run after publishing:** follow its error, then re-run the failed jobs. A published release can't change, so if the release itself is wrong, release the next patch.
