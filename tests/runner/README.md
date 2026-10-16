# T04 runner

Run `./scripts/verify_textguard.sh --group runner --evidence <new-external-directory>`.
`--list` only lists frozen arrays; `--dry-run` records NOT_RUN and exits 2.
`--task T04` runs runner-selftest, t04-build-contract and t04-migration. The build
selector needs the integrated tree (`tests/build_contract.py`, skeleton lane) and
reports BLOCKED when unavailable; the migration selector needs TEXTGUARD_FORK_ROOT.
The runner selftest covers AC-010/114 only; it never invokes full EUnit.

`tests/manifest.json` maps selectors and ACs. `tests/runner/commands.json` holds
literal argv, discovery argv, cwd, environment allowlist, timeout and count parser.
Additional Rust selectors can use `count_format: rust` with an explicit `--list`
discovery command. Each attempt requires a new evidence directory.

The distributable implementations are `scripts/command_receipt.py` and
`scripts/checkpoint.py`. The runner and ordinary selftests load only these
repository files; they have no personal directory dependency. Their digests
and resolved executables are fingerprinted in every result. No implicit download.

External control validation is opt-in:
`TEXTGUARD_CONTROL_TOOLS=<control-tools-directory> ./scripts/verify_textguard.sh --group control --evidence <new-external-directory>`.
It verifies that both persisted helpers match the repository implementation,
then runs checkpoint recovery tests only in a temporary sandbox. Missing or
different external helpers give BLOCKED/2. Ordinary runner results mark external
control validation NOT_APPLICABLE and never require that directory.

Checkpoint API: `coordinator_lock(control_root, session_id)` checks process identity;
`save(event, update, control_root)` serializes an atomic state replacement with
PREPARED/COMMITTED events and a retained original backup. `migrate_v2` returns a
copy preserving identities, legacy fields and retry budgets. It does not write.
`reconcile(state, current_fingerprint)` is an explicit conservative safety drill:
it applies one fingerprint to every PASS, so unrelated baseline phases may become
NOT_RUN/STALE. Do not invoke it automatically on coordinator startup or use its
output as actual recovery without a surface/phase impact mapping. Schema 3 stays 3.
Synthetic tests pass temporary control roots; no real state is modified.
Existing schema-3 relative references resolve against the supplied control root.
Saving an unrelated event preserves statuses and retry budgets, while every PASS
still requires existing evidence, including historical summaries. Missing proof
must be backfilled by the coordinator; save never exempts a historical PASS.
The migration helper also applies the conservative drill and requires explicit
recovery review before its returned copy is used for a live coordinator.
