# Execution decisions

[中文](decisions-ledger.md) | [English](decisions-ledger.en.md)

D001 Control-directory mapping: the user prompt's CONTROL_ROOT pointed at an existing plan
file, so the file could not be treated as a directory nor moved or overwritten. After matching
the original digest, the fixed directory from plan §1.3/§22.1
(`/Users/leeyi/imboy-evidence/textguard-v2`) was adopted; the original plan and its read-only
copy are preserved, and later prompts resume from that stable directory. This adjustment is an
authorized actual-directory mapping and changes no T/AC obligation.

D002 No `mem` executable was found; do not invent `/mem load`, do not modify local memory. The
current task does not depend on prior chat history.

D003 T01/AC-111 staged obligation: the current code confirms that channel edits are unreviewed,
moments have no edit route, and App-side permanent business rejections can enter the outbox.
T01 is a read-only inventory task and cannot be required to first complete the T20 code
adaptation that depends on T19; AC-111 is therefore staged as source_baseline/consumer_final —
T01 closes the source chain and the existing targeted baseline, while final compatibility closes
under T20/T23. G0 uses only the baseline sub-result; G8 must check consumer_final and must not
label a to-be-modified side as UNCHANGED_VERIFIED. This interpretation removes the timing
conflict between the task card's read-only scope and the final AC requirement while preserving
every acceptance obligation.

D004 Business scope is frozen against the current code: channel content creation (including
incoming webhooks that traverse the same publication function) and authorized edits, moment
creation, and updates to the five existing profile fields; plaintext is sent only for verified,
approved public scenes, and private channels/private moments must be branched before the remote
call. Moments have no existing edit API, so no edit interface is invented; C2C/C2G, ciphertext,
comments, registration creation and channel-entity fields are not automatically included merely
because they share underlying functions. The existing profile review precedes saving, and later
minimal adaptation must bind the actually written value and version; pre-existing ordinary
enqueue failures may still publish, but the exception boundary must be preserved and alarmed,
without claiming durable exactly-once review.

D005 Separate the contract PoC from the product phase: the PoC only verifies both-side
generation, presence, stable application errors and deadline, with the service explicitly
returning NOT_READY, and does not count toward engine or T19 dual-protocol business acceptance.
T03 froze the commands as the task-invocation interface and expected oracle; actual
scripts/test selectors are registered item by item from T04, unimplemented ones return BLOCKED,
and the contract list is never treated as executed tests.

D006 HTTP/gRPC error classification follows the original V2.1 §7.2. Old Rust comments and tests
treated OVERLOADED as 503, detection-budget exhaustion as 413, and PROTOCOL_ERROR as a parameter
error; those must not become backend allowlist expectations. Red tests reproduced 503/429,
400/503 and gRPC INVALID_ARGUMENT/DATA_LOSS respectively. Service and adapter are corrected
together: HTTP PROTOCOL_ERROR maps to 502, gRPC to DATA_LOSS, and other internal errors to
500/INTERNAL. Management CAS 412, busy 409 and the existing safety rejections are preserved.
This corrects the original contract rather than lowering requirements;
BACKEND-P1-HTTP-STATUS-CODE keeps the accumulated second-round budget.

D007 The T17 allowed-path table did not list main/lib/API assembly, yet the actual main still
exits NOT_READY and both HTTP and gRPC bypass the existing Admission; patching only runtime files
cannot deliver cross-protocol shared admission. Under single-writer rights for shared files, the
integrator minimally extends assembly into main/lib/API and the PublishedCore cancellation
callback, preserving the original protocol/Schema and default runtime budgets. Integration
happens only after independent review of the T17 component; real TLS, fault and privacy final
scenarios remain T23, the release performance freeze remains T18, and the full soak remains T24 —
no AC is recorded as passing early.

D008 T17 entry deadline restoration: the old gRPC component test fed 150ms into the core
unchanged, violating the global 80ms budget; after real assembly the tests assert a 20ms client
deadline and that 150ms is truncated by the 80ms global budget, proving the actual task does not
release the permit early. Independent pre-review further found the deadline missing before
decode; a real delayed-body counterexample went red first, then the entry absolute Instant was
threaded through decode/admission instead of being restarted in the handler. HealthWatch's
same-state repetition locks in tonic-health 0.12.3 visible behaviour (changed after the initial
borrow); repeats are allowed but must observe Serving after publication, at most 8 per second —
the business expectation that ready must change is not relaxed. T17-runtime-fix2 is formally
86/86, component phase only, and cannot substitute for T19/T23/T24.

## T10 regex configuration budget (2026-10-16)

D009 T18 real-Linux measurement found that the custom hyper acceptor bypassed tonic's default
TCP_NODELAY: gRPC single-concurrency 20-hit responses settled at 41–51ms, with the last
high-density request causing about 42ms of draining; the same rules in pure core take about
100us. After preserving the raw T18-perf02 red evidence, the integrator serially enabled NODELAY
in runtime/server.rs for already-accepted TCP streams, restoring tonic's default. The T18 card
scope extends minimally to this one wiring point; protocol, matching, admission, deadline and
the original performance thresholds are unchanged. Real Linux A/B, T17 component combination
regression and independent review are required before integration. The original baseline RSS
failure stands and must not be explained away as fixed.
Plan §4.7 bounds both compilation and DFA cache and aggregates configuration at 32 MiB; B1
requires all 32 rules. The fixed product builder uses 768 KiB compilation plus 256 KiB DFA per
rule, totaling 1 MiB per rule and 32 MiB for 32 rules. No runtime/RSS hard-limit claim. Tests
reject inflated compilation, cache, aggregate and expression expansion. AC-041 continuity
remains a T14/T22 runtime obligation.

### D010 — T18 measurement frozen and T24 capacity acceptance split by phase

Plan §13/§15.3 and the T18 card explicitly separate T18 measurement and prior freezing from the
T24 final assertions. Independent review T18-measurement-review02 confirmed that the old runner
mixed short-run capacity into early gates. All formal01–03 failures are retained, and
per-metric strict original-threshold observations are added; T18's own execution/oracle/
protocol/resource evidence gates are not downgraded. The runner sets phase=T18_measurement only
for T18, and overall AC091/092 with G5 are not marked PASS.
benches/support-targets.json freezes the original 50k plus the enhanced scale / 500 completed
QPS / P99 / RSS; T24 uses a 600s warm-up, a fixed 3600s window and draining, all reported, with
no deletion of high-RSS samples and no rounding. The final branch where a single-connection
NODELAY set failure only drops the connection needs a new-image measurement. Independent
reviewer_type=agent cannot substitute for final capacity or business sign-off.

### D011 — T19 independent golden data aligned with frozen exceptions/CAS contract

attempt09 had 7 failures out of 118 real items; the original failure receipts remain unchanged.
An independent agent used the same archived Rust binary and a synthetic package to run isolated
HTTP diagnostics; receipt T19-attempt09-isolated-diagnostic-UX2sGU/observations.json shows:
alias returns word_source=pattern; a B-only authorized exception keeps the first T evidence;
an old ETag with the same package returns already_active with generation unchanged. §4.8
requires same-view exceptions, while §9.1 states that same-package idempotency takes precedence
over different-package CAS. Test construction is corrected: the demo exception explicitly
authorizes B/T while the original partial-hit expectation is kept; a new low B-only negative
case proves T is not expanded by the exemption; the CAS same-leaf case proves both same-package
idempotency and rejection of a legitimate different-package old ETag. Since the plan does not
specify canonical/pattern for alias.word, it is frozen to the configuration pattern already
returned by Rust; the independent golden pins synthetic constants without calling production to
generate expectations, rule_id still binds to canonical, and severity/action/offset assertions
are unchanged. AC060/077 must close through the new candidate's formal 119-item dual-protocol
verification and an independent delta review; diagnostics do not count as acceptance.

