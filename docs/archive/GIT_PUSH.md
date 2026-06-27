# Git push notes

## `main` branch vs `main` tag

This repository has a lightweight tag named `main` that predates branch-based workflows. A plain `git push origin main` may fail or push the tag instead of the branch.

Push the branch explicitly:

```bash
git push -u origin HEAD:refs/heads/main
```

To remove the conflicting tag (only if you intend to use `main` as the default branch name):

```bash
git tag -d main
git push origin :refs/tags/main
```
