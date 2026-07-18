# EngManager portfolio static integration

Issue 37 crosses a repository boundary, while the release request is one Frame
pull request. This repository therefore carries a deterministic patch for the
reviewed public portfolio snapshot instead of mutating or publishing the
external repository.

The patch targets only:

```text
matthewharwood/engmanager.xyz@1de52bc8f25793dea3697e67765d53785c05cdfa
```

Validate the artifact alone:

```sh
python3 -I scripts/ci/check-engmanager-portfolio-integration.py
```

Validate it against a clean or exactly patched checkout:

```sh
python3 -I scripts/ci/check-engmanager-portfolio-integration.py \
  --checkout /path/to/engmanager.xyz
```

On a clean pinned checkout, apply and run the portfolio's own gates:

```sh
git apply /path/to/frame/fixtures/engmanager-portfolio/v1/engmanager-static-frame.patch
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

The initial stage is intentionally static. It adds one validated
`FRAME_ORIGIN`, a visible homepage project CTA, a shared article/search nav
link, exact cross-origin browser boundaries, and a Frame-host collision guard.
It adds no Frame client, polling, handler-path network I/O, status data,
authentication, cookies, embed, CORS, or recorder permission boundary.

The portfolio's live healthy/stale/not-configured state machine is therefore
not applicable until a separately reviewed live-data stage proves enough user
value to justify availability coupling. The static link remains usable whether
Frame is healthy, slow, unavailable, or not yet deployed.

External acceptance, portfolio preview/browser results, provider cache traces,
production deployment, and the paired release record remain protected actions.
They must be attached in the portfolio repository and the launch runbook; the
Frame PR does not claim them.