## D012 - T20 current-entrypoint binding and public-only transport

Channel public eligibility is visibility=0/access_type=0, active and outside workspace; moment public is visibility=0 (default1=friends). Preserve local legacy checks. Only explicitly configured rust mode sends public text without ciphertext/e2ee/encrypted markers. Unknown channel visibility fails closed. Current approved channel/moment APIs expose content only; profile updates one whitelisted field per operation. No moment edit API is invented.

Channel edit checks after author/window permissions and before workspace_guard transaction; conditional update binds original content+updated_at. Queue only after successful writes; duplicate idempotent creates do not enqueue again. Profile failed saves cannot enqueue. Review removal conditionally matches checked channel/profile content to avoid deleting changed content; workspace guard retained. Metadata JSONB stores structured hits/trace and a content digest. JSON word arrays preserve canonical commas; legacy rows keep fallback display. Current migration maximum194 verified; coordinator allocates195 with up/down.

Applies T20/AC078-083; T21 adds bounded shadow/modes at the same boundary. Final proof requires real isolated PG and Rust; mocks only support targeted fault/notification observations.

## D013 - T20 independent review tightens payload and content-version binding

The first-draft review T20-readonly-review-preflight identified two P1 and two P2 issues: PostgreSQL JSONB payload bytes skipped remote edit checks; caller encryption markers could downgrade public text; content-only review deletion allowed ABA; truncated hit lists lost their completeness flag. This supplements D012 without widening approved scenes. Text payloads accept decoded maps or valid JSON object bytes; malformed or unsupported encryption-marked text is rejected locally with zero remote calls. C2C/C2G are still outside this adapter.

Migration195 additionally stores channel moderation_version and per-profile-field moderation_versions. Initial channel rows use the empty version; every content update rotates a random token, including legacy updates. Profile updates use jsonb_set so changing one field preserves the other field versions. Rust edits compare the content and version observed before detection; newly enqueued channel/profile reviews carry that saved version, and reject uses a conditional version+content match. Old review rows retain their legacy content-only compatibility. Metadata preserves response completeness and snapshot trace without recalculating decision from displayed hits. Verification must exercise real JSONB reads, forged markers, ABA, enqueue exceptions and truncated hits before review closure.

Source caller audit also found generic user_repo updates (for example ai_agent profile nickname). A narrowly scoped PostgreSQL trigger rotates any actually changed approved profile field whose caller did not supply a fresh version; this keeps old review tokens invalid across those existing paths without adding new remote-check scenes. Explicit checked writes keep their supplied token. Trigger creation/removal is part of migration195 up/down and must be tested on the isolated database.

New trigger-generated versions use UUID bytes rendered as 32 lowercase hexadecimal characters (UUID text with hyphens removed), matching checked-write/queue token shape. Reading and comparing an existing opaque stored token remains unrestricted, including a preexisting 36-character UUID; only newly authored schema1 review bindings require the defined 32-byte shape (or an initial empty channel token). This is a producer-format consistency change; the real PG trigger, unchanged-field, checked-write and ABA tests retain their original expectations.

## D014 - T21 shadow uses the actual legacy cache snapshot and bounded admission

Plan §10.1 names conceptual moderation.backend; the current T15/T20 configuration is moderation_detector.mode. Keep that existing key and the approved public plaintext entrypoints. inspect_snapshot uses one cached_words result for all independent fields and computes TGBASEv1 from those exact original word/category/severity tuples. Export reuses the same pure digest encoder. Missing metadata produces legacy_unavailable, never a synthetic comparable allow. Mode shadow retains the old verdict and performs no synchronous Rust call.

The supervised worker uses 64 fixed ETS slots, two linked/monitored workers, and a 1000ms deadline measured at admission. Request bodies never enter the coordinator mailbox. Mode switches change an epoch, discard queued objects, terminate old workers, and release capacity only on DOWN. Legacy/rust remain explicit modes; defaults remain legacy. Fixed classifications distinguish base/normalization/semantics mismatch, comparable decisions, enhanced evidence, unexplained enhanced differences, and facility/drop counters. Enhanced observations are not business approval or compatibility PASS. T21 real PG/Rust HTTP+gRPC verification and independent review remain required; T22 onward is outside the user's current stop boundary.

## D015 T21 gRPC stream cancellation and OTP privacy fix

Contract §10.1/§10.4 and T21-review01 require actual underlying stream cleanup before shadow
resources are released. The pinned grpcbox unary timeout does not cancel the stream; in the first
round 53 of 55 leaves passed, 2 were fixture cumulative-count errors, and cleanup logs separately
revealed a task-random authorization. The existing production bridge is extended: the guardian
obtains shadow-coordinator confirmation before creating a stream; the concurrency slot is
released only when worker and guardian are both DOWN; after RST_STREAM CANCEL an independent
monitor confirms the stream DOWN. The unary channel pick and send_request are reused; this
deployment has no interceptor, and unknown interceptors are explicitly not_ready rather than
silently ignored. A shadow job is settled once; counters do not mean resources were released.
The OTP primary filter executes synchronously in the sender: exact service markers identify
stream-termination reports, sender-local markers continue to cover the proc_lib/supervisor
report family; whole safety events are replaced, metadata is whitelisted, unknown
callbacks/bodies/tokens are not retained; unrelated senders are untouched. Underlying decode
errors/malformed success bodies still fail. The affected T19 shared bridge must pass real
dual-protocol regression; the Rust core is unchanged and can reuse the 452-component evidence.
Everything is unaccepted until new tests and the formal freeze02/re-review pass.

D015 addendum (after the independent delta finding, before formal retest): a PID DOWN after stop
must not be treated as the stream set being released; RST prefers graceful close, and if a kill
is needed only this stream's active registration is closed and verified — if cleanup cannot be
proven, the guardian slot is held until the connection ends. Do not use the library's internal
default 5-second stream-creation timeout that drops the handle; switch to headers-only async
stream creation, keep the body with the guardian and discard it on expiry or owner death, and
pending connection messages carry no body; keep the guardian waiting for a late handle and cancel
it immediately. Shared supervisors are not persistently marked. Log adjudication must count
before redaction, while on-disk redaction is retained; 4 new formal runner self-tests verify that
the real finish gate fails on injected output.

## D016 T21 review03 restart ownership and caller deadline

Independent review03 reproduced restart oversubscription, foreground deadline overrun and connection error remapping. The two fixed shadow lease processes now outlive coordinator/guardian death; a registered connection must acknowledge ownership before headers-only stream creation. A lease is released after guardian DOWN with cleanup proof, or actual connection DOWN. Foreground callers use a fixed absolute context deadline and a reply alias; they return timeout without abandoning guardian cleanup or accepting late replies. No foreground two-slot limit is introduced. connection_failed and overloaded retain the existing closed error vocabulary. New real-connection regressions cover restart, 60ms deadline with live caller mailbox, and physical connection failure through bridge/adapter/business. T19 shared bridge acceptance must be rerun after integration. This decision does not advance T23 resource/memory acceptance.

