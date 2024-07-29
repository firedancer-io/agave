# Solfuzz patches
This branch `solfuzz-agave-v2.0-patches` tracks the specific agave-v2.0 revision that [solfuzz-agave](https://github.com/firedancer-io/solfuzz-agave)
is based on. Patches can be applied on top of this branch to help with fuzzing.

## Breakdown

### Original solfuzz-agave dependency model
`solfuzz-agave` originally directly depended on a specific v2.0 revision of agave by specifying the commit hash
in the [`Cargo.toml`](https://github.com/firedancer-io/solfuzz-agave/blob/762219d921cdd9e5f4d2851f1ea90cdebdf431d9/Cargo.toml) file.

```mermaid
%%{init: { 'gitGraph': {'mainBranchName':'anza-xyz/agave'} }}%%
gitGraph LR:
	commit id:"1-commit"
	commit id:"2-commit"
	commit id:"v2.0-chkpt" type:Highlight tag: "solfuzz-agave"
	commit id:"3-commit"
	commit id:"4-commit"
	
```

### Patch model

```mermaid
%%{init: { 'gitGraph': {'mainBranchName':'firedancer-io/agave', 'parallelCommits': true} }}%%
gitGraph LR:
	commit id:"1-commit"
	commit id:"2-commit"
	commit id:"v2.0-chkpt"
	branch solfuzz-agave-v2.0-patches
	commit id:"1-patch-commit"
	commit id:"2-patch-commit" type:Highlight tag: "solfuzz-agave"
	checkout firedancer-io/agave
	commit id:"3-commit"
	commit id:"4-commit"

```

Here, the `solfuzz-agave-v2.0-patches` branch tracks the specific agave-v2.0 revision that `solfuzz-agave` is based on in the [firedancer-io fork of agave](https://github.com/firedancer-io/agave).
We update the `Cargo.toml` file in the `solfuzz-agave` repository to point to the **HEAD** of `solfuzz-agave-v2.0-patches`.

⚠️ Note: We want the dependencies to point to the **commit hash** of the patches branch, **not** the branch itself. This keeps the dependencies stable.

## Applying patches
Checkout a new branch from the `solfuzz-agave-v2.0-patches` branch and apply patches on top of it.
Open a PR to merge the patch into the `solfuzz-agave-v2.0-patches` branch.


```mermaid
%%{init: { 'gitGraph': {'mainBranchName':'firedancer-io/agave', 'parallelCommits': true} }}%%
gitGraph LR:
	commit id:"1-commit"
	commit id:"2-commit"
	commit id:"v2.0-chkpt"
	branch solfuzz-agave-v2.0-patches
	commit id:"1-patch-commit"
	commit id:"2-patch-commit"
	branch solfuzz-agave-v2.0-syscall-patch
	commit id:"1-syscall-patch"
	commit id:"2-syscall-patch"
	checkout solfuzz-agave-v2.0-patches
	merge solfuzz-agave-v2.0-syscall-patch type:Highlight tag: "solfuzz-agave"
	checkout firedancer-io/agave
	commit id:"3-commit"
	commit id:"4-commit"

```

After merging the new patches, update the `Cargo.toml` file in the `solfuzz-agave` repository to point to the (new) HEAD of `solfuzz-agave-v2.0-patches`.



## Rebasing patch branch
Say we have a new commit `v2.0-new-chkpt` that we want to track.

```mermaid
%%{init: { 'gitGraph': {'mainBranchName':'firedancer-io/agave', 'parallelCommits': true} }}%%
gitGraph LR:
	commit id:"1-commit"
	commit id:"2-commit"
	commit id:"v2.0-chkpt"
	branch solfuzz-agave-v2.0-patches
	commit id:"1-patch-commit"
	commit id:"2-patch-commit" type:Highlight tag: "solfuzz-agave"
	checkout firedancer-io/agave
	commit id:"3-commit"
	commit id:"4-commit"
	commit id:"v2.0-new-chkpt"
	commit id:"5-commit"

```

We can rebase the `solfuzz-agave-v2.0-patches` branch on top of the new commit.

```mermaid
%%{init: { 'gitGraph': {'mainBranchName':'firedancer-io/agave', 'parallelCommits': true} }}%%
gitGraph LR:
	commit id:"1-commit"
	commit id:"2-commit"
	commit id:"v2.0-chkpt"
	commit id:"3-commit"
	commit id:"4-commit"
	commit id:"v2.0-new-chkpt"
	branch solfuzz-agave-v2.0-patches
	commit id:"1-patch-commit"
	commit id:"2-patch-commit" type:Highlight tag: "solfuzz-agave"
	checkout firedancer-io/agave
	commit id:"5-commit"

```

Open a PR for the rebase (pointing to the new base commit). After rebasing, update the `Cargo.toml` file in the `solfuzz-agave` repository to point to the (new) HEAD of `solfuzz-agave-v2.0-patches`.