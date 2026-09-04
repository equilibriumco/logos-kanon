# Delivering a milestone to Logos

Internal. This file is not part of any deliverable; see "Keeping this file out of the
delivery" at the end before cutting a milestone branch.

Two repositories, one history.

| | |
|---|---|
| `equilibriumco/logos-kanon-dev` | private. Remote `dev`. The trunk: every milestone, in flight or delivered |
| `equilibriumco/logos-kanon` | public. Remote `logos`. What Logos reviews, one milestone at a time |

They share commit SHAs. That is the whole design: a change Logos asks for comes back as
a real commit that `git merge` understands, rather than a patch to transplant by hand.
Everything below exists to keep that property true.

## The `delivery` branch

`delivery` is the authoritative record of what Logos has been shown. It lives in both
repositories and is pushed to `logos` as the head of a per-milestone pull request.

Two invariants hold, and the workflow is arranged so they keep holding:

- **`delivery` is an ancestor of `main`.** Anything Logos has is also in the trunk.
- **`delivery` contains only delivered milestones.** It is never merged from `main`,
  because `main` carries work on milestones that have not shipped.

`main` may be, and usually is, far ahead. That is expected: `main` is a descendant, not
a mirror.

## Delivering a milestone

Milestone work lives on its own branch, and that branch is merged into `delivery` and
into `main` separately. Both merges bring in the same commits, so the graph stays
coherent and later merges between the two are conflict-free.

```sh
git checkout delivery
git merge --no-ff origin-milestone-branch -m "M1: foundations and verification core"
git config remote.logos.push refs/heads/delivery:refs/heads/m1-delivery
git push logos
gh pr create --repo equilibriumco/logos-kanon --base main --head m1-delivery
git checkout main && git merge delivery
```

The refspec is updated per milestone on purpose. A bare `git push logos` can then only
ever write the branch named in it, so nothing you type locally can advance the public
`main` or publish an unreleased milestone. `main` on `logos` moves only when Logos
merges a pull request.

M0 was based on the empty initial commit, so its diff read as the whole tree. Later
milestones are based on the previous delivery and read as ordinary diffs.

## When Logos asks for a change

Either direction works, and neither rewrites anything:

```sh
# the change landed on the public main
git fetch logos
git checkout delivery && git merge --ff-only logos/main
git checkout main     && git merge delivery

# the change is ours to make
git checkout -b fix/whatever delivery      # off delivery, never off main
...
git checkout delivery && git merge --no-ff fix/whatever && git push logos
git checkout main     && git merge delivery
```

Branching off `delivery` rather than `main` is the one discipline to hold. A fix
branched off `main` drags unreleased milestones into the delivery lineage.

## What breaks it

**Squash and rebase merges.** Both replace commits with new SHAs, which destroys the
shared lineage and turns every subsequent sync into a manual transplant. It is not a
theoretical risk: a squashed milestone produces a tree identical to `main`, so it looks
fine until the next fix arrives and `add/add`-conflicts on every file the milestone
touched.

Squash and rebase merging are therefore disabled on `logos`, leaving merge commits only:

```sh
gh api repos/equilibriumco/logos-kanon --jq \
  '{allow_merge_commit, allow_squash_merge, allow_rebase_merge}'
```

Both must read `false`. Check after any settings change; a re-enabled button is a
silent trap.

**Rebasing `delivery`.** Same failure, self-inflicted. Do not rebase it, amend its
commits, or reset it.

**`git push logos --all` or `--mirror`.** These ignore the configured refspec and would
publish every milestone branch. The refspec guard makes the bare push safe; it cannot
protect against an explicit one.

## Local configuration that is not in the repository

Two settings live in `.git/config` and are lost with the working copy:

```sh
git remote add logos git@github.com:equilibriumco/logos-kanon.git
git config remote.logos.push refs/heads/delivery:refs/heads/<current-milestone>-delivery
git config remote.logos.tagOpt --no-tags
```

`tagOpt` keeps tags from the public repository out of this one, so a tag pushed there
does not become ambiguous here. Do not push a branch and a tag under the same name;
`m0-delivery` was briefly both, which makes the ref ambiguous on the remote.

## Keeping this file out of the delivery

`delivery` receives only what is explicitly merged into it, so this file stays on `main`
as long as no milestone branch carries it. A milestone branch cut from `main` will carry
it. Before merging one into `delivery`, check:

```sh
git diff --name-only delivery..<milestone-branch> | grep DELIVERY.md
```

If it appears, drop it from the merge. It documents how Logos is delivered to, which is
not something Logos needs shipped to them.