## D017 W5 native correctness environment and generated cache retention

Docker's own host log records an ENOSPC shutdown; shared Docker resources are preserved. W5 correctness uses the installed PostgreSQL18.4 in an isolated loopback cluster with private temporary credentials and the real Rust service. This provides no Linux filesystem, performance, soak or production evidence. Review04 required independent PGDATA/credential directories, preservation when stop/status is uncertain, and PG raw-log scanning before redaction; review05 closes the changes with three real-PG regression tests. Final integration must exercise the updated helper and bind its digest. The failed selftest03 recovery receipt collision remains recorded; selftest04 uses separate recovery receipts without changing fault assertions.

Per the user's disk request, completed task-owned Cargo target/registry copies may be removed after checking process identity, open files, generated-only content and Git ownership. Keep raw logs, receipts, source archives, lockfiles, all recovery branches and final candidate worktrees. Retain the latest useful final build cache; remove superseded generated copies after terminal receipts exist. Cleanup receipt06 records about 19.8GiB additional physical recovery; snapshots and original test claims are not rewritten. No shared Docker image/volume, global Cargo cache or user resource is removed.

## 2026-10-18 20:45 session3 (zcode-20261018T2035-resume) resume verification conclusions

1. **Lock takeover**: the previous coordinator PID 49711 (zcode-20261018-w6-resume) was confirmed dead via ps → took over; the soak09 performance lock PID 73310 was dead and T24 was already terminal → archived to state.backups.
2. **soak09 verification closure confirmed** (persisted by the previous session at 12:54, no rework needed): 100/100 publications; 1h soak-measure 1.8M requests p99 3.4ms; AC-095=MECHANISM_VERIFIED+DEFECT_FIXED_PARTIALLY (RSS defect fix A/B 932MB→222MB; residual +3.3MB/pub documented; the 512MiB bound was not met at 594MB; the zero-unexpected hour was interrupted by host bursts). The soak-rss-growth.json driver produced nothing; adjudication rests on capacity-observations.json + rss-fix-ab, which is acceptable.
3. **imboy candidate drift clarified (important)**: on 2026-10-18 17:50 the user, acting on their own behalf, merged the textguard task into imboy main (83899ae2), then merged PR #20 at 18:05 (f3ca4fbf, GitHub noreply). The imboy-integration branch has advanced with main to f3ca4fbf. AC-105 attempt3 (started 20:23) runs on that latest candidate = the correct final candidate base. The f87aeae2 recorded in the 13:03 manifest is stale → update at T27 freeze after eunit finishes. This is a user-initiated action, not an unauthorized task merge.
4. **Candidate-integrity false alarm clarified**: src/textguard.proto is already tracked by git (the initial check misjudged because the ls-files output was truncated by head -15). src/textguard_pb.erl is a build artifact regenerated by gpb from the proto at eunit startup (Makefile: gpb-generated code does not participate in gates); being untracked is expected. A clean candidate checkout regenerates it via make compile — no defect.
5. **G5 rerun decision**: imboy_local (PID 67918, a user process, alive since Friday) still bursts at 88.7% CPU and AC-105 eunit is consuming CPU → this session starts no capacity rerun (performance tasks must not run in parallel with full EUnit). Re-evaluate host quietness after eunit completes; if conditions are unmet, keep PARTIAL_ENV_BLOCKED and report honestly.

## 2026-10-18 20:52 G5 rerun plan (executed after eunit completes)

Preconditions (all must hold before starting): (a) AC-105 eunit attempt3 finished and verified; (b) restart the contention monitor (T24-contention-monitor/, old PID 60896 dead) and observe imboy_local <20% CPU for ≥30 continuous minutes; (c) clean up this task's leftover Docker containers (t28-*/abA/abB/abC/rebuild), keeping eunit-pg until eunit finishes; (d) re-register the performance lock.

Rerun commands (cwd=textguard-integration worktree, image e328a097, final candidate):
1. `python3 -B tests/performance/run_capacity.py Capacity.test_ac092_b1_capacity` → target a clean B1-grpc window (the previous 3 all landed in contention: formal05 58 unexp/122% CPU, formal07 11 unexp)
2. `python3 -B tests/performance/run_capacity.py Capacity.test_ac094_boundary_workload` → boundary-4096 (previous 4/8/2 unexp all landed in burst windows; 256 already PASS)
3. `python3 -B tests/performance/run_capacity.py Capacity.test_ac093_mixed_fairness_and_overload_ladder` → the 750/1000/1500 ladder (fairness already proven; only the ladder is missing)
4. Optional: `Capacity.test_ac095_one_hour_soak_with_100_publications` (a zero-unexpected hour; the 512MiB RSS bound is a documented allocator limit that a rerun does not change)

Re-check host quietness between groups; if any window lands in contention again, stop the rerun and keep PARTIAL_ENV_BLOCKED (the evidence chain already suffices: formal05/06/07 plus monitor timeline correlation). imboy_local is a user process and must not be interfered with.

## 2026-10-19 09:20 AC-105 attempt3 mid-run diagnosis (pg_jieba environment gap)

**Facts**: at 03:10 CST one failure appeared: adm_organization_handler_tests:setup_db → inttest_marker_db:ensure_extensions → CREATE EXTENSION pg_jieba → 0A000 feature_not_supported. The isolated eunit PG (textguard-eunit-pg-c9ccc9) is a vanilla postgres:18 image with no pg_jieba/timescaledb/postgis extension files (0 hits when listing the extension directory); the product contract (AGENTS/user environment imboy_pg18 = imboy/pg18:3.6.1-2) requires those extensions. The attempt1 log, 382744 lines, contains the **exact same** failure (pg_jieba appears twice) → this failure is unrelated to the 10-18 evening candidate drift (attempt1 ran on f87aeae2 before the drift) and is an environment gap shared by both attempts; the previous session's "attempt1 zero failures" conclusion only checked the eunit terminal summary marker (attempt1 never reached the end).

**Classification**: ENV gap, not a product defect; same class as existing repository doctrine (the AG31/SC153 environment-gated suites recorded in Makefile:95-140 are all carried by dedicated single runs).

**Handling**: do not interrupt attempt3 (the 13h+ already run is valid); wait for it to finish and take the authoritative final failure list from eunit; start a replacement container from the local imboy/pg18:3.6.1-2 image on the **same port 49919** (same credentials/database name, zero config change); for each failing module run `make eunit t=<module> EUNIT_CONFIG=config/sys.tg-eunit EUNIT_USE_RELAY=0` targeted to obtain evidence. cs_application_tests (another provision_fresh consumer that has not run) is expected to hit the same gap and is handled together.

## 2026-10-19 09:40 AC-105 attempt3 adm_admin_handler_tests ×2 failures classified (concurrent interference, not a regression)

**Failures**: 13:01:47 / 13:06:05 UTC, config_product_experience_success_payload (expected configured_value=workspace, got chat) and failsafe_visible (expected configured_raw=workspce-typo, got chat).

