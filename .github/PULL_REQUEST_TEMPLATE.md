## Summary

What changed and why (1–3 sentences).

## Type of change

- [ ] Feature
- [ ] Bug fix
- [ ] Refactor / chore
- [ ] Docs only

## Checklist

- [ ] Linked **Milestone** (epic) is set on the PR’s issues where applicable.
- [ ] Requirements and acceptance criteria for linked issues are met (or issue updated).
- [ ] Edge cases from the issue(s) are covered in tests or explicitly documented as follow-ups.
- [ ] `cargo fmt --all` / `cargo clippy --all-targets --all-features -- -D warnings` / `cargo test --all` pass locally.
- [ ] Proto or DB migration changes are called out in the summary.

## Merge gate (repo settings)

Ensure **branch protection** on `main` requires these checks before merge:

`fmt`, `clippy`, `test`, `release-build-smoke`

GitHub: **Settings → Branches → Edit rule** → *Require status checks to pass* → select all four job names from this repo’s CI workflow.
