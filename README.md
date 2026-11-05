# herdr Nexus regressions

External regressions for the pinned herdr Nexus source. `nix flake check -L`
copies that source into a build sandbox, applies the explicit test-only hook in
`patches/external-tests.patch`, and runs the stored tests.

The hook pauses the final configuration between successful listener staging
and the ready-state commit. On the pre-`9047730` ordering, the public listener
was already visible during that pause and the readiness test observed no
snapshot. On the pinned fixed ordering, the listener is published after the
commit and every accepted connection has a ready snapshot. Patch application
is part of the check, so source drift fails rather than silently skipping the
instrumentation.

The suite also keeps regressions for the frame-capacity lower boundary,
duplicate live peer identity, duplicate created pane ownership, stale List
versus uncertain Prompt, and accepted peer grants revoked by removal or
rebind.