**Root-cause chain** (closed by static forensics):
1. The user's PR #19 (bf5523f9, a user-owned commit inside the drift window) changed these two tests' configuration injection from the meck `application:get_env` to the `with_product_experience` helper = `application:set_env(imboy, product_experience, Value)` (VM global state).
2. The handler's `configured_experience()` reads `application:get_env(imboy, product_experience, chat)`; sys.tg-eunit.config does not set that key (grep=0).
3. Inside the failure window a concurrent migration suite made the imboy application exit and restart every 3 seconds (migration_dirty churn, present throughout 24,828 times); each app restart resets app env back to sys.config (unset → handler default chat).
4. The two assertion moments (13:01:47/13:06:05) coincide precisely with churn windows, with identical signatures (both read back chat).

**Classification**: the class documented at Makefile:95-140 — "exclusive/global-state suites necessarily interfere on a full 4-way concurrent track"; attempt1 has no record of this module at all (it never reached it, so not a pass); unrelated to the TextGuard commit (TextGuard never touches the app env/product_experience path); unrelated to candidate drift (a new fragility introduced by the user's own PR in the same window — the old meck design was resistant to interference).

**Handling**: do not interrupt the suite; when it finishes, run targeted single runs together with the pg_jieba environment gap: `make eunit t=adm_admin_handler_tests EUNIT_CONFIG=config/sys.tg-eunit EUNIT_USE_RELAY=0` (corrected PG 49919 = imboy/pg18:3.6.1-2); a green single run proves the interference classification. Further same-class suite failures are expected in the remaining run; the final list is triaged into "env-gated / app-env-fragile" and rerun in a targeted way.

## 2026-10-19 10:40 attempt3 termination root cause + attempt4 design (chunking + launchd)

**attempt3 termination root cause (hard evidence)**: 2026-10-19 09:58:31 CST, beam/make received SIGTERM (log tail `Terminated: 15` + `SIGTERM received`); at the same moment the contention monitor spawned via nohup by this session died and the harness background task registry was cleared (TaskOutput reported No task found). Attempt1 terminated at 20:20 CST (the previous session's wrap-up snapshot was written 20:26–20:30, immediately after) → **culprit = process cleanup by the ZCode harness session/backend reclamation** (happens roughly every 7–14h under heavy use). The "launchctl detachment" claimed in resume.md did not take effect — the process was still inside a group-kill range.

**attempt4 design** (engineering countermeasure to that root cause):
1. **launchd domain hosting**: `launchctl submit` proved separable in practice (test task PPID=1); the supervisor is a direct child of launchd, so harness reclamation no longer reaches it.
2. **Serial chunking**: 1436 modules (966 src + 470 effective suites − Excl 9) ÷ 60 per chunk = 24 chunks; each chunk gets an independent VM, exit status and log receipt; the supervisor is idempotent (chunks with an existing .exit are skipped) → any unexpected termination loses at most the current chunk, and resubmission resumes.
3. **Corrected PG**: textguard-eunit-pg-fixed (imboy/pg18:3.6.1-2) is already serving on 49919 (see below); the pg_jieba gap is fixed inside the chunks instead of relying on after-the-fact reruns.
4. EUNIT_MODS is passed through MAKEFLAGS to the inner make (GNU make semantics), aligning the module set with the real full track (erlang.mk EUNIT_MODS logic replicated + project Excl filtering).

**Corrected PG deployment record (10:25–10:40)**: the vanilla postgres:18 container was stopped (kept for evidence, not deleted); textguard-eunit-pg-fixed = imboy/pg18:3.6.1-2 @ 127.0.0.1:49919, credentials as before (tg_eunit). Two image init defects were bypassed: 10_init.sh lacked the execute bit, so sourcing it expanded ${@} into a file list (fixed with chmod +x); 20_update.sh created timescaledb during the temporary-server phase before preload took effect (skipped; postgis-family extensions are created per database by runtime ensure_extensions, and the final server has preload active). The tg_eunit database was created manually; pg_available_extensions lists all four extensions; shared_preload_libraries=pgcrypto,pg_jieba,timescaledb,vector,pg_stat_statements.

## 2026-10-19 11:20 adm_admin_handler_tests single run: interference hypothesis refuted, classified as a deterministic user-domain failure

Single run on corrected PG with no concurrency: **Failed: 2 / Passed: 9**, failures identical in signature to the full run (line 244 configured_value expected workspace, got chat; line 282 configured_raw expected workspce-typo, got chat). Key corroboration: the line 242 effective_product_experience=workspace assertion passed (value from meck), while line 244 comes from the real application:get_env = unset → the setup's set_env value is invisible at assertion time. ensure_config_loaded only idempotently loads and does not clear env; the exact wipe mechanism was not pinned down (nested setup order/meck interaction possible), but **the classification does not depend on the mechanism**: deterministic reproduction in a single run ⇒ not full-run interference; it is a deterministic failure of the user's PR#19-rewritten test code on this candidate/environment (the user's cloud CI 12777 all passed = environment difference). Assigned to class D (user domain): reported honestly, user test code not modified on their behalf, TextGuard delivery not blocked. adm_organization_handler_tests is reclassified the same way: the pg_jieba gap is fixed by the corrected PG (setup passes), but 11 elib_tsid_not_initialized occurrences reproduce in a single run → class D environment sensitivity. The attempt4 in-track list will provide consistency evidence for both modules.

## 2026-10-19 12:00 chunk environment failure-chain root cause + fix (tg_eunit missing extension set)

**Failure chain (hard evidence)**: in chunk-01/targeted single runs, ac04_*/account_ds/adm_organization showed elib_tsid_not_initialized (attempt1/3 vanilla-PG full tracks had zero such failures) → chain: the corrected PG's brand-new tg_eunit database had no extensions → the foundation migration (00000001) calls create_hypertable (timescaledb) → on vanilla PG the extension is unavailable and the path conditionally skips (success); on the corrected PG the extension is available but tg_eunit lacked it → SQL 42883 create_hypertable does not exist → schema_migrations version=1 dirty=true (written 02:34:40) → imboy_migrate:migrate/1:101 aborts on every start → the imboy app never starts → TSID (imboy_sup:327, initialized at app start) is never available → suites depending on app state fail in a cascade. eunit_setup silently continues when app startup fails ({app_not_started, test_continues}), amplifying the damage.

**Fix (12:00)**: stop the supervisor and process group (launchctl remove group kill, without touching the user's imboy_local); DROP/CREATE tg_eunit; install the same 12 extensions as inttest_marker_db ?EXTENSIONS into the database (pgcrypto/pg_jieba/timescaledb/vector/postgis/pg_trgm/btree_gin/btree_gist/citext/unaccent/intarray/pg_stat_statements); restart the supervisor (PID 70339, PPID=1). **Verification: the chunk-01 rerun's first start reports Application imboy started, migration_dirty=0**. The earlier chunk-01 first run produced contaminated evidence (its logs were overwritten by the rerun, no cleanup needed); the two targeted single runs' elib_tsid failures are explained by the same chain — adm_organization's class-D classification is corrected to "environment-chain failure, fixed, pending chunk rerun verdict"; adm_admin's 2 cases await chunk-01 recheck (its setup does not depend on provision_fresh; if still failing, the user-domain classification stands).

## 2026-10-19 12:06 attempt4 first-pass progress and environment stability confirmation

chunk-01 (rerun after the fix) 91s EXIT:0 with no eunit summary, and chunk-02 55s EXIT:2 (Failed:6/Passed:444), both containing db_down (03:57–03:58 UTC) = a brief instability window after the extension installation (timescaledb worker initialization/connection storm); from chunk-03 onward the env is stable: EXIT:0 / 377s / 0 failures / db_down only 2 transient occurrences. **Handling**: chunk-01 (VM died with no summary = unverifiable) and chunk-02 (db_down contamination) are marked invalid; after the first pass reaches chunk-24, delete both .exit files and resubmit for automatic rerun (supervisor idempotent design). Valid chunk evidence keeps accumulating. Estimated total ~2.5h (at chunk-03's average 6min per chunk; heavy chunks lengthen it).

## 2026-10-19 12:12 attempt4 pass1 new failure set and discrimination plan

chunks 04/06 showed a new failure set (attachment_upload_logic×8, agent_hird_mini_spike×6, attachment_upload_spool×3, cs_first_reply_pg×1, attachment_lifecycle_v5_w403×1, etc.); chunks 03/05 were clean. attempt3 baseline: all of these suites passed cleanly. One failure signature: FK violation fk_ear_a*(23503) plus a singleton "Failed to load config /config/sys.config: enoent" warning (only that suite in chunk-04; zero occurrences in attempt3). Three candidate difference variables: (a) corrected PG content (extensions/brand-new database state) (b) chunked VM cold-start state (c) launchd supervisor environment (PATH etc.). **Discrimination experiment** (run after pass1 ends, to avoid port conflicts): for the failing suites run `make eunit t=<mod>` in an interactive shell (same corrected PG) — green = supervisor environment difference (fix supervisor env and resubmit the chunks), red = PG/state difference (bisect further). chunk-01/02 stay invalid pending rerun.

## 2026-10-19 12:55 AC-105 attempt4 final list and classification (first complete full evidence)

**Historic fact**: attempt4 completed two full passes (pass1 all 24 chunks in 42 minutes; pass2 reran chunk-01/02), giving this machine its first complete full EUnit evidence. attempt1/3 never reached the attachment/enterprise/cs region (died at 543 groups), so "attempt3 had these suites clean" was a **no-baseline illusion** (non-verbose passes print nothing; death leaves no summary) — earlier classifications based on it are corrected.

**TextGuard product suites: 9/9 all green** (textguard_base_digest/business/contract/export/grpc_call/integration/pb/publisher/shadow, chunk-23 Failed:0 Passed:841).

**adm_admin_handler_tests re-decided**: on the fixed-environment full track it PASSES (pass1/pass2 consistent); its ×2 failures appear only in solo single runs and in the attempt3 vanilla-PG context → context-dependent (with_product_experience's app env depends on concurrent app runtime state); green in-track is green for acceptance purposes; the context sensitivity is recorded honestly. adm_organization / ac04_* / account_ds are all green after the fix.

**Final failure classification** (all non-TextGuard):
1. Toolchain gate (designed FAIL, missing external hird command): agent_hird_mini_spike ×6 + imboy_hird_acceptance ×3
2. macOS /tmp symlink class (fixtures do not resolve symlinks; Linux CI has no such problem; consistent with the user PR's own "native acceptance pending" statement): attachment_upload_logic ×8 + attachment_upload_spool ×3 + attachment_lifecycle_v5_w403 ×1; isolated_pg_fixture ×3 is an inner cancel wrapper (attachment_pending_location_pg inner cancellation, same class)
3. Explicit env gate (designed FAIL): enterprise_directory_sync_pg ×1 (missing DIRECTORY_SYNC_INTTEST_PG_HOST)
4. Behavioural singletons (mechanism recorded, awaiting user triage): eb_seat_send_protocol ×1 (500≠200), cs_first_reply_pg ×1 (42501 trg_enterprise_message_purge_guard), enterprise_internal_audit_policy_pg ×1, enterprise_internal_wiring_http ×1
5. Cancellations: a few each in chunk-08/16/20/23 (Failed:0; a known full-track phenomenon, with 62 cancellation precedents in main branch history)

## 2026-10-19 13:10 final verdict persisted + G5 window open

**AC-105/G8 final verdict** (see state for detail): attempt4's two complete passes = this machine's first full evidence. TextGuard 9/9 green; adm_* all green on the fixed environment; residual failures are all non-TextGuard with complete classification (native-runner gate 22 + behavioural singletons 4 + a few cancellations). native-eunit-run.sh is designed for GitHub runner only (hard exit 2 locally), so the gate environment is not locally reproducible by user design; the user's own statement "cloud 12777 passed / native pending" corroborates. **Overall = PARTIAL_COMPLETE_NOT_LOCAL_CANDIDATE_PASS** (plan 1319's strict criterion: unrelated failures not cleared means LOCAL_CANDIDATE_PASS must not be claimed; all TextGuard-scope ACs are green).

**G5 rerun started** (13:07): the user restarted imboy_local (old runaway PID 67918 dead, new PID 68476 quiet), the best window in three days; ac092-b1-grpc → ac094-boundary → ac093-ladder rerun in order (launchd textguard-g5, final image sha256:e328a097..., per-group evidence dir, contention monitor redirected to 68476). Three driver fixes: full sha256 image reference / independent evidence directory (exist_ok=False) / docker PATH (launchd lacks the Docker Desktop path).

## 2026-10-19 15:45 session wrap-up (end of session3)

G5 final state: ac092 PASS / ac094 PASS / ac093 PARTIAL_NARROWED (ladder 1000 point 6 unexpected = 0.001%, 1500 not reached; fairness proven twice). Cleanup complete: all this task's containers deleted, both eunit-pg containers deleted, /tmp scripts deleted, launchd labels cleared; all evidence directories retained; user resources untouched. Terminal overview in state.json (overall=PARTIAL_COMPLETE_NOT_LOCAL_CANDIDATE_PASS; TextGuard green across its full scope).

## 2026-10-19 imboy-textguard main directory switched to main
- In response to the user's challenge "why is imboy-textguard/ not committed": the product code's 73 commits were always in the repository (main=integration=49dfe3f), but the main directory sat on the bootstrap placeholder branch (only .gitignore+README.md, clean worktree), because git cannot check out the same branch in two worktrees at once — main had been placed in the evidence-area worktree.
- Handling: unregistered the textguard-main worktree (clean worktree; branch and commits unaffected; runs/ evidence untouched), then ran git checkout main in the main directory; HEAD=49dfe3f and the full product code is now visible.
- Same day: the full EUnit rerun of a2fe60cd completed 24/24 chunks (all T27-eunit-attempt4 receipts dated today), residuals matching the classified list, TextGuard 9/9 green; ac093 was finalized as PARTIAL_NARROWED_REPRODUCED after its second reproduction.

## 2026-10-19 aho-corasick dependency upgraded to 1.1.5
- The user manually pinned =1.1.5 in Cargo.toml in the imboy-textguard main directory (the committed state was =1.1.3) and pointed out that upstream BurntSushi/aho-corasick has a 1.1.5 tag.
- Forensics: crates.io already published 1.1.5 (2026-08-03, not yanked, same day as the upstream tag); the local AC mirror repo tag 5178060=1.1.5, with master 2 lint commits ahead; the mirror does not participate in dependency resolution (the lock goes through the registry).
- Handling: `cargo update -p aho-corasick --precise 1.1.5` refreshed the lock (that package only, checksum c982642f...); `cargo check --locked --all-targets` passed; committed Cargo.toml+Cargo.lock to main as leeyi (new HEAD in git log).

## 2026-10-19 task worktree disk cleanup
- Authorized by the user. Deleted all 33 worktrees under textguard-v2/worktrees (28 clean ones deleted directly; 5 containing only untracked files were checked first: the observability and textguard_pb untracked copies = already committed or already retained; 3 shadow test drafts differing from the committed version were backed up with sys.tg-eunit.config to runs/.../pre-cleanup-untracked/ before force deletion).
- 6.4G→12K; all branches and commits preserved in the 4 main repositories; prune also removed a stale registration for /private/tmp/imboy-unified-main-merge-20261013 inside the imboy repo (the directory had long been gone; registration cleanup only).
- Untouched: other task lines' worktrees under .codex-runs/.codex; runs/ 21G of evidence; Docker images (information reported to the user).

## 2026-10-19 evidence-area build cache cleanup
- Deleted 8 pure cargo build caches under runs/.../ (5×rust-target with CACHEDIR.TAG + 2×cargo-home registry + contract-poc/target), about 4.5G total, all regenerable from committed sources + lock, with no evidentiary value; textguard-v2 dropped from 21G to its current size.
- Candidate second layer awaiting user decision: intermediate attempts superseded by later rounds (T21-shadow-attempt03/04/05/08/09 at 304M each, T24-soak08 395M, early formal/ac096 rounds) — delete only after cross-checking against state.json adjudication references.
- Must keep: T24-g5-rerun 2.1G (AC-093 reproduction evidence, needed for root-cause work), the final formal/soak09-fixed, T27 full EUnit, T28, release/state/resume/decisions.

## 2026-10-19 second-layer cleanup (user authorized)
- Deleted about 400M of intermediate directories not referenced by state + 21 Docker task volumes / 8 old image tags / builder containers / 1.1G build cache. Final asset list verified OK (state/resume/decisions/release/g5-rerun/eunit-attempt4/T28-run07/formal05/soak09-fixed/pre-cleanup-untracked).

## 2026-10-19 remaining reclaimable Docker cleanup (user authorized, deletion list archived)
### Deleted dangling volumes (66)
0e4221d7074f6b5686952794f7426ba87d5960a78036209bc9bd372404367c8f
02f979a5092cdf01f3a93ad576fa3112f68136fa195014e39535c1aa032fe9b3
3b59f9def9fc9ddd75ac31bb0ecb2ac6ed7fa11edb63c72996edc066cdd74b2a
4a597035d30bc717201facce43533a44f45b4ceda0804b179fd2e250cf598458
4c38a7e0c68d12ea439d0d3e0eb842250f9c5456679d39c9730a0c4fb89ee140
4c463ee92f7d2151f1bd6fc78ed27fbee2b3afaeba815e711a37c0f249ceb11b
4e05fbf38323512c1abb44592455468b3366bdd52ae97edbbf46d29167059d82
5df16e52ed8cc0a88e65d7cc41b26cab29615e7681c110c82dd47a8326d78efe
5ee6e4742a76ce5e2d4a86567fff76b56fe4ba585dc2ca09dda96ce4e14e1742
5f0530a3edf423b89e9e05a0345981ee5a4d76d0de37c3cf5e806840226aee3b
6b04f610f8e7272a44c18bc98b159bfb65424035722d65bd064a83e264b81f8a
7fdca538605c9dc013a13a5fe85ddae2a3d7c3b43fbc53708af31289e7d402ac
8a0f868616310f88aea5c1b9fe0cf6e951ee6ae730f3ab897c6da0242e239fff
8e8b853a780b4419216b50eda9e104c9c80b18b982c27f6eeb4869b1171c8a16
11c777e46ee19fda0e56dd699b0209feb3c5f11a3630122bdb062894c9a1bf0f
23b8de3b5002ee1c1aded97de46c620ad2c8295760600349b3a2c951742d5f70
27d37c9c8d854400cc823ec85003e7752de35fc8fd663bf3d85ce84cd0ccc2b2
30a5e11d5e0c42e21f21f932545701a6908f83ebc9727a6a550d624e2899f9f9
31e1266fe235003beb707e283b769fa129d3fff5a31c6e5b7f31dc32e04d250d
39b32aa3cd70f7d7a854292b4a15efc6659a1c04908a04d9d255a66cf01fa433
81e3583398d0d1ed8e4ef4aa4731d252570de66c658282d0884a8fa3bcd66eda
97f9679f5a328d64a18cc6427559d6c9c79d33c96f24afc6f96dc7a64143d8b1
230caf9eb9ed8c81d19ea291a88f3cb6e14306255288f77b0dcb382483520e8c
441b43bce961b76697e3fd737d0027e0361e989d27ecbcf8921fbf5a30d37134
469db9fa3b10b25ba526a56d0185fbe12d7aea1361a566110d1eacd9674fe9e1
511f0c2aa55c9488af0cad2ae657d8f2891e868bfc92e522e9174e6d1c7732e5
667eab6a911dd0b8c1af0a678852e19af79d72c4ac12c8e007a6c0676c563d3e
857b86dbeb3483392c03a65532ebae5ca8887e30939ea1c854deefa7ce766847
1953d31a2f25b742f818a100e5c2d1c7fa27267b2df98ff404dd3b08ca180607
4583f6d9c9889cbe9877efc050b9e548951a11474f5005ce60b4bb2faf13e2ca
10467a1b37ac99eb0c0d4e5ff7d3fbb982b9bf43a2b8aa7247631fc74e20e52a
52118e05be05f8f06021c55a06fef50f25ef41a82b6ef0f5cafaabf8988c1deb
61313dae035bcd5b641dda4bff9f8690dc561c36dc1a12d09c4223b7325c07cb
73518b0179fc8b54c54c6dca4b73f2b7ef8f2a93f008590bd877c2a33263ea08
727546cf61ece22f2b195928c1148c2c23711ec8e8c3ace0e38629a17ee1a71c
7824429fe5682e19bc672d511f1d93da6d83370457b44e3a94e632dbb54313e5
4343712382b86e553b2630608fbe8470968c7d91db48c5bd1b5877de2ae230c9
b9c5725f35c028516618e92ef21b729eb74ba926453ddf2ffa2797217057a412
b49ffe2253aeafc3a1498c8bdcf1f3baec01eafacdd6ea59cb2782af26e2a2d1
b925f66a3704cd9712e75e40bd5b4ba715652336daed34abe850820375e6503a
bb11f55945ce6ec16e78de47eee5a0b4941eecbd1e996d2f2badc1caa8c9965c
bdaf811e4f87b4911e102b9857506d68641219971911ab91ae1ece07d79305e0
buildx_buildkit_mybuilder0_state
d5e0519764da199ee78016c119469fe7afc71ec7e401cf459ecf428a8fb5599b
dacdbe8636e385d25b7b7587eaa65b41ad23a9eed4d35e783e4ca7207a3befc1
dcfe22cef1e5808f97eda9e156c352516b4f574f96bd22a886809d8628c3ab30
de965229363ceba9ddafb34e5e0ce600099860b84d994334fdf6d33d2bd9a864
deepflux-e2e-garage
deepflux-poc_garage-data
deepflux-poc_redis-data
deploy_fuyao-redis-data
dev_garage
dev_nats
dev_pg
dev_qdrant
e1b4d6f41c9eb34f34f8518680957e3efd7ff0266d7ddff068676c41eadafa81
e71e6ad5f839e8ef35b96bf7e2c51b402586e0474bd07d512ce5c59dec20118c
e9676f0acc95246ca079f168a86b768511f9663b730c4e128033bf69c3fb3eac
edd6dd088fe06007191165dd2296d4b7a542dc9392a100152de4d7561c3da451
f5d568ec57cfa650403134f9c164fd47ed9b1bebc31baa1704beab36d2792d80
f85e893cfc631b3b98dbcb610d478589d402d8984d3f76808ba956188cc23a95
infra_redis_data
infra_rmqdata
tg-run-20261016t075111z-pg-data
tg-run-20261016t075111z-pg-r1-data
tg-run-20261016t075111z-toolchain

### Deleted unused standard images
imboy-backend:tsid-v10-hcfix imboy-backend:tsid-v10-3a01829d mysql:8 postgres:18 docker:dind oven/bun:1-slim livekit/livekit-server:v1.13.7 livekit/livekit-cli:v2.1.1 dxflrs/garage:v2.3.0 dxflrs/garage:v2.0.0 nginx:alpine nginx:1.27-alpine redis:7-alpine nats:2.10-alpine alpine:latest debian:trixie-slim prom/alertmanager:latest
### Protected, untouched
imboy/pg18:3.6.1-2 (in use by two running containers), textguard-perf:t24fix-01, textguard-builder-base:1, pgvector/pgvector:pg18 (closing item), csww-widget:v0/v1, cs-hosted-widget-img:20260920t150315 (custom build, cannot be pulled again, 150M total)
### Execution result
- Volumes 68→2 (only mounts of running containers remain), ~9.2G reclaimed; images 28→11 (6.29G→3.71G), ~2.6G reclaimed (125 untagged/deleted entries); host has 53G free.
- The remaining 2.718G of "reclaimable" space is all deliberately protected: textguard-perf:t24fix-01 + builder-base + pgvector (closing item) + csww custom builds (cannot be pulled again).

## 2026-10-19 21:10 G5 quiet-window rerun final verdict (session4 zcode-20261019-1930-resume)

- **Environment change**: the contention source imboy_local (PID 68476) died (ps empty at 19:27), leaving only this session and the GUI on the host; the new monitor timeline3.jsonl witnessed the whole period. The previous coordinator codex-20261019T170219 (PID 72561) had zero CPU for 2.5h, held no flock and wrote no state, so the lock was taken over per precedent.
- **rerun3** (19:40–20:53, 4370s, launchd com.imboy.textguard.g5r3, main-repo runner + image sha256:e328a097 t24fix-01): ac092 `ok` (warm/cap http 0-fail p99 4.1–4.6ms; **cap-B1-grpc 0-fail p99 3.9ms** — the point that failed three times under contention now reproduces cleanly, consistent with the T18 baseline 3.7ms; mixed p99 3.9–4.4ms; closed-B1 13.8k QPS). ac093 `ok`: **750/1000 tiers unexpected=0 rejected=0 failed=0; 1500 tier unexpected=0** (the single 1 failed is an explicit HTTP_429 admission rejection, an allowed bounded rejection). The narrow finding reproduced twice (6/9 ~130ms deadline timeouts at the 1000 tier, exactly coinciding with 83–115% imboy_local spikes) disappeared entirely in the quiet window, closing the contention attribution.
- **ac094-clean** (20:56–21:02, 345s): all five tiers 256/4096/16384/multi32k/dense unexpected=0 green (a clean re-proof of the same EXIT:0 seen in the 14:20 contention window).
- **Final verdict**: G5=PASS_WITH_DECLARED_SOAK_RESIDUAL; AC-092/093/094=PASS; AC-095 stays PARTIAL_DEFECT_FIXED (rss_growth_bound +3.3MB/pub is the allocator high-water residual; A/B fix evidence 932→222MB is documented and the 16.4 explanation path is satisfied; a rerun would not change the classification, so the soak is not rerun; its unexpected=49 is attributed by the timeline to the first-5-minute contention burst, with 0 over minutes 5–60). AC-105 closes as PASS_WITH_DECLARED_RESIDUALS (completing the AC-level persistence of the T27 task-level verdict). T24=COMPLETED_DECLARED_RESIDUALS.
- **Incidental fix**: directory-type evidence references for AC-091/096 and empty references in G1/G3/G4/G6/G7 in the on-disk state (caused by the 17:00 session writing directly while bypassing validation) were replaced with concrete files and sha256 filled in; checkpoint.save full validation passes again.
- overall stays PARTIAL_COMPLETE_NOT_LOCAL_CANDIDATE_PASS (plan §15.5: the G8 full EUnit contains pre-existing failures unrelated to this task — 22 native-runner gate items designed for GitHub runner only plus 4 pre-existing behavioural singletons, outside the authorized scope).

## 2026-10-19 21:57 goal-completion audit + U-line conditional items closed (session4)

- The audit found bookkeeping gaps on the U line: UC-004—015 and UG0—UG2 were still NOT_RUN even though U01 had concluded NO_UPSTREAM_CHANGE_NEEDED. Per plan §15.3 ("inactive U-line items are recorded as NOT_APPLICABLE"): UC-004/005 and UC-007—015 → NOT_APPLICABLE (conditions not activated); UC-006 → PASS (its criterion is precisely "no candidate exists", an outcome that already occurred; evidence = T26 independent review + traceability); UG0 → PASS (the qualification gate actually ran and legitimately terminated the candidate line: U00 PASS + U01 no general value); UG1/UG2 → NOT_APPLICABLE; UG3/UC-016—018 stay NOT_AUTHORIZED.
- Terminal scan: tasks 36/36, acceptance 120/120, gates 10/10, upstream_acceptance 18/18, upstream_gates 4/4 all terminal, with no NOT_RUN/IN_FLIGHT/PENDING/BLOCKED/FAIL.
- The plan's completion definition under current authorization is met: every executable task has a real terminal state and valid evidence; T29/U05/UG3/UC-016—018 are recorded NOT_AUTHORIZED per prompt §1.2/§13.8 (public submission and production are separately authorized items, not unfinished tasks). overall stays PARTIAL_COMPLETE_NOT_LOCAL_CANDIDATE_PASS; every gap to LOCAL_CANDIDATE_PASS consists of pre-existing out-of-scope failures (native-runner gate design + 4 user-domain behavioural singletons), with unlock conditions written into resume.md.

## 2026-10-20 05:40 independent review findings and fixes (session4, read-only audit + bookkeeping fixes)

- **Fix 1 (introduced this session)**: the timeline3.jsonl digest referenced by AC-092/093/094/G5 was stale — the monitor was still appending when the G5 closing state was persisted, invalidating the hash immediately. Refreshed against the current file bytes.
- **Fix 2 (historical)**: 79 PASS items' evidence references had never recorded sha256 (early sessions recorded paths only; the checkpoint validator only checked existence and therefore did not catch it). All backfilled.
- **Fix 3 (introduced this session)**: the .strip() in build_manifest's git() helper caused cargo_lock_sha256 to be computed over newline-stripped bytes. Changed to exact blob bytes; after the fix 5e07482a... matches the value bound in the 2026-10-18 original manifest (cross-confirming the candidate lock is unchanged).
- **Re-verification result**: all 391 PASS evidence references exist and all digests match; checkpoint full validation passes; manifest and git branch references (head/tree) agree item by item.
- No change at the adjudication level: all states, gates and residual declarations remain as they were.

## 2026-10-20 05:5x second review round: manifest reference hash ordering defect fixed

- AC-104/106/119/120 use release-manifest.json as hash evidence; the previous round's order ("backfill digests first, then fix the lock computation and regenerate the manifest") made those 4 stale. Refreshed.
- Rule (now in the resume checklist): after any manifest regeneration, every reference hash that uses it as evidence must be refreshed; aggregate documents used as hash evidence must not be rewritten in place afterwards — only new references may be added.

## 2026-10-20 06:30 fourth review round: runtime startability smoke (zero defects)

- Candidate 49dfe3f in a cold worktree built in 21.2s per the README, contract tests 7/7, and all six real-startup behaviour probes passed (401 / 400 / 503-NOT_READY-never-allow / 401 / 503+200 / gRPC reachable); the publication-store symlink hardening incidentally gained runtime proof (a /tmp symlinked ancestor was refused).
- No state changes; receipt in runs/run-20261016T075111Z/R4-startability-smoke.md. Review resources (worktree/secrets) cleaned up.

## 2026-10-20 07:0x fifth review round: functional end-to-end smoke (zero defects)

- The real binary's publish→detect loop passed completely: CAS publication / ready flip / snapshot identity rejection / seven detection probes all contract-correct (including decision-action-severity separation, original-text byte offsets, view-scoped exceptions, dual-semantics scenes). The initial "exception not suppressed" judgement was a probe misunderstanding — exceptions exempt per view (with B-only, the T witness survives) is §4.8 contract behaviour; declaring B+T yields allow, as proven.
- Three-repository drift audit: no moderation-contract path changes in app/admin since the verification heads.
- Receipt runs/run-20261016T075111Z/R5-functional-e2e-smoke.md; resources cleaned up, no state changes.

## 2026-10-20 07:3x sixth review round: gRPC wire + §9.4 recovery smoke (zero defects)

- grpc_contract 15/15 (real TCP); rules survive restarts after publication (revision/instance identical, detection works); ACTIVE corruption fails closed (503 NOT_READY, no crash and no false allow). Receipt R6-recovery-grpc-smoke.md; resources cleaned up.

## 2026-10-20 08:0x seventh review round: TextGuard drift audit on the imboy product side (zero defects)

- The only TextGuard change from f3ca4fbf→a2fe60cd is the generated file textguard_pb.erl entering the repository (proto unchanged). The committed version and the Makefile canonical version differ in layout but have identical exported function sets, identical maps representation and byte-identical CheckTextRequest encoding → zero functional drift; the EUnit evidence substantively covers the candidate head (AC-118 drift audit closed on the product side). Hygiene note: the user is advised to regenerate the pb via make so the bytes are canonically tracked (user domain, non-blocking).
- verify_textguard.sh --list with no environment returns BLOCKED/exit 2 = §15.3 fail-safe design ✓.
- Receipt R7-imboy-drift-audit.md; no state changes.

## 2026-10-20 08:3x eighth review round: code hygiene audit (zero defects)

- Rust src/ has no dbg!/TODO/DIAG; println/eprintln only for version output and startup-failure/panic observability; the historical DIAG residue was cleared by 11dea3e. The Erlang moderation path has zero residue. CI and deploy/README claims (ports/credentials/80ms budget/loopback metrics) match the code item by item. Receipt R8; no state changes.

## 2026-10-20 09:0x ninth review round: concurrency + CAS smoke, operations manual consolidation (zero defects)

- 60 concurrent detections all returned 200 with correct decisions; the CAS three-way behaviour is clarified: stale + identical content = idempotent 200 (harmless), stale + different content = 412, invalid package = 400 dangling-reference rejection. OPERATIONS-RUNBOOK.md was consolidated to complete the §11.3 command list. Receipt R9; no state changes.

## 2026-10-20 09:3x user-authorized merge confirmation, branch cleanup and test-data cleanup

- **Merge confirmation**: no actual merge was needed on any of the three sides — the textguard repo's 25 task branches and the imboy repo's 10 task branches were all verified as merged via `git branch --no-merged main` (the user had already merged them into main in earlier sessions); no task-related uncommitted changes (the 5 dirty files in imboy and 1 in imboyadmin are user WIP and were not committed on their behalf).
- **Migration files (database growth files) verified**: imboy main has a single migration root priv/migrations with 197 migrations and no duplicate .up.sql numbers; textguard's 00000197_review_metadata and the user's 00000198_conversation_last_msg_id_xid follow each other without conflict.
- **Traceability**: tags were created before deleting branches — textguard repo `textguard-candidate-run-20261016T075111Z`→49dfe3f, imboy repo `imboy-textguard-candidate-run-20261016T075111Z`→a2fe60cd; the release-manifest branch field is a historical name from now on, and the SHA resolves through both main lineage and the tags.
- **Branch deletion**: 35 task branches deleted with safe `-d`, 0 refusals (all confirmed merged); none of this task's worktrees remain (other task lines' .codex-runs worktrees untouched).
- **Test-data cleanup**: docker removed textguard-perf:t24fix-01 (122MB) and textguard-builder-base:1 (427MB); /tmp cleared of tg-review-secrets, boot logs, detach test output, tg16-enh-corrupt.json (corruption-injection artifact); zero this-task test databases remain in the user's PG (the eunit isolated container was already deleted); untouched: the imboy_pg18 container and databases, other task lines' inttest_*/imboy_closeout_* test databases, the pgvector:pg18 image (ownership unclear, left to the user).
- Not pushed (not authorized).

## 2026-10-20 production deployment execution (T29/G9, explicitly authorized by the user)

- Authorization: the user explicitly "authorized deployment to prod, binary mode". Server 106.53.76.53:32 (pro-imboy, Debian12 x86_64).
- Execution: Mac Docker cross-build (rust:1.90-bookworm+protoc, TEXTGUARD_BUILD_ID=0.1.0-prod_49dfe3f, binary sha256 b622639762b3…) → scp with verification, installed to /usr/local/imboy-textguard-0.1.0-prod_49dfe3f/bin → systemd imboy-textguard.service (dedicated user imboy-tg, NoNewPrivileges/ProtectSystem hardening, bound only to loopback 8081/8082/8083) → published an empty ruleset generation 1 (honouring the x-textguard-confirm-empty guard header; deployment troubleshooting point: the secret owner was not changed, causing SECRET_FILE_UNREADABLE, recovered after chown — fail-closed working as designed).
- Verification: live 200 / ready 200 (gen1) / detection allow+complete / unauthenticated 401 / systemd enabled for boot start.
- Boundaries: the Erlang backend stays mode=legacy without switching traffic (zero business impact); peak observation and business traffic switching await later authorization/process. Operations manual in docs/operations/deploy-binary-prod.md (4b27a43).
- Same-day user cleanup: the out-of-repo evidence area (17G of raw data) was deleted by user decision; this directory is the sole proof carrier.
