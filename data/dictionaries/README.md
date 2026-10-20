# Bundled Chinese review seed

This directory contains **52 manually authored candidate phrases**, not a validated 40,000-term production dictionary. Categories: sexual (14), drugs (14), terrorism (12), political_abuse (12). These are category tags, not conclusions that a user or text is illegal.

On the **first start with an empty rules store** (no ACTIVE manifest), the service publishes `default_review_seed.json` through its normal persistent CAS publication mechanism. On subsequent starts the existing ACTIVE package wins. If ACTIVE is corrupt, the service does **not** silently restore this seed. Operators can replace the seed through the authenticated management publication API. The seed is never a substitute for IMBoy's canonical PostgreSQL publisher.

All seed rules use `medium` severity (review), `compat-v1` semantics, no aliases, no regex, no gap, and no exceptions. Review does **not** mean an automatic block. Even review hits can be false positives (e.g. news, reporting, education, or criticism). Do not promote these entries to automatic blocking without business validation.

The terms are **original hand-authored examples**. No third-party dictionary text has been imported, and no claim of source-verified 10,000 terms per category is made. Political views, people, parties, lawful political speech, and news topics are not inherently prohibited.

Run `cargo test --locked --test bundled_seed` and the existing integration tests before merging. Verify fresh boot, second boot retaining an operator-published ruleset, and corrupt ACTIVE failing closed. The GitHub PR is a code proposal, not proof of a passing local/Linux runtime validation.
