# Published reports

This branch is written by CI only; don't push to it by hand. It holds the
latest reports of `main`, which the Observatory reads:

- `cli.size.json`: byte and gzip size of the release `comline` binary.
- `cli.coverage.json`: line and function coverage per crate.
- `cli.compile.json`: a clean release build's seconds per crate.

All three come from `.github/workflows/reports.yml` on every push to `main`;
the formats are the Observatory's `DATA.md` (`size`, `coverage`, `compile`).
Each report names the `main` commit it measured. Every publish is one commit
here, so `git log -- cli.size.json` is the series.
